//! Volume envelope shared by the square and noise channels.

const MAX_VOLUME: u8 = 15;
/// A zero period behaves as 8 in the timer.
const ZERO_PERIOD: u8 = 8;
const PERIOD_MASK: u8 = 0x07;
const RISING: u8 = 0x08;
/// A DAC is powered while any of these `NRx2` bits is set.
const DAC_MASK: u8 = 0xF8;

#[derive(Debug, Default)]
pub(super) struct Envelope {
    register: u8,
    volume: u8,
    timer: u8,
}

impl Envelope {
    pub(super) fn write(&mut self, register: u8) {
        self.register = register;
    }

    pub(super) fn dac_enabled(&self) -> bool {
        self.register & DAC_MASK != 0
    }

    #[cfg(test)]
    pub(super) fn volume(&self) -> u8 {
        self.volume
    }

    /// Volume as a fraction of full scale.
    pub(super) fn amplitude(&self) -> f32 {
        f32::from(self.volume) / f32::from(MAX_VOLUME)
    }

    pub(super) fn trigger(&mut self) {
        self.volume = self.register >> 4;
        self.timer = self.period();
    }

    pub(super) fn clock(&mut self) {
        if self.register & PERIOD_MASK == 0 {
            return;
        }
        self.timer = self.timer.saturating_sub(1);
        if self.timer != 0 {
            return;
        }
        self.timer = self.period();
        self.volume = if self.register & RISING != 0 {
            (self.volume + 1).min(MAX_VOLUME)
        } else {
            self.volume.saturating_sub(1)
        };
    }

    fn period(&self) -> u8 {
        match self.register & PERIOD_MASK {
            0 => ZERO_PERIOD,
            period => period,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn volume_after(register: u8, clocks: usize) -> u8 {
        let mut envelope = Envelope::default();
        envelope.write(register);
        envelope.trigger();
        (0..clocks).for_each(|_| envelope.clock());
        envelope.volume()
    }

    #[test]
    fn clock_moves_volume_each_period() {
        for (register, clocks, expected) in [
            (0xF1, 0, 15),
            (0xF1, 1, 14),
            (0xF2, 3, 14),
            (0x11, 5, 0),
            (0xE9, 5, 15),
            (0xF0, 50, 15),
        ] {
            assert_eq!(volume_after(register, clocks), expected, "{register:#x}");
        }
    }

    #[test]
    fn dac_enabled_requires_volume_or_direction() {
        for (register, expected) in [(0x00, false), (0x07, false), (0x08, true), (0x10, true)] {
            let mut envelope = Envelope::default();
            envelope.write(register);
            assert_eq!(envelope.dac_enabled(), expected);
        }
    }
}
