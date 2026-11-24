//! Square channels: CH1 and CH2.

use super::envelope::Envelope;
use super::length::Length;
use super::sweep::{Sweep, SweepStep};
use super::{Control, MAX_FREQUENCY};
use crate::hardware::T_CYCLES_PER_M_CYCLE;

const LENGTH_MAX: u16 = 64;
const LENGTH_MASK: u8 = 0x3F;
const DUTY_SHIFT: u32 = 6;
const DUTY_STEP_MASK: u8 = 7;
/// Waveforms by duty selector, first step in the high bit.
const DUTY_PATTERNS: [u8; 4] = [0b0000_0001, 0b1000_0001, 0b1000_0111, 0b0111_1110];

#[derive(Debug)]
pub(super) struct Square {
    pub(super) enabled: bool,
    pub(super) envelope: Envelope,
    pub(super) length: Length,
    /// Stays inert on CH2, whose NR20 does not exist.
    pub(super) sweep: Sweep,
    frequency: u16,
    timer: u16,
    /// Pattern of the selected duty.
    waveform: u8,
    duty_step: u8,
}

impl Square {
    pub(super) fn new() -> Self {
        Self {
            enabled: false,
            envelope: Envelope::default(),
            length: Length::new(LENGTH_MAX),
            sweep: Sweep::default(),
            frequency: 0,
            timer: period(0),
            waveform: DUTY_PATTERNS[0],
            duty_step: 0,
        }
    }

    /// Advances one M-cycle, which is also the shortest possible step.
    ///
    /// # Returns
    /// Whether the output may have changed.
    pub(super) fn tick(&mut self) -> bool {
        if !self.enabled {
            return false;
        }
        self.timer = self.timer.saturating_sub(T_CYCLES_PER_M_CYCLE);
        if self.timer == 0 {
            self.timer = period(self.frequency);
            self.duty_step = (self.duty_step + 1) & DUTY_STEP_MASK;
            return true;
        }
        false
    }

    pub(super) fn write_sweep(&mut self, val: u8) {
        if self.sweep.write(val) {
            self.enabled = false;
        }
    }

    pub(super) fn write_length(&mut self, val: u8) {
        self.length.load(val & LENGTH_MASK);
    }

    pub(super) fn write_duty(&mut self, val: u8) {
        let duty = usize::from(val >> DUTY_SHIFT);
        self.waveform = DUTY_PATTERNS.get(duty).copied().unwrap_or(0);
    }

    pub(super) fn write_envelope(&mut self, val: u8) {
        self.envelope.write(val, self.enabled);
        self.enabled &= self.envelope.dac_enabled();
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
            self.timer = period(self.frequency);
            self.envelope.trigger();
            let overflow = self.sweep.trigger(self.frequency);
            self.enabled = self.envelope.dac_enabled() && !overflow;
        }
    }

    pub(super) fn clock_length(&mut self) {
        if self.length.clock() {
            self.enabled = false;
        }
    }

    pub(super) fn clock_sweep(&mut self) {
        match self.sweep.clock() {
            SweepStep::Keep => {}
            SweepStep::Retune(frequency) => self.frequency = frequency,
            SweepStep::Silence => self.enabled = false,
        }
    }

    /// Current output, up to [`super::FULL_SCALE`] either way.
    pub(super) fn output(&self) -> i16 {
        if !self.enabled {
            return 0;
        }
        let amplitude = self.envelope.amplitude();
        if self.is_high() {
            amplitude
        } else {
            -amplitude
        }
    }

    fn is_high(&self) -> bool {
        self.waveform.rotate_left(u32::from(self.duty_step)) & 0x80 != 0
    }
}

fn period(frequency: u16) -> u16 {
    (MAX_FREQUENCY + 1).saturating_sub(frequency) * T_CYCLES_PER_M_CYCLE
}

#[cfg(test)]
mod tests {
    use super::super::FULL_SCALE;
    use super::*;

    fn playing(frequency: u16) -> Square {
        let mut square = Square::new();
        let [high, low] = frequency.to_be_bytes();
        square.write_duty(0x80);
        square.write_envelope(0xF0);
        square.write_frequency_low(low);
        square.write_control(Control {
            frequency_high: high,
            length_enabled: false,
            trigger: true,
            between_clocks: false,
        });
        square
    }

    #[test]
    fn tick_waveform_period_matches_frequency() {
        for frequency in [0, 0x7F9, 0x7FF] {
            let mut square = playing(frequency);
            let m_cycles = u32::from(2048 - frequency) * 8;
            let before = square.duty_step;
            (0..m_cycles - 1).for_each(|_| {
                square.tick();
            });
            assert_ne!(square.duty_step, before);
            square.tick();
            assert_eq!(square.duty_step, before);
        }
    }

    #[test]
    fn is_high_fraction_matches_duty() {
        for (register, expected) in [(0x00, 1), (0x40, 2), (0x80, 4), (0xC0, 6)] {
            let mut square = playing(0x7FF);
            square.write_duty(register);
            let high = (0..8)
                .filter(|_| {
                    square.tick();
                    square.is_high()
                })
                .count();
            assert_eq!(high, expected, "{register:#x}");
        }
    }

    #[test]
    fn write_envelope_disabling_dac_silences() {
        let mut square = playing(0);
        assert!(square.enabled);
        square.write_envelope(0);
        assert!(!square.enabled);
        assert_eq!(square.output(), 0);
    }

    #[test]
    fn clock_sweep_overflow_silences() {
        let mut square = playing(0x500);
        square.write_sweep(0x11);
        square.write_control(Control {
            frequency_high: 5,
            length_enabled: false,
            trigger: true,
            between_clocks: false,
        });
        assert!(square.enabled);
        square.clock_sweep();
        assert!(!square.enabled);
    }

    #[test]
    fn clock_length_expiry_silences() {
        let mut square = playing(0);
        square.write_length(0x3F);
        square.write_control(Control {
            frequency_high: 0,
            length_enabled: true,
            trigger: false,
            between_clocks: false,
        });
        square.clock_length();
        assert!(!square.enabled);
    }

    #[test]
    fn output_amplitude_is_full_scale_at_max_volume() {
        let mut square = playing(0x7FF);
        for _ in 0..8 {
            square.tick();
            assert_eq!(square.output().abs(), FULL_SCALE);
        }
    }
}
