//! Frequency sweep of the first square channel.

use super::MAX_FREQUENCY;

/// A zero period behaves as 8 in the timer.
const ZERO_PERIOD: u8 = 8;
const NEGATE: u8 = 0x08;
const SHIFT_MASK: u8 = 0x07;

/// What a sweep step does to its channel.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum SweepStep {
    Keep,
    Retune(u16),
    Silence,
}

#[derive(Debug, Default)]
pub(super) struct Sweep {
    register: u8,
    enabled: bool,
    shadow: u16,
    timer: u8,
    /// A negated calculation happened since the trigger.
    negated: bool,
}

impl Sweep {
    /// Writes NR10; returns whether the channel must be silenced.
    pub(super) fn write(&mut self, register: u8) -> bool {
        self.register = register;
        // Hardware quirk: leaving negate mode after using it kills the channel.
        self.negated && register & NEGATE == 0
    }

    /// Restarts the sweep; returns whether the channel must be silenced.
    pub(super) fn trigger(&mut self, frequency: u16) -> bool {
        self.shadow = frequency;
        self.timer = self.timer_reload();
        self.negated = false;
        self.enabled = self.period() != 0 || self.shift() != 0;
        self.shift() != 0 && self.target() > MAX_FREQUENCY
    }

    pub(super) fn clock(&mut self) -> SweepStep {
        self.timer = self.timer.saturating_sub(1);
        if self.timer != 0 {
            return SweepStep::Keep;
        }
        self.timer = self.timer_reload();
        if !self.enabled || self.period() == 0 {
            return SweepStep::Keep;
        }
        let target = self.target();
        if target > MAX_FREQUENCY {
            return SweepStep::Silence;
        }
        if self.shift() == 0 {
            return SweepStep::Keep;
        }
        self.shadow = target;
        if self.target() > MAX_FREQUENCY {
            return SweepStep::Silence;
        }
        SweepStep::Retune(target)
    }

    fn target(&mut self) -> u16 {
        let delta = self.shadow >> self.shift();
        if self.register & NEGATE != 0 {
            self.negated = true;
            // Never underflows: the delta is at most the shadow itself.
            self.shadow.saturating_sub(delta)
        } else {
            self.shadow.saturating_add(delta)
        }
    }

    fn period(&self) -> u8 {
        (self.register >> 4) & 7
    }

    fn shift(&self) -> u8 {
        self.register & SHIFT_MASK
    }

    fn timer_reload(&self) -> u8 {
        match self.period() {
            0 => ZERO_PERIOD,
            period => period,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn triggered(register: u8, frequency: u16) -> Sweep {
        let mut sweep = Sweep::default();
        sweep.write(register);
        assert!(!sweep.trigger(frequency));
        sweep
    }

    #[test]
    fn clock_retunes_up_and_down() {
        for (register, expected) in [(0x11, 0x300), (0x19, 0x100)] {
            let mut sweep = triggered(register, 0x200);
            assert_eq!(sweep.clock(), SweepStep::Retune(expected));
        }
    }

    #[test]
    fn clock_waits_for_period() {
        let mut sweep = triggered(0x21, 0x200);
        assert_eq!(sweep.clock(), SweepStep::Keep);
        assert_eq!(sweep.clock(), SweepStep::Retune(0x300));
    }

    #[test]
    fn clock_second_overflow_silences() {
        let mut sweep = triggered(0x11, 0x500);
        assert_eq!(
            sweep.clock(),
            SweepStep::Silence,
            "second calculation overflows"
        );
    }

    #[test]
    fn clock_without_shift_or_period_keeps() {
        for register in [0x10, 0x01] {
            let mut sweep = triggered(register, 0x200);
            for _ in 0..8 {
                assert_eq!(sweep.clock(), SweepStep::Keep);
            }
        }
    }

    #[test]
    fn trigger_initial_overflow_silences() {
        let mut sweep = Sweep::default();
        sweep.write(0x11);
        assert!(sweep.trigger(0x7FF));
    }

    #[test]
    fn write_leaving_negate_after_use_silences() {
        let mut sweep = triggered(0x19, 0x200);
        assert!(sweep.write(0x11));
        let mut unused = triggered(0x18, 0x200);
        assert!(!unused.write(0x10));
    }
}
