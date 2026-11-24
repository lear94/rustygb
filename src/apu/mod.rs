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
use crate::memory_map::{
    NR10, NR11, NR12, NR14, NR20, NR21, NR24, NR30, NR31, NR32, NR33, NR34, NR41, NR42, NR43, NR44,
    NR50, NR51, NR52, WAVE_END, WAVE_START,
};
use noise::Noise;
use square::Square;
use std::collections::VecDeque;
use wave::Wave;

const SWEEP_STEPS: [u8; 2] = [2, 6];
const SEQUENCER_STEP_MASK: u8 = 7;
/// Largest 11-bit channel frequency.
const MAX_FREQUENCY: u16 = 0x7FF;
/// Queue bound (~170 ms at 48 kHz): a slow host loses the oldest samples.
const MAX_BUFFERED_SAMPLES: usize = 8192;
/// Largest channel output: divisible by every envelope volume and wave scale.
const FULL_SCALE: i16 = 60;
/// Two sides with volumes 1-8 each.
const MAX_GAIN: i16 = 16;
const CHANNELS: i16 = 4;
const CHANNEL_COUNT: usize = 4;
const _: () = assert!(CHANNELS as usize == CHANNEL_COUNT);
/// Full-scale channels at full gain.
const MIX_FULL_SCALE: i16 = FULL_SCALE * MAX_GAIN * CHANNELS;

const REGISTER_BASE: u16 = 0xFF10;
const REGISTER_COUNT: usize = 0x17;
const WAVE_RAM_BASE: u16 = 0xFF30;
/// Left side in the high nibble, right side in the low one.
const SIDE_SHIFT: u32 = 4;
const SIDE_VOLUME_MASK: u8 = 7;
/// NR52 bits that do not exist read as 1.
const STATUS_UNUSED: u8 = 0x70;
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
    (NR10, 0x80),
    (NR11, 0x80),
    (NR12, 0xF3),
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

