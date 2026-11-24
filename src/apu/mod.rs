//! Audio: four channels mixed to mono at the host's sample rate.

mod envelope;
mod length;
mod noise;
mod square;
mod sweep;
#[cfg(test)]
mod tests;
mod wave;

use crate::hardware::{M_CYCLES_PER_SECOND, OPEN_BUS};
use noise::Noise;
use square::Square;
use std::collections::VecDeque;
use wave::Wave;

const SEQUENCER_STEP_MASK: u8 = 7;
/// Largest 11-bit channel frequency.
const MAX_FREQUENCY: u16 = 0x7FF;
/// Queue bound (~170 ms at 48 kHz): a slow host loses the oldest samples.
const MAX_BUFFERED_SAMPLES: usize = 8192;
/// Keeps the sum of four full-scale channels within `[-1.0, 1.0]`.
const CHANNEL_GAIN: f32 = 0.25;
/// Two sides with volumes 1-8 each.
const MAX_SIDE_GAIN: f32 = 16.0;

const REGISTER_BASE: u16 = 0xFF10;
const REGISTER_COUNT: usize = 0x17;
const WAVE_RAM_BASE: u16 = 0xFF30;
const NR50: u16 = 0xFF24;
const NR51: u16 = 0xFF25;
const NR52: u16 = 0xFF26;
const POWER: u8 = 0x80;
const TRIGGER: u8 = 0x80;
const LENGTH_ENABLE: u8 = 0x40;
const FREQUENCY_HIGH_MASK: u8 = 0x07;
/// Bits that read as 1, from NR10 to NR52.
const READ_MASKS: [u8; REGISTER_COUNT] = [
    0x80, 0x3F, 0x00, 0xFF, 0xBF, // NR10-NR14
    0xFF, 0x3F, 0x00, 0xFF, 0xBF, // NR20-NR24
    0x7F, 0xFF, 0x9F, 0xFF, 0xBF, // NR30-NR34
    0xFF, 0xFF, 0x00, 0x00, 0xBF, // NR40-NR44
    0x00, 0x00, 0x70, // NR50-NR52
];
/// Registers as the DMG boot ROM leaves them; its chime has faded but CH1 is still on.
const POST_BOOT_WRITES: [(u16, u8); 5] = [
    (0xFF10, 0x80),
    (0xFF11, 0x80),
    (0xFF12, 0xF3),
    (NR50, 0x77),
    (NR51, 0xF3),
];

/// Decoded `NRx4` write.
#[derive(Clone, Copy, Debug)]
struct Control {
    frequency_high: u8,
    length_enabled: bool,
    trigger: bool,
    /// The next sequencer step does not clock lengths.
    between_clocks: bool,
}

/// Audio processing unit.
#[derive(Debug)]
pub struct Apu {
    square_1: Square,
    square_2: Square,
    wave: Wave,
    noise: Noise,
    registers: [u8; REGISTER_COUNT],
    powered: bool,
    /// Next sequencer step.
    sequencer_step: u8,
    sample_rate_hz: u32,
    /// Fractional sample position, in units of 1 / [`M_CYCLES_PER_SECOND`].
    sample_phase: u32,
    samples: VecDeque<f32>,
}

impl Apu {
    /// Creates an APU in its post-boot state.
    ///
    /// # Arguments
    /// * `sample_rate_hz` - output rate, capped at 1 048 576 Hz; zero disables output.
    #[must_use]
    pub fn new(sample_rate_hz: u32) -> Self {
        let mut apu = Self {
            square_1: Square::new(),
            square_2: Square::new(),
            wave: Wave::new(),
            noise: Noise::new(),
            registers: [0; REGISTER_COUNT],
            powered: true,
            sequencer_step: 0,
            sample_rate_hz: sample_rate_hz.min(M_CYCLES_PER_SECOND),
            sample_phase: 0,
            samples: VecDeque::with_capacity(MAX_BUFFERED_SAMPLES),
        };
        for (addr, val) in POST_BOOT_WRITES {
            apu.write(addr, val);
        }
        apu.square_1.enabled = true;
        apu
    }

