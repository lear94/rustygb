//! MBC2: 16 ROM banks and 512 built-in half-bytes of RAM.

use super::{enables_ram, RamTarget, REGISTERS_START, ROM_BANK_END};

const ROM_MASK: u8 = 0x0F;
/// Address line choosing the ROM bank register over RAM enable.
const ROM_REGISTER_LINE: u16 = 0x0100;
pub(super) const RAM_UNUSED_BITS: u8 = 0xF0;

#[derive(Debug)]
pub(in crate::cartridge) struct Mbc2 {
    ram_enabled: bool,
    rom_bank: u8,
}

impl Mbc2 {
    pub(super) fn new() -> Self {
        Self {
            ram_enabled: false,
            rom_bank: 1,
        }
    }

    pub(super) fn rom_bank(&self) -> usize {
        usize::from(self.rom_bank)
    }

    pub(super) fn ram_target(&self) -> RamTarget {
        if self.ram_enabled {
            RamTarget::Bank(0)
        } else {
            RamTarget::Disabled
        }
    }

    pub(super) fn write(&mut self, addr: u16, val: u8) {
        match addr {
            REGISTERS_START..=ROM_BANK_END if addr & ROM_REGISTER_LINE != 0 => {
                self.rom_bank = (val & ROM_MASK).max(1);
            }
            REGISTERS_START..=ROM_BANK_END => self.ram_enabled = enables_ram(val),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_address_line_selects_register() {
        let mut mbc = Mbc2::new();
        mbc.write(0x3F00, 0x15);
        assert_eq!((mbc.rom_bank(), mbc.ram_target()), (5, RamTarget::Disabled));
        mbc.write(0x3E00, 0x0A);
        assert_eq!((mbc.rom_bank(), mbc.ram_target()), (5, RamTarget::Bank(0)));
        mbc.write(0x0100, 0x10);
        assert_eq!(mbc.rom_bank(), 1);
        mbc.write(0x4100, 7);
        assert_eq!(mbc.rom_bank(), 1);
    }
}