/// Sequencer start-up: powering on halfway through a clock period swallows its edge, and
/// until the next one lengths behave as if step 0 were already done.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PowerOn {
    Settled,
    SkipClock,
    Skipped,
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
    power_on: PowerOn,
    sample_rate_hz: u32,
    /// Fractional sample position, in units of 1 / [`M_CYCLES_PER_SECOND`].
    sample_phase: u32,
    channel_gains: [i16; CHANNEL_COUNT],
    /// Current mixed output, up to [`MIX_FULL_SCALE`]; scaled once per sample.
    level: f32,
    /// Sum and count of the levels since the last sample.
    level_sum: f32,
    level_count: f32,
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
            power_on: PowerOn::Settled,
            sample_rate_hz: capped_rate(sample_rate_hz),
            sample_phase: 0,
            channel_gains: [0; CHANNEL_COUNT],
            level: 0.0,
            level_sum: 0.0,
            level_count: 0.0,
            samples: VecDeque::with_capacity(MAX_BUFFERED_SAMPLES),
        };
        for (addr, val) in POST_BOOT_WRITES {
            apu.write(addr, val, false);
        }
        apu.square_1.enabled = true;
        apu
    }

    /// Changes the output rate; see [`Self::new`].
    pub fn set_sample_rate(&mut self, sample_rate_hz: u32) {
        self.sample_rate_hz = capped_rate(sample_rate_hz);
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
            // Non-short-circuiting: every channel advances.
            let changed =
                self.square_1.tick() | self.square_2.tick() | self.wave.tick() | self.noise.tick();
            if sequencer_clock {
                self.clock_sequencer();
            }
            if changed | sequencer_clock {
                self.level = self.mix();
            }
        }
        self.accumulate(self.level);
    }

    /// Lets one M-cycle pass with the system clock off: time flows, silently.
    pub(crate) fn tick_stopped(&mut self) {
        self.accumulate(0.0);
    }

    /// Averages the output over each sample, or channels above the audible range would
    /// alias into it.
    fn accumulate(&mut self, level: f32) {
        self.level_sum += level;
        self.level_count += 1.0;
        if self.sample_due() {
            let sample = self.level_sum / (self.level_count * f32::from(MIX_FULL_SCALE));
            self.level_sum = 0.0;
            self.level_count = 0.0;
            self.emit(sample);
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
            WAVE_START..=WAVE_END => self.wave.read_ram(offset(addr, WAVE_RAM_BASE)),
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
    ///
    /// # Arguments
    /// * `clock_high` - level of the divider bit clocking the sequencer.
    pub fn write(&mut self, addr: u16, val: u8, clock_high: bool) {
        match addr {
            WAVE_START..=WAVE_END => self.wave.write_ram(offset(addr, WAVE_RAM_BASE), val),
            NR52 => self.write_power(val, clock_high),
            _ => {
                self.write_length(addr, val);
                if self.powered {
                    self.write_register(addr, val);
                }
            }
        }
        self.update_gains();
        self.level = self.mix();
    }

    /// DMG length counters stay writable while powered off.
    fn write_length(&mut self, addr: u16, val: u8) {
        match addr {
            NR11 => self.square_1.write_length(val),
            NR21 => self.square_2.write_length(val),
            NR31 => self.wave.write_length(val),
            NR41 => self.noise.write_length(val),
            _ => {}
        }
    }

    fn write_register(&mut self, addr: u16, val: u8) {
        if let Some(register) = self.registers.get_mut(offset(addr, REGISTER_BASE)) {
            *register = val;
        }
        match addr {
            NR10 => self.square_1.write_sweep(val),
            NR11..=NR14 => {
                let control = self.control(val);
                write_square(&mut self.square_1, addr - NR10, val, control);
            }
            NR21..=NR24 => {
                let control = self.control(val);
                write_square(&mut self.square_2, addr - NR20, val, control);
            }
            NR30..=NR34 => self.write_wave(addr, val),
            NR42 => self.noise.write_envelope(val),
            NR43 => self.noise.write_polynomial(val),
            NR44 => self.noise.write_control(self.control(val)),
            _ => {}
        }
    }

    fn write_wave(&mut self, addr: u16, val: u8) {
        match addr {
            NR30 => self.wave.write_dac(val),
            NR32 => self.wave.write_volume(val),
            NR33 => self.wave.write_frequency_low(val),
            NR34 => self.wave.write_control(self.control(val)),
            _ => {}
        }
    }

    fn write_power(&mut self, val: u8, clock_high: bool) {
        let powered = val & POWER != 0;
        if self.powered && !powered {
            for addr in REGISTER_BASE..NR52 {
                if !matches!(addr, NR11 | NR21 | NR31 | NR41) {
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
            self.power_on = if clock_high {
                PowerOn::SkipClock
            } else {
                PowerOn::Settled
            };
        }
        self.powered = powered;
    }

    fn control(&self, val: u8) -> Control {
        Control {
            frequency_high: val & FREQUENCY_HIGH_MASK,
            length_enabled: val & LENGTH_ENABLE != 0,
            trigger: val & TRIGGER != 0,
            between_clocks: self.sequencer_step & 1 != 0 || self.power_on != PowerOn::Settled,
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
        power | STATUS_UNUSED | channels
    }

    fn clock_sequencer(&mut self) {
        if self.power_on == PowerOn::SkipClock {
            self.power_on = PowerOn::Skipped;
            return;
        }
        self.power_on = PowerOn::Settled;
        let step = self.sequencer_step;
        self.sequencer_step = (step + 1) & SEQUENCER_STEP_MASK;
        if step & 1 == 0 {
            self.square_1.clock_length();
            self.square_2.clock_length();
            self.wave.clock_length();
            self.noise.clock_length();
        }
        if SWEEP_STEPS.contains(&step) {
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

    /// Mixes the channels in integers: this runs whenever an output changes, up to every
    /// M-cycle, and only the result needs to be a float.
    fn mix(&self) -> f32 {
        let [gain_1, gain_2, gain_3, gain_4] = self.channel_gains;
        let sum = self.square_1.output() * gain_1
            + self.square_2.output() * gain_2
            + self.wave.output() * gain_3
            + self.noise.output() * gain_4;
        f32::from(sum)
    }

    /// Recomputes the channel gains from the master volumes and panning.
    fn update_gains(&mut self) {
        let volumes = self.read(NR50);
        let panning = self.read(NR51);
        let left_volume = i16::from((volumes >> SIDE_SHIFT) & SIDE_VOLUME_MASK) + 1;
        let right_volume = i16::from(volumes & SIDE_VOLUME_MASK) + 1;
        let mut sides = panning;
        for gain in &mut self.channel_gains {
            let left = left_volume * i16::from((sides >> SIDE_SHIFT) & 1);
            let right = right_volume * i16::from(sides & 1);
            *gain = left + right;
            sides >>= 1;
        }
    }
}

/// At most one sample per M-cycle.
fn capped_rate(sample_rate_hz: u32) -> u32 {
    sample_rate_hz.min(M_CYCLES_PER_SECOND)
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
