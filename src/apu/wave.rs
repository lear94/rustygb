//! Wave channel: CH3 plays 32 four-bit samples from wave RAM.

use super::length::Length;
use super::{Control, MAX_FREQUENCY, T_CYCLES_PER_TICK};

const LENGTH_MAX: u16 = 256;
pub(super) const WAVE_RAM_LEN: usize = 16;
const POSITION_MASK: u8 = 31;
const DAC_ENABLE: u8 = 0x80;
const T_CYCLES_PER_PERIOD_UNIT: u16 = 2;
/// Extra delay before the first sample after a trigger.
const TRIGGER_DELAY_T_CYCLES: u16 = 6;
/// Midpoint of the 4-bit range, so the output is centred.
const SAMPLE_MIDPOINT: f32 = 7.5;

#[derive(Debug)]
pub(super) struct Wave {
    pub(super) enabled: bool,
    pub(super) length: Length,
    ram: [u8; WAVE_RAM_LEN],
    dac_enabled: bool,
    volume_code: u8,
    frequency: u16,
    timer: u16,
    position: u8,
    current: u8,
}

impl Wave {
    pub(super) fn new() -> Self {
        Self {
            enabled: false,
            length: Length::new(LENGTH_MAX),
            ram: [0; WAVE_RAM_LEN],
            dac_enabled: false,
            volume_code: 0,
            frequency: 0,
            timer: period(0),
            position: 0,
            current: 0,
        }
    }

    /// Advances one M-cycle; the shortest period fits twice in it.
    pub(super) fn tick(&mut self) {
        if !self.enabled {
            return;
        }
        let mut elapsed = T_CYCLES_PER_TICK;
        while elapsed >= self.timer {
            elapsed -= self.timer;
            self.timer = period(self.frequency);
            self.position = (self.position + 1) & POSITION_MASK;
            self.current = self.nibble();
        }
        self.timer -= elapsed;
    }

    pub(super) fn write_dac(&mut self, val: u8) {
        self.dac_enabled = val & DAC_ENABLE != 0;
        self.enabled &= self.dac_enabled;
    }

    pub(super) fn write_length(&mut self, val: u8) {
        self.length.load(val);
    }

    pub(super) fn write_volume(&mut self, val: u8) {
        self.volume_code = (val >> 5) & 3;
    }

    pub(super) fn write_frequency_low(&mut self, val: u8) {
        let [high, _] = self.frequency.to_be_bytes();
        self.frequency = u16::from_be_bytes([high, val]);
    }

    pub(super) fn write_control(&mut self, control: Control) {
        let [_, low] = self.frequency.to_be_bytes();
        self.frequency = u16::from_be_bytes([control.frequency_high, low]);
        if self.length.control(
            control.length_enabled,
            control.trigger,
            control.between_clocks,
        ) {
            self.enabled = false;
        }
        if control.trigger {
            self.timer = period(self.frequency) + TRIGGER_DELAY_T_CYCLES;
            self.position = 0;
            self.enabled = self.dac_enabled;
        }
    }

    pub(super) fn clock_length(&mut self) {
        if self.length.clock() {
            self.enabled = false;
        }
    }

    /// Reads wave RAM; while playing, every address hits the byte being played.
    pub(super) fn read_ram(&self, idx: usize) -> u8 {
        self.ram.get(self.ram_idx(idx)).copied().unwrap_or(0xFF)
    }

    /// Writes wave RAM; while playing, every address hits the byte being played.
    pub(super) fn write_ram(&mut self, idx: usize, val: u8) {
        let idx = self.ram_idx(idx);
        if let Some(cell) = self.ram.get_mut(idx) {
            *cell = val;
        }
    }

    /// Current output in `[-1.0, 1.0]`.
    pub(super) fn sample(&self) -> f32 {
        if !self.enabled {
            return 0.0;
        }
        let scale = match self.volume_code {
            0 => 0.0,
            1 => 1.0,
            2 => 0.5,
            _ => 0.25,
        };
        (f32::from(self.current) / SAMPLE_MIDPOINT - 1.0) * scale
    }

    fn ram_idx(&self, idx: usize) -> usize {
        if self.enabled {
            usize::from(self.position >> 1)
        } else {
            idx
        }
    }

    fn nibble(&self) -> u8 {
        let byte = self
            .ram
            .get(usize::from(self.position >> 1))
            .copied()
            .unwrap_or(0);
        if self.position & 1 == 0 {
            byte >> 4
        } else {
            byte & 0x0F
        }
    }
}

fn period(frequency: u16) -> u16 {
    (MAX_FREQUENCY + 1).saturating_sub(frequency) * T_CYCLES_PER_PERIOD_UNIT
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRIGGER: Control = Control {
        frequency_high: 7,
        length_enabled: false,
        trigger: true,
        between_clocks: false,
    };

    fn playing() -> Wave {
        let mut wave = Wave::new();
        for (idx, val) in (0..16u8).enumerate() {
            wave.write_ram(idx, val << 4 | (15 - val));
        }
        wave.write_dac(0x80);
        wave.write_volume(0x20);
        wave.write_frequency_low(0xFE);
        wave.write_control(TRIGGER);
        wave
    }

    #[test]
    fn tick_plays_nibbles_in_order() {
        let mut wave = playing();
        wave.tick();
        wave.tick();
        let played: Vec<u8> = (0..4)
            .map(|_| {
                wave.tick();
                wave.current
            })
            .collect();
        assert_eq!(played, [15, 1, 14, 2]);
    }

    #[test]
    fn tick_shortest_period_steps_twice() {
        let mut wave = playing();
        wave.write_frequency_low(0xFF);
        (0..8).for_each(|_| wave.tick());
        let before = wave.position;
        wave.tick();
        assert_eq!(wave.position, (before + 2) & POSITION_MASK);
    }

    #[test]
    fn sample_is_scaled_by_volume_code() {
        for (register, expected) in [(0x00, 0.0f32), (0x20, 1.0), (0x40, 0.5), (0x60, 0.25)] {
            let mut wave = playing();
            wave.current = 15;
            wave.write_volume(register);
            assert_eq!(wave.sample().to_bits(), expected.to_bits());
        }
    }

    #[test]
    fn write_dac_off_silences() {
        let mut wave = playing();
        wave.write_dac(0);
        assert!(!wave.enabled);
        wave.write_control(TRIGGER);
        assert!(!wave.enabled, "a powered-off DAC cannot be triggered");
    }

    #[test]
    fn read_ram_while_playing_hits_current_byte() {
        let wave = playing();
        assert_eq!(wave.read_ram(9), wave.read_ram(0));
        let mut stopped = playing();
        stopped.write_dac(0);
        assert_eq!(stopped.read_ram(9), 0x96);
    }

    #[test]
    fn clock_length_expiry_silences() {
        let mut wave = playing();
        wave.write_length(0xFF);
        wave.write_control(Control {
            length_enabled: true,
            trigger: false,
            ..TRIGGER
        });
        wave.clock_length();
        assert!(!wave.enabled);
    }
}