    /// Moves the oldest samples into `out` and returns how many were written.
    ///
    /// # Complexity
    /// O(`out.len()`) time, no allocation.
    pub fn drain_samples(&mut self, out: &mut [f32]) -> usize {
        let count = out.len().min(self.samples.len());
        for (slot, sample) in out.iter_mut().zip(self.samples.drain(..count)) {
            *slot = sample;
        }
        count
    }

    /// Returns the number of queued samples.
    #[must_use]
    pub fn buffered_samples(&self) -> usize {
        self.samples.len()
    }

    /// Advances one M-cycle.
    ///
    /// # Arguments
    /// * `sequencer_clock` - the divider clocks the 512 Hz sequencer.
    pub fn tick(&mut self, sequencer_clock: bool) {
        if self.powered {
            self.square_1.tick();
            self.square_2.tick();
            self.wave.tick();
            self.noise.tick();
            if sequencer_clock {
                self.clock_sequencer();
            }
        }
        if self.sample_due() {
            let sample = self.mix();
            self.emit(sample);
        }
    }

    /// Lets one M-cycle pass with the system clock off: time flows, silently.
    pub(crate) fn tick_stopped(&mut self) {
        if self.sample_due() {
            self.emit(0.0);
        }
    }

    fn sample_due(&mut self) -> bool {
        self.sample_phase += self.sample_rate_hz;
        if self.sample_phase >= M_CYCLES_PER_SECOND {
            self.sample_phase -= M_CYCLES_PER_SECOND;
            return true;
        }
        false
    }

