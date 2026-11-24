//! MBC3, including the larger MBC30 address space.
//!
//! Bank numbers keep every MBC30 line; the cartridge masks them to its real sizes, which
//! reproduces the narrower MBC3.

use super::{enables_ram, RamTarget};

const LAST_RAM_BANK: u8 = 0x07;

#[derive(Debug)]
pub(in crate::cartridge) struct Mbc3 {
    ram_enabled: bool,
    rom_bank: u8,
    ram_select: u8,
}

impl Mbc3 {
    pub(super) fn new() -> Self {
        Self {
            ram_enabled: false,
            rom_bank: 1,
            ram_select: 0,
        }
    }

    pub(super) fn rom_bank(&self) -> usize {
        usize::from(self.rom_bank)
    }

    pub(super) fn ram_target(&self) -> RamTarget {
        match (self.ram_enabled, self.ram_select) {
            (true, bank @ 0..=LAST_RAM_BANK) => RamTarget::Bank(usize::from(bank)),
            (true, register @ 0x08..=0x0C) => RamTarget::Rtc(register),
            _ => RamTarget::Disabled,
        }
    }

    pub(super) fn write(&mut self, addr: u16, val: u8) {
        match addr {
            0x0000..=0x1FFF => self.ram_enabled = enables_ram(val),
            0x2000..=0x3FFF => self.rom_bank = val.max(1),
            0x4000..=0x5FFF => self.ram_select = val & 0x0F,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rom_bank_zero_selects_one() {
        for (val, expected) in [(0, 1), (1, 1), (0x7F, 0x7F), (0xFF, 0xFF)] {
            let mut mbc = Mbc3::new();
            mbc.write(0x2000, val);
            assert_eq!(mbc.rom_bank(), expected);
        }
    }

    #[test]
    fn ram_target_selects_bank_or_clock() {
        for (enable, select, expected) in [
            (0x00, 0, RamTarget::Disabled),
            (0x0A, 0, RamTarget::Bank(0)),
            (0x0A, 7, RamTarget::Bank(7)),
            (0x0A, 8, RamTarget::Rtc(8)),
            (0x0A, 0x0C, RamTarget::Rtc(0x0C)),
            (0x0A, 0x0D, RamTarget::Disabled),
        ] {
            let mut mbc = Mbc3::new();
            mbc.write(0x0000, enable);
            mbc.write(0x4000, select);
            assert_eq!(mbc.ram_target(), expected);
        }
    }
}
