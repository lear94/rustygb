//! MBC1 and its multicart wiring.

use super::{enables_ram, RamTarget};

const LOW_MASK: u8 = 0x1F;
const HIGH_MASK: u8 = 0x03;
const HIGH_SHIFT: u32 = 5;
/// Multicarts leave one line of the low register unconnected.
const MULTICART_HIGH_SHIFT: u32 = 4;

#[derive(Debug)]
pub(in crate::cartridge) struct Mbc1 {
    ram_enabled: bool,
    low: u8,
    high: u8,
    advanced: bool,
    high_shift: u32,
}

impl Mbc1 {
    pub(super) fn new(is_multicart: bool) -> Self {
        Self {
            ram_enabled: false,
            low: 1,
            high: 0,
            advanced: false,
            high_shift: if is_multicart {
                MULTICART_HIGH_SHIFT
            } else {
                HIGH_SHIFT
            },
        }
    }

    pub(super) fn fixed_rom_bank(&self) -> usize {
        if self.advanced {
            self.upper_bits()
        } else {
            0
        }
    }

    pub(super) fn switchable_rom_bank(&self) -> usize {
        let low_lines = (1 << self.high_shift) - 1;
        self.upper_bits() | (usize::from(self.low) & low_lines)
    }

    pub(super) fn ram_target(&self) -> RamTarget {
        match (self.ram_enabled, self.advanced) {
            (false, _) => RamTarget::Disabled,
            (true, false) => RamTarget::Bank(0),
            (true, true) => RamTarget::Bank(usize::from(self.high)),
        }
    }

    pub(super) fn write(&mut self, addr: u16, val: u8) {
        match addr {
            0x0000..=0x1FFF => self.ram_enabled = enables_ram(val),
            // Only the 5-bit register is checked for zero, so banks 0x20/0x40/0x60 are
            // unreachable here, as on hardware.
            0x2000..=0x3FFF => self.low = (val & LOW_MASK).max(1),
            0x4000..=0x5FFF => self.high = val & HIGH_MASK,
            _ => self.advanced = val & 1 != 0,
        }
    }

    fn upper_bits(&self) -> usize {
        usize::from(self.high) << self.high_shift
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn written(is_multicart: bool, writes: &[(u16, u8)]) -> Mbc1 {
        let mut mbc = Mbc1::new(is_multicart);
        for &(addr, val) in writes {
            mbc.write(addr, val);
        }
        mbc
    }

    #[test]
    fn rom_banks_combine_registers() {
        for (is_multicart, writes, fixed, switchable) in [
            (false, &[][..], 0, 1),
            (false, &[(0x2000, 0x20)], 0, 1),
            (false, &[(0x2000, 0), (0x4000, 1)], 0, 0x21),
            (false, &[(0x4000, 2), (0x6000, 1)], 0x40, 0x41),
            (true, &[(0x2000, 0x1F), (0x4000, 1)], 0, 0x1F),
            (
                true,
                &[(0x2000, 0x10), (0x4000, 2), (0x6000, 1)],
                0x20,
                0x20,
            ),
        ] {
            let mbc = written(is_multicart, writes);
            assert_eq!(
                (mbc.fixed_rom_bank(), mbc.switchable_rom_bank()),
                (fixed, switchable),
                "{writes:?}"
            );
        }
    }

    #[test]
    fn ram_target_follows_mode() {
        for (writes, expected) in [
            (&[][..], RamTarget::Disabled),
            (&[(0, 0x0A), (0x4000, 2)], RamTarget::Bank(0)),
            (&[(0, 0x0A), (0x4000, 2), (0x6000, 1)], RamTarget::Bank(2)),
        ] {
            assert_eq!(written(false, writes).ram_target(), expected);
        }
    }
}
