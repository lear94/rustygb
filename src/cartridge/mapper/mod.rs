//! Memory bank controllers: register writes in, bank numbers out.

mod mbc1;
mod mbc2;
mod mbc3;
mod mbc5;

use mbc1::Mbc1;
use mbc2::Mbc2;
use mbc3::Mbc3;
use mbc5::Mbc5;

const RAM_ENABLE_MASK: u8 = 0x0F;
const RAM_ENABLE_KEY: u8 = 0x0A;

/// Device answering at `0xA000-0xBFFF`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RamTarget {
    Disabled,
    Bank(usize),
    Rtc(u8),
}

/// Controller fitted to the cartridge.
#[derive(Debug)]
pub(super) enum Mapper {
    RomOnly,
    Mbc1(Mbc1),
    Mbc2(Mbc2),
    Mbc3(Mbc3),
    Mbc5(Mbc5),
}

impl Mapper {
    /// Builds the controller named by header byte `0x147`.
    ///
    /// # Arguments
    /// * `is_multicart` - selects the MBC1 wiring of game collections.
    pub(super) fn from_code(code: u8, is_multicart: bool) -> Option<Self> {
        match code {
            0x00 | 0x08 | 0x09 => Some(Self::RomOnly),
            0x01..=0x03 => Some(Self::Mbc1(Mbc1::new(is_multicart))),
            0x05 | 0x06 => Some(Self::Mbc2(Mbc2::new())),
            0x0F..=0x13 => Some(Self::Mbc3(Mbc3::new())),
            0x19..=0x1B => Some(Self::Mbc5(Mbc5::new(false))),
            0x1C..=0x1E => Some(Self::Mbc5(Mbc5::new(true))),
            _ => None,
        }
    }

    /// ROM bank at `0x0000-0x3FFF`.
    pub(super) fn fixed_rom_bank(&self) -> usize {
        match self {
            Self::Mbc1(mbc) => mbc.fixed_rom_bank(),
            _ => 0,
        }
    }

    /// ROM bank at `0x4000-0x7FFF`.
    pub(super) fn switchable_rom_bank(&self) -> usize {
        match self {
            Self::RomOnly => 1,
            Self::Mbc1(mbc) => mbc.switchable_rom_bank(),
            Self::Mbc2(mbc) => mbc.rom_bank(),
            Self::Mbc3(mbc) => mbc.rom_bank(),
            Self::Mbc5(mbc) => mbc.rom_bank(),
        }
    }

    pub(super) fn ram_target(&self) -> RamTarget {
        match self {
            Self::RomOnly => RamTarget::Bank(0),
            Self::Mbc1(mbc) => mbc.ram_target(),
            Self::Mbc2(mbc) => mbc.ram_target(),
            Self::Mbc3(mbc) => mbc.ram_target(),
            Self::Mbc5(mbc) => mbc.ram_target(),
        }
    }

    /// RAM bits that do not exist and read as 1.
    pub(super) fn ram_unused_bits(&self) -> u8 {
        match self {
            Self::Mbc2(_) => mbc2::RAM_UNUSED_BITS,
            _ => 0,
        }
    }

    pub(super) fn write_register(&mut self, addr: u16, val: u8) {
        match self {
            Self::RomOnly => {}
            Self::Mbc1(mbc) => mbc.write(addr, val),
            Self::Mbc2(mbc) => mbc.write(addr, val),
            Self::Mbc3(mbc) => mbc.write(addr, val),
            Self::Mbc5(mbc) => mbc.write(addr, val),
        }
    }
}

fn enables_ram(val: u8) -> bool {
    val & RAM_ENABLE_MASK == RAM_ENABLE_KEY
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_code_unknown_is_none() {
        for code in [0x04, 0x07, 0x0B, 0x14, 0x1F, 0xFF] {
            assert!(Mapper::from_code(code, false).is_none());
        }
    }

    #[test]
    fn from_code_rom_only_is_fixed() {
        let mut mapper = Mapper::from_code(0x08, false).expect("ROM+RAM");
        mapper.write_register(0x2000, 7);
        assert_eq!(
            (mapper.fixed_rom_bank(), mapper.switchable_rom_bank()),
            (0, 1)
        );
        assert_eq!(mapper.ram_target(), RamTarget::Bank(0));
    }

    #[test]
    fn enables_ram_checks_low_nibble() {
        for (val, expected) in [(0x0A, true), (0xFA, true), (0x00, false), (0x0B, false)] {
            assert_eq!(enables_ram(val), expected);
        }
    }
}