    /// Reads an APU register or wave RAM; other addresses are open bus.
    #[must_use]
    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0xFF30..=0xFF3F => self.wave.read_ram(offset(addr, WAVE_RAM_BASE)),
            NR52 => self.status(),
            _ => {
                let idx = offset(addr, REGISTER_BASE);
                match (self.registers.get(idx), READ_MASKS.get(idx)) {
                    (Some(register), Some(mask)) => register | mask,
                    _ => OPEN_BUS,
                }
            }
        }
    }

    /// Writes an APU register or wave RAM; other addresses are ignored.
    pub fn write(&mut self, addr: u16, val: u8) {
        match addr {
            0xFF30..=0xFF3F => self.wave.write_ram(offset(addr, WAVE_RAM_BASE), val),
            NR52 => self.write_power(val),
            _ => {
                self.write_length(addr, val);
                if self.powered {
                    self.write_register(addr, val);
                }
            }
        }
    }

    /// DMG length counters stay writable while powered off.
    fn write_length(&mut self, addr: u16, val: u8) {
        match addr {
            0xFF11 => self.square_1.write_length(val),
            0xFF16 => self.square_2.write_length(val),
            0xFF1B => self.wave.write_length(val),
            0xFF20 => self.noise.write_length(val),
            _ => {}
        }
    }

    fn write_register(&mut self, addr: u16, val: u8) {
        if let Some(register) = self.registers.get_mut(offset(addr, REGISTER_BASE)) {
            *register = val;
        }
        match addr {
            0xFF10 => self.square_1.write_sweep(val),
            0xFF11..=0xFF14 => {
                let control = self.control(val);
                write_square(&mut self.square_1, addr - 0xFF10, val, control);
            }
            0xFF16..=0xFF19 => {
                let control = self.control(val);
                write_square(&mut self.square_2, addr - 0xFF15, val, control);
            }
            0xFF1A..=0xFF1E => self.write_wave(addr, val),
            0xFF21 => self.noise.write_envelope(val),
            0xFF22 => self.noise.write_polynomial(val),
            0xFF23 => self.noise.write_control(self.control(val)),
            _ => {}
        }
    }

    fn write_wave(&mut self, addr: u16, val: u8) {
        match addr {
            0xFF1A => self.wave.write_dac(val),
            0xFF1C => self.wave.write_volume(val),
            0xFF1D => self.wave.write_frequency_low(val),
            0xFF1E => self.wave.write_control(self.control(val)),
            _ => {}
        }
    }

    fn write_power(&mut self, val: u8) {
        let powered = val & POWER != 0;
        if self.powered && !powered {
            for addr in REGISTER_BASE..NR52 {
                if !matches!(addr, 0xFF11 | 0xFF16 | 0xFF1B | 0xFF20) {
                    self.write_register(addr, 0);
                }
            }
            self.square_1.write_duty(0);
            self.square_2.write_duty(0);
            self.registers = [0; REGISTER_COUNT];
            self.square_1.length.power_off();
            self.square_2.length.power_off();
            self.wave.length.power_off();
            self.noise.length.power_off();
        }
        if !self.powered && powered {
            self.sequencer_step = 0;
        }
        self.powered = powered;
    }

    fn control(&self, val: u8) -> Control {
        Control {
            frequency_high: val & FREQUENCY_HIGH_MASK,
            length_enabled: val & LENGTH_ENABLE != 0,
            trigger: val & TRIGGER != 0,
            between_clocks: self.sequencer_step & 1 != 0,
        }
    }

    fn status(&self) -> u8 {
        let active = [
            self.square_1.enabled,
            self.square_2.enabled,
            self.wave.enabled,
            self.noise.enabled,
        ];
        let channels = active
            .iter()
            .rev()
            .fold(0, |bits, &enabled| (bits << 1) | u8::from(enabled));
        let power = if self.powered { POWER } else { 0 };
        power | 0x70 | channels
    }

    fn clock_sequencer(&mut self) {
        let step = self.sequencer_step;
        self.sequencer_step = (step + 1) & SEQUENCER_STEP_MASK;
        if step & 1 == 0 {
            self.square_1.clock_length();
            self.square_2.clock_length();
            self.wave.clock_length();
            self.noise.clock_length();
        }
        if matches!(step, 2 | 6) {
            self.square_1.clock_sweep();
        }
        if step == SEQUENCER_STEP_MASK {
            self.square_1.envelope.clock();
            self.square_2.envelope.clock();
            self.noise.envelope.clock();
        }
    }

    fn emit(&mut self, sample: f32) {
        if self.samples.len() >= MAX_BUFFERED_SAMPLES {
            self.samples.pop_front();
        }
        // Never reallocates: the capacity is reserved up front and just checked.
        self.samples.push_back(sample);
    }

    fn mix(&self) -> f32 {
        let volumes = self.read(NR50);
        let panning = self.read(NR51);
        let left_volume = ((volumes >> 4) & 7) + 1;
        let right_volume = (volumes & 7) + 1;
        let outputs = [
            self.square_1.sample(),
            self.square_2.sample(),
            self.wave.sample(),
            self.noise.sample(),
        ];
        let mut sides = panning;
        let mut mixed = 0.0;
        for output in outputs {
            let left = left_volume * ((sides >> 4) & 1);
            let right = right_volume * (sides & 1);
            mixed += output * f32::from(left + right);
            sides >>= 1;
        }
        mixed * (CHANNEL_GAIN / MAX_SIDE_GAIN)
    }
}

/// Writes register `NRx<number>` of a square channel.
fn write_square(square: &mut Square, number: u16, val: u8, control: Control) {
    match number {
        1 => square.write_duty(val),
        2 => square.write_envelope(val),
        3 => square.write_frequency_low(val),
        _ => square.write_control(control),
    }
}

/// Index of `addr` in a table starting at `base`; below it, wraps beyond any table.
fn offset(addr: u16, base: u16) -> usize {
    usize::from(addr.wrapping_sub(base))
}
