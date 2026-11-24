//! MBC3, including the larger MBC30 address space.
//!
//! RAM banks keep every MBC30 line; the cartridge masks them to its real size. The ROM
//! register width differs, because it decides which writes select bank 0.

use super::{
    enables_ram, RamTarget, RAM_BANK_END, RAM_BANK_START, RAM_ENABLE_END, REGISTERS_START,
    ROM_BANK_END, ROM_BANK_START,
};

const ROM_MASK: u8 = 0x7F;
const MBC30_ROM_MASK: u8 = 0xFF;
const RAM_SELECT_MASK: u8 = 0x0F;
const FIRST_RTC_REGISTER: u8 = 0x08;
const LAST_RTC_REGISTER: u8 = 0x0C;
const LAST_RAM_BANK: u8 = 0x07;

#[derive(Debug)]
pub(in crate::cartridge) struct Mbc3 {
    ram_enabled: bool,
    rom_bank: u8,
    ram_select: u8,
    rom_mask: u8,
}

impl Mbc3 {
    pub(super) fn new(is_mbc30: bool) -> Self {
        Self {
            ram_enabled: false,
            rom_bank: 1,
            ram_select: 0,
            rom_mask: if is_mbc30 { MBC30_ROM_MASK } else { ROM_MASK },
        }
    }

    pub(super) fn rom_bank(&self) -> usize {
        usize::from(self.rom_bank)
    }

    pub(super) fn ram_target(&self) -> RamTarget {
        match (self.ram_enabled, self.ram_select) {
            (true, bank @ 0..=LAST_RAM_BANK) => RamTarget::Bank(usize::from(bank)),
            (true, register @ FIRST_RTC_REGISTER..=LAST_RTC_REGISTER) => RamTarget::Rtc(register),
            _ => RamTarget::Disabled,
        }
    }

    pub(super) fn write(&mut self, addr: u16, val: u8) {
        match addr {
            REGISTERS_START..=RAM_ENABLE_END => self.ram_enabled = enables_ram(val),
            ROM_BANK_START..=ROM_BANK_END => self.rom_bank = (val & self.rom_mask).max(1),
            RAM_BANK_START..=RAM_BANK_END => self.ram_select = val & RAM_SELECT_MASK,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rom_bank_zero_selects_one() {
        for (is_mbc30, val, expected) in [
            (false, 0, 1),
            (false, 0x7F, 0x7F),
            (false, 0x80, 1),
            (true, 0x80, 0x80),
            (true, 0xFF, 0xFF),
        ] {
            let mut mbc = Mbc3::new(is_mbc30);
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
            let mut mbc = Mbc3::new(false);
            mbc.write(0x0000, enable);
            mbc.write(0x4000, select);
            assert_eq!(mbc.ram_target(), expected);
        }
    }
}
