//! Noise channel: CH4 plays a linear-feedback shift register.

use super::envelope::Envelope;
use super::length::Length;
use super::{Control, T_CYCLES_PER_TICK};

const LENGTH_MAX: u16 = 64;
const LENGTH_MASK: u8 = 0x3F;
const DIVISORS_T_CYCLES: [u32; 8] = [8, 16, 32, 48, 64, 80, 96, 112];
const DIVISOR_MASK: u8 = 0x07;
const NARROW: u8 = 0x08;
/// Shifts above this stop the register clock.
const MAX_SHIFT: u8 = 13;
const LFSR_SEED: u16 = 0x7FFF;
const LFSR_FEEDBACK_SHIFT: u32 = 14;
const LFSR_NARROW_SHIFT: u32 = 6;

#[derive(Debug)]
pub(super) struct Noise {
    pub(super) enabled: bool,
    pub(super) envelope: Envelope,
    pub(super) length: Length,
    register: u8,
    timer: u32,
    lfsr: u16,
}

impl Noise {
    pub(super) fn new() -> Self {
        Self {
            enabled: false,
            envelope: Envelope::default(),
            length: Length::new(LENGTH_MAX),
            register: 0,
            timer: period(0),
            lfsr: LFSR_SEED,
        }
    }

    /// Advances one M-cycle; the shortest period spans two.
    pub(super) fn tick(&mut self) {
        self.timer = self.timer.saturating_sub(u32::from(T_CYCLES_PER_TICK));
        if self.timer != 0 {
            return;
        }
        self.timer = period(self.register);
        if self.register >> 4 <= MAX_SHIFT {
            self.shift();
        }
    }

    pub(super) fn write_length(&mut self, val: u8) {
        self.length.load(val & LENGTH_MASK);
    }

    pub(super) fn write_envelope(&mut self, val: u8) {
        self.envelope.write(val);
        self.enabled &= self.envelope.dac_enabled();
    }

    pub(super) fn write_polynomial(&mut self, val: u8) {
        self.register = val;
    }

    pub(super) fn write_control(&mut self, control: Control) {
        if self.length.control(
            control.length_enabled,
            control.trigger,
            control.between_clocks,
        ) {
            self.enabled = false;
        }
        if control.trigger {
            self.timer = period(self.register);
            self.lfsr = LFSR_SEED;
            self.envelope.trigger();
            self.enabled = self.envelope.dac_enabled();
        }
    }

    pub(super) fn clock_length(&mut self) {
        if self.length.clock() {
            self.enabled = false;
        }
    }

    /// Current output in `[-1.0, 1.0]`.
    pub(super) fn sample(&self) -> f32 {
        if !self.enabled {
            return 0.0;
        }
        let amplitude = f32::from(self.envelope.volume()) / 15.0;
        if self.lfsr & 1 == 0 {
            amplitude
        } else {
            -amplitude
        }
    }

    fn shift(&mut self) {
        let feedback = (self.lfsr ^ (self.lfsr >> 1)) & 1;
        self.lfsr = (self.lfsr >> 1) | (feedback << LFSR_FEEDBACK_SHIFT);
        if self.register & NARROW != 0 {
            self.lfsr = (self.lfsr & !(1 << LFSR_NARROW_SHIFT)) | (feedback << LFSR_NARROW_SHIFT);
        }
    }
}

fn period(register: u8) -> u32 {
    let divisor = DIVISORS_T_CYCLES
        .get(usize::from(register & DIVISOR_MASK))
        .copied()
        .unwrap_or(0);
    divisor << (register >> 4)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRIGGER: Control = Control {
        frequency_high: 0,
        length_enabled: false,
        trigger: true,
        between_clocks: false,
    };

    fn playing(register: u8) -> Noise {
        let mut noise = Noise::new();
        noise.write_envelope(0xF0);
        noise.write_polynomial(register);
        noise.write_control(TRIGGER);
        noise
    }

    fn sequence_len(register: u8) -> usize {
        let mut noise = playing(register);
        // The narrow sequence only repeats once the seed has left the low bits.
        (0..64).for_each(|_| noise.shift());
        let start = noise.lfsr;
        (1..=0x8000)
            .find(|_| {
                noise.shift();
                noise.lfsr == start
            })
            .unwrap_or(0)
    }

    #[test]
    fn shift_sequence_length_depends_on_width() {
        assert_eq!(sequence_len(0x00), 32_767);
        assert_eq!(sequence_len(NARROW), 127);
    }

    #[test]
    fn tick_shifts_once_per_period() {
        for (register, m_cycles) in [(0x00, 2), (0x01, 4), (0x10, 4), (0x27, 112)] {
            let mut noise = playing(register);
            (0..m_cycles - 1).for_each(|_| noise.tick());
            assert_eq!(noise.lfsr, LFSR_SEED);
            noise.tick();
            assert_ne!(noise.lfsr, LFSR_SEED, "{register:#x}");
        }
    }

    #[test]
    fn tick_shift_above_limit_is_frozen() {
        let mut noise = playing(0xE0);
        (0..100_000).for_each(|_| noise.tick());
        assert_eq!(noise.lfsr, LFSR_SEED);
    }

    #[test]
    fn write_envelope_disabling_dac_silences() {
        let mut noise = playing(0);
        noise.write_envelope(0);
        assert!(!noise.enabled);
        assert_eq!(noise.sample().to_bits(), 0.0f32.to_bits());
    }

    #[test]
    fn clock_length_expiry_silences() {
        let mut noise = playing(0);
        noise.write_length(0x3F);
        noise.write_control(Control {
            length_enabled: true,
            trigger: false,
            ..TRIGGER
        });
        noise.clock_length();
        assert!(!noise.enabled);
    }

    #[test]
    fn sample_is_full_scale_at_max_volume() {
        assert_eq!(playing(0).sample().to_bits(), (-1.0f32).to_bits());
    }
}
