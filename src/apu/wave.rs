//! Wave channel: CH3 plays 32 four-bit samples from wave RAM.

use super::length::Length;
use super::{Control, MAX_FREQUENCY};
use crate::hardware::{OPEN_BUS, T_CYCLES_PER_M_CYCLE};

const LENGTH_MAX: u16 = 256;
pub(super) const WAVE_RAM_LEN: usize = 16;
/// Bytes a retrigger overwrites, and the alignment it copies from.
const CORRUPTION_LEN: usize = 4;
const POSITION_MASK: u8 = 31;
const VOLUME_SHIFT: u32 = 5;
const VOLUME_MASK: u8 = 3;
const NIBBLE_BITS: u32 = 4;
const NIBBLE_MASK: u8 = 0x0F;
const DAC_ENABLE: u8 = 0x80;
const T_CYCLES_PER_PERIOD_UNIT: u16 = 2;
/// Extra delay before the first sample after a trigger.
const TRIGGER_DELAY_T_CYCLES: u16 = 6;
/// Largest 4-bit sample.
const SAMPLE_MAX: i16 = 15;

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
    /// Wave RAM is on the bus at the moment of a CPU access.
    just_read: bool,
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
            just_read: false,
        }
    }

    /// Advances one M-cycle; the shortest period fits twice in it.
    ///
    /// # Returns
    /// Whether the output may have changed.
    pub(super) fn tick(&mut self) -> bool {
        if !self.enabled {
            return false;
        }
        let mut changed = false;
        let mut elapsed = T_CYCLES_PER_M_CYCLE;
        self.just_read = false;
        while elapsed >= self.timer {
            elapsed -= self.timer;
            self.timer = period(self.frequency);
            self.position = (self.position + 1) & POSITION_MASK;
            self.current = self.nibble();
            changed = true;
            // CPU accesses land on the last T-cycles of the M-cycle.
            self.just_read = elapsed == 0;
        }
        self.timer -= elapsed;
        changed
    }

    pub(super) fn write_dac(&mut self, val: u8) {
        self.dac_enabled = val & DAC_ENABLE != 0;
        self.enabled &= self.dac_enabled;
    }

    pub(super) fn write_length(&mut self, val: u8) {
        self.length.load(val);
    }

    pub(super) fn write_volume(&mut self, val: u8) {
        self.volume_code = (val >> VOLUME_SHIFT) & VOLUME_MASK;
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
            if self.enabled && self.timer == T_CYCLES_PER_PERIOD_UNIT {
                self.corrupt_ram();
            }
            self.just_read = false;
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

    /// Reads wave RAM; while playing, only the byte the channel is reading answers.
    pub(super) fn read_ram(&self, idx: usize) -> u8 {
        self.ram_idx(idx)
            .and_then(|idx| self.ram.get(idx))
            .copied()
            .unwrap_or(OPEN_BUS)
    }

    /// Writes wave RAM; while playing, only the byte the channel is reading is reachable.
    pub(super) fn write_ram(&mut self, idx: usize, val: u8) {
        if let Some(cell) = self.ram_idx(idx).and_then(|idx| self.ram.get_mut(idx)) {
            *cell = val;
        }
    }

    /// DMG quirk: retriggering as a byte is fetched overwrites the start of wave RAM.
    fn corrupt_ram(&mut self) {
        let fetched = usize::from(((self.position + 1) & POSITION_MASK) >> 1);
        if fetched < CORRUPTION_LEN {
            let byte = self.ram.get(fetched).copied().unwrap_or(0);
            if let Some(first) = self.ram.first_mut() {
                *first = byte;
            }
        } else {
            let start = fetched - fetched % CORRUPTION_LEN;
            self.ram.copy_within(start..start + CORRUPTION_LEN, 0);
        }
    }

    /// Current output, up to [`super::FULL_SCALE`] either way.
    pub(super) fn output(&self) -> i16 {
        if !self.enabled {
            return 0;
        }
        let scale = match self.volume_code {
            0 => 0,
            1 => 4,
            2 => 2,
            _ => 1,
        };
        // Twice the sample minus its maximum: centred on zero.
        (i16::from(self.current) * 2 - SAMPLE_MAX) * scale
    }

    fn ram_idx(&self, idx: usize) -> Option<usize> {
        match (self.enabled, self.just_read) {
            (false, _) => Some(idx),
            (true, true) => Some(usize::from(self.position >> 1)),
            (true, false) => None,
        }
    }

    fn nibble(&self) -> u8 {
        let byte = self
            .ram
            .get(usize::from(self.position >> 1))
            .copied()
            .unwrap_or(0);
        if self.position & 1 == 0 {
            byte >> NIBBLE_BITS
        } else {
            byte & NIBBLE_MASK
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
        (0..8).for_each(|_| {
            wave.tick();
        });
        let before = wave.position;
        wave.tick();
        assert_eq!(wave.position, (before + 2) & POSITION_MASK);
    }

    #[test]
    fn output_is_scaled_by_volume_code() {
        for (register, expected) in [(0x00, 0), (0x20, 60), (0x40, 30), (0x60, 15)] {
            let mut wave = playing();
            wave.current = 15;
            wave.write_volume(register);
            assert_eq!(wave.output(), expected);
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
    fn read_ram_while_playing_needs_fetch_cycle() {
        let mut wave = playing();
        wave.tick();
        assert_eq!(wave.read_ram(9), OPEN_BUS);
        wave.write_ram(9, 0);
        wave.write_frequency_low(0xFE);
        wave.tick();
        wave.tick();
        assert!(!wave.just_read);
        wave.timer = T_CYCLES_PER_M_CYCLE;
        wave.tick();
        assert_eq!(wave.read_ram(9), wave.ram[usize::from(wave.position >> 1)]);
        wave.write_dac(0);
        assert_eq!(wave.read_ram(9), 0x96);
    }

    #[test]
    fn write_control_retrigger_during_fetch_corrupts_ram() {
        for (position, expected) in [(3, [0x2D, 0x1E, 0x2D, 0x3C]), (9, [0x4B, 0x5A, 0x69, 0x78])] {
            let mut wave = playing();
            wave.position = position;
            wave.timer = T_CYCLES_PER_PERIOD_UNIT;
            wave.write_control(TRIGGER);
            assert_eq!(wave.ram[..4], expected);
        }
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
