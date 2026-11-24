//! MBC5 and its rumble variant.

use super::{
    RamTarget, RAM_BANK_END, RAM_BANK_START, RAM_ENABLE_END, RAM_ENABLE_KEY, REGISTERS_START,
    ROM_BANK_END, ROM_BANK_START,
};

/// The ROM bank register is split in two.
const ROM_LOW_END: u16 = 0x2FFF;
const ROM_HIGH_START: u16 = 0x3000;
const RAM_MASK: u8 = 0x0F;
/// On rumble cartridges bit 3 drives the motor instead of selecting RAM.
const RUMBLE_RAM_MASK: u8 = 0x07;

#[derive(Debug)]
pub(in crate::cartridge) struct Mbc5 {
    ram_enabled: bool,
    rom_low: u8,
    rom_high: u8,
    ram_bank: u8,
    ram_mask: u8,
}

impl Mbc5 {
    pub(super) fn new(has_rumble: bool) -> Self {
        Self {
            ram_enabled: false,
            rom_low: 1,
            rom_high: 0,
            ram_bank: 0,
            ram_mask: if has_rumble {
                RUMBLE_RAM_MASK
            } else {
                RAM_MASK
            },
        }
    }

    pub(super) fn rom_bank(&self) -> usize {
        usize::from(u16::from_be_bytes([self.rom_high, self.rom_low]))
    }

    pub(super) fn ram_target(&self) -> RamTarget {
        if self.ram_enabled {
            RamTarget::Bank(usize::from(self.ram_bank))
        } else {
            RamTarget::Disabled
        }
    }

    pub(super) fn write(&mut self, addr: u16, val: u8) {
        match addr {
            // Unlike earlier controllers, all eight bits are compared.
            REGISTERS_START..=RAM_ENABLE_END => self.ram_enabled = val == RAM_ENABLE_KEY,
            ROM_BANK_START..=ROM_LOW_END => self.rom_low = val,
            ROM_HIGH_START..=ROM_BANK_END => self.rom_high = val & 1,
            RAM_BANK_START..=RAM_BANK_END => self.ram_bank = val & self.ram_mask,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rom_bank_joins_nine_bits() {
        for (low, high, expected) in [(0, 0, 0), (0xFF, 0, 0xFF), (0xFF, 0xFF, 0x1FF)] {
            let mut mbc = Mbc5::new(false);
            mbc.write(0x2000, low);
            mbc.write(0x3000, high);
            assert_eq!(mbc.rom_bank(), expected);
        }
    }

    #[test]
    fn ram_target_masks_rumble_motor() {
        for (has_rumble, expected) in [(false, 0x0B), (true, 0x03)] {
            let mut mbc = Mbc5::new(has_rumble);
            assert_eq!(mbc.ram_target(), RamTarget::Disabled);
            mbc.write(0x0000, 0x0A);
            mbc.write(0x4000, 0x0B);
            assert_eq!(mbc.ram_target(), RamTarget::Bank(expected));
        }
    }
}
