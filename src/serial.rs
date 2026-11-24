//! Serial port with no link partner attached.

const START: u8 = 0x80;
const INTERNAL_CLOCK: u8 = 0x01;
const UNUSED: u8 = 0x7E;
/// 8 bits at 8192 Hz, in M-cycles.
const TRANSFER_M_CYCLES: u16 = 1024;
/// A disconnected line idles high, so every received bit is 1.
const DISCONNECTED: u8 = 0xFF;

/// Serial registers SB and SC.
#[derive(Debug, Default)]
pub struct Serial {
    data: u8,
    control: u8,
    remaining: u16,
}

impl Serial {
    /// Creates an idle port.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Advances one M-cycle.
    ///
    /// # Returns
    /// Whether the serial interrupt is requested.
    pub fn tick(&mut self) -> bool {
        if self.remaining == 0 {
            return false;
        }
        self.remaining -= 1;
        if self.remaining != 0 {
            return false;
        }
        self.data = DISCONNECTED;
        self.control &= !START;
        true
    }

    /// Reads SB.
    #[must_use]
    pub fn data(&self) -> u8 {
        self.data
    }

    /// Reads SC.
    #[must_use]
    pub fn control(&self) -> u8 {
        self.control | UNUSED
    }

    /// Writes SB.
    pub fn write_data(&mut self, val: u8) {
        self.data = val;
    }

    /// Writes SC. A transfer on the external clock never completes: nobody drives it.
    pub fn write_control(&mut self, val: u8) {
        self.control = val & !UNUSED;
        self.remaining = if val & START != 0 && val & INTERNAL_CLOCK != 0 {
            TRANSFER_M_CYCLES
        } else {
            0
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_internal_clock_receives_ones() {
        let mut serial = Serial::new();
        serial.write_data(0x42);
        serial.write_control(0x81);
        assert!((1..TRANSFER_M_CYCLES).all(|_| !serial.tick()));
        assert_eq!((serial.data(), serial.control()), (0x42, 0xFF));
        assert!(serial.tick());
        assert_eq!((serial.data(), serial.control()), (0xFF, 0x7F));
        assert!(!serial.tick());
    }

    #[test]
    fn tick_external_clock_never_completes() {
        let mut serial = Serial::new();
        serial.write_control(0x80);
        assert!((0..TRANSFER_M_CYCLES * 2).all(|_| !serial.tick()));
        assert_eq!(serial.control(), 0xFE);
    }
}
