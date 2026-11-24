//! Serial port with no link partner attached.

const START: u8 = 0x80;
const INTERNAL_CLOCK: u8 = 0x01;
const UNUSED: u8 = 0x7E;
const TRANSFER_BITS: u8 = 8;
/// A disconnected line idles high.
const DISCONNECTED_BIT: u8 = 1;

/// Serial registers SB and SC.
#[derive(Debug, Default)]
pub struct Serial {
    data: u8,
    control: u8,
    remaining_bits: u8,
}

impl Serial {
    /// Creates an idle port.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Shifts one bit; called at 8192 Hz, in phase with the divider.
    ///
    /// # Returns
    /// Whether the serial interrupt is requested.
    pub fn clock(&mut self) -> bool {
        if self.remaining_bits == 0 {
            return false;
        }
        self.remaining_bits -= 1;
        self.data = (self.data << 1) | DISCONNECTED_BIT;
        if self.remaining_bits != 0 {
            return false;
        }
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
        self.remaining_bits = if val & START != 0 && val & INTERNAL_CLOCK != 0 {
            TRANSFER_BITS
        } else {
            0
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_internal_clock_receives_ones() {
        let mut serial = Serial::new();
        serial.write_data(0x42);
        serial.write_control(0x81);
        assert!((1..TRANSFER_BITS).all(|_| !serial.clock()));
        assert_eq!((serial.data(), serial.control()), (0x7F, 0xFF));
        assert!(serial.clock());
        assert_eq!((serial.data(), serial.control()), (0xFF, 0x7F));
        assert!(!serial.clock());
    }

    #[test]
    fn clock_external_clock_never_completes() {
        let mut serial = Serial::new();
        serial.write_control(0x80);
        assert!((0..TRANSFER_BITS * 2).all(|_| !serial.clock()));
        assert_eq!(serial.control(), 0xFE);
    }
}
