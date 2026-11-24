//! Length counter: silences a channel after a programmed time.

#[derive(Debug)]
pub(super) struct Length {
    remaining: u16,
    max: u16,
    enabled: bool,
}

impl Length {
    pub(super) fn new(max: u16) -> Self {
        Self {
            remaining: 0,
            max,
            enabled: false,
        }
    }

    /// Loads the counter from the value written to NRx1.
    pub(super) fn load(&mut self, elapsed: u8) {
        self.remaining = self.max.saturating_sub(u16::from(elapsed));
    }

    /// Applies the enable and trigger bits of NRx4.
    ///
    /// # Arguments
    /// * `between_clocks` - the next sequencer step does not clock lengths; hardware then
    ///   clocks once more when the counter becomes enabled.
    ///
    /// # Returns
    /// Whether the channel expired.
    pub(super) fn control(&mut self, enabled: bool, trigger: bool, between_clocks: bool) -> bool {
        let extra_clock = between_clocks && enabled && !self.enabled;
        self.enabled = enabled;
        let expired = extra_clock && self.clock();
        if !trigger {
            return expired;
        }
        if self.remaining == 0 {
            self.remaining = self.max - u16::from(enabled && between_clocks);
        }
        false
    }

    /// Clocks the counter and returns whether the channel expired.
    pub(super) fn clock(&mut self) -> bool {
        if !self.enabled || self.remaining == 0 {
            return false;
        }
        self.remaining -= 1;
        self.remaining == 0
    }

    /// Clears the enable bit on power-off; DMG keeps the counter itself.
    pub(super) fn power_off(&mut self) {
        self.enabled = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clocks_to_expire(length: &mut Length) -> usize {
        (1..=1000).find(|_| length.clock()).unwrap_or(0)
    }

    #[test]
    fn clock_expires_after_loaded_time() {
        let mut length = Length::new(64);
        length.load(60);
        assert!(!length.control(true, true, false));
        assert_eq!(clocks_to_expire(&mut length), 4);
        assert!(!length.clock());
    }

    #[test]
    fn clock_disabled_never_expires() {
        let mut length = Length::new(64);
        length.load(63);
        assert_eq!(clocks_to_expire(&mut length), 0);
    }

    #[test]
    fn control_trigger_reloads_empty_counter() {
        for (between_clocks, expected) in [(false, 256), (true, 255)] {
            let mut length = Length::new(256);
            length.control(true, true, between_clocks);
            assert_eq!(clocks_to_expire(&mut length), expected);
        }
    }

    #[test]
    fn control_enable_between_clocks_clocks_once() {
        let mut length = Length::new(64);
        length.load(63);
        assert!(length.control(true, false, true));
    }

    #[test]
    fn power_off_keeps_counter() {
        let mut length = Length::new(64);
        length.load(62);
        length.control(true, false, false);
        length.power_off();
        assert!(!length.clock());
        length.control(true, false, false);
        assert_eq!(clocks_to_expire(&mut length), 2);
    }
}
