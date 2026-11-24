//! Memory bank controllers: register writes in, bank numbers out.

const RAM_ENABLE_MASK: u8 = 0x0F;
const RAM_ENABLE_KEY: u8 = 0x0A;
const MBC1_LOW_MASK: u8 = 0x1F;
const MBC1_HIGH_MASK: u8 = 0x03;
const MBC1_HIGH_SHIFT: u32 = 5;
const MBC3_ROM_MASK: u8 = 0x7F;
const MBC3_LAST_RAM_BANK: u8 = 0x03;
const MBC5_RAM_MASK: u8 = 0x0F;
/// First address of the switchable ROM window.
const SWITCHABLE_ROM: u16 = 0x4000;

pub(super) enum Mapper {
    RomOnly,
    Mbc1 {
        ram_enabled: bool,
        low: u8,
        high: u8,
        advanced: bool,
    },
    Mbc3 {
        ram_enabled: bool,
        rom_bank: u8,
        ram_select: u8,
    },
    Mbc5 {
        ram_enabled: bool,
        rom_low: u8,
        rom_high: u8,
        ram_bank: u8,
    },
}

impl Mapper {
    pub(super) fn from_code(code: u8) -> Option<Self> {
        match code {
            0x00 => Some(Self::RomOnly),
            0x01..=0x03 => Some(Self::Mbc1 {
                ram_enabled: false,
                low: 1,
                high: 0,
                advanced: false,
            }),
            0x0F..=0x13 => Some(Self::Mbc3 {
                ram_enabled: false,
                rom_bank: 1,
                ram_select: 0,
            }),
            0x19..=0x1E => Some(Self::Mbc5 {
                ram_enabled: false,
                rom_low: 1,
                rom_high: 0,
                ram_bank: 0,
            }),
            _ => None,
        }
    }

    pub(super) fn rom_bank(&self, addr: u16) -> usize {
        let switchable = addr >= SWITCHABLE_ROM;
        match *self {
            Self::RomOnly => usize::from(switchable),
            Self::Mbc1 {
                low,
                high,
                advanced,
                ..
            } => {
                let upper = usize::from(high) << MBC1_HIGH_SHIFT;
                match (switchable, advanced) {
                    (true, _) => upper | usize::from(low),
                    (false, true) => upper,
                    (false, false) => 0,
                }
            }
            Self::Mbc3 { rom_bank, .. } if switchable => usize::from(rom_bank),
            Self::Mbc5 {
                rom_low, rom_high, ..
            } if switchable => usize::from(u16::from_be_bytes([rom_high, rom_low])),
            Self::Mbc3 { .. } | Self::Mbc5 { .. } => 0,
        }
    }

    /// Mapped RAM bank; `None` while RAM is disabled or an RTC register is selected.
    pub(super) fn ram_bank(&self) -> Option<usize> {
        match *self {
            Self::Mbc1 {
                ram_enabled: true,
                high,
                advanced,
                ..
            } => Some(if advanced { usize::from(high) } else { 0 }),
            Self::Mbc3 {
                ram_enabled: true,
                ram_select: select @ 0..=MBC3_LAST_RAM_BANK,
                ..
            } => Some(usize::from(select)),
            Self::Mbc5 {
                ram_enabled: true,
                ram_bank,
                ..
            } => Some(usize::from(ram_bank)),
            _ => None,
        }
    }

    pub(super) fn selects_rtc(&self) -> bool {
        matches!(
            self,
            Self::Mbc3 {
                ram_enabled: true,
                ram_select: 0x08..=0x0C,
                ..
            }
        )
    }

    pub(super) fn write_register(&mut self, addr: u16, val: u8) {
        match self {
            Self::RomOnly => {}
            Self::Mbc1 {
                ram_enabled,
                low,
                high,
                advanced,
            } => match addr {
                0x0000..=0x1FFF => *ram_enabled = enables_ram(val),
                // Only the 5-bit register is checked for zero, so banks 0x20/0x40/0x60
                // are unreachable here, as on hardware.
                0x2000..=0x3FFF => *low = (val & MBC1_LOW_MASK).max(1),
                0x4000..=0x5FFF => *high = val & MBC1_HIGH_MASK,
                _ => *advanced = val & 1 != 0,
            },
            Self::Mbc3 {
                ram_enabled,
                rom_bank,
                ram_select,
            } => match addr {
                0x0000..=0x1FFF => *ram_enabled = enables_ram(val),
                0x2000..=0x3FFF => *rom_bank = (val & MBC3_ROM_MASK).max(1),
                0x4000..=0x5FFF if matches!(val, 0..=MBC3_LAST_RAM_BANK | 0x08..=0x0C) => {
                    *ram_select = val;
                }
                _ => {}
            },
            Self::Mbc5 {
                ram_enabled,
                rom_low,
                rom_high,
                ram_bank,
            } => match addr {
                0x0000..=0x1FFF => *ram_enabled = enables_ram(val),
                0x2000..=0x2FFF => *rom_low = val,
                0x3000..=0x3FFF => *rom_high = val & 1,
                0x4000..=0x5FFF => *ram_bank = val & MBC5_RAM_MASK,
                _ => {}
            },
        }
    }
}

fn enables_ram(val: u8) -> bool {
    val & RAM_ENABLE_MASK == RAM_ENABLE_KEY
}

#[cfg(test)]
mod tests {
    use super::*;

    const MBC1: u8 = 0x03;
    const MBC3: u8 = 0x13;
    const MBC5: u8 = 0x1B;

    fn mapper(code: u8, writes: &[(u16, u8)]) -> Mapper {
        let mut mapper = Mapper::from_code(code).expect("supported mapper");
        for &(addr, val) in writes {
            mapper.write_register(addr, val);
        }
        mapper
    }

    #[test]
    fn from_code_unknown_is_none() {
        for code in [0x04, 0x06, 0x14, 0x1F, 0xFF] {
            assert!(Mapper::from_code(code).is_none());
        }
    }

    #[test]
    fn rom_bank_rom_only_is_fixed() {
        let mapper = mapper(0, &[(0x2000, 7)]);
        assert_eq!((mapper.rom_bank(0), mapper.rom_bank(0x4000)), (0, 1));
    }

    #[test]
    fn rom_bank_combines_registers() {
        for (code, writes, low, high) in [
            (MBC1, &[(0x2000, 0x20)][..], 0, 1),
            (MBC1, &[(0x2000, 0), (0x4000, 1)], 0, 0x21),
            (MBC1, &[(0x4000, 2), (0x6000, 1)], 0x40, 0x41),
            (MBC3, &[(0x2000, 0)], 0, 1),
            (MBC3, &[(0x2000, 0xFF)], 0, 0x7F),
            (MBC5, &[(0x2000, 0)], 0, 0),
            (MBC5, &[(0x2000, 0xFF), (0x3000, 0xFF)], 0, 0x1FF),
        ] {
            let mapper = mapper(code, writes);
            assert_eq!(
                (mapper.rom_bank(0), mapper.rom_bank(0x4000)),
                (low, high),
                "{code:#x} {writes:?}"
            );
        }
    }

    #[test]
    fn ram_bank_requires_enable() {
        for (code, writes, expected) in [
            (0, &[(0, 0x0A)][..], None),
            (MBC1, &[], None),
            (MBC1, &[(0, 0x0A), (0x4000, 2)], Some(0)),
            (MBC1, &[(0, 0x0A), (0x4000, 2), (0x6000, 1)], Some(2)),
            (MBC3, &[(0, 0x1A), (0x4000, 3)], Some(3)),
            (MBC3, &[(0, 0x0A), (0x4000, 3), (0x4000, 7)], Some(3)),
            (MBC3, &[(0, 0x0A), (0x4000, 8)], None),
            (MBC5, &[(0, 0x0A), (0x4000, 2)], Some(2)),
            (MBC5, &[(0, 0x0A), (0, 0)], None),
        ] {
            assert_eq!(mapper(code, writes).ram_bank(), expected, "{code:#x}");
        }
    }

    #[test]
    fn selects_rtc_requires_enable_and_selection() {
        for (writes, expected) in [
            (&[(0x4000, 8)][..], false),
            (&[(0, 0x0A), (0x4000, 8)], true),
            (&[(0, 0x0A), (0x4000, 0x0C)], true),
            (&[(0, 0x0A), (0x4000, 0)], false),
        ] {
            assert_eq!(mapper(MBC3, writes).selects_rtc(), expected);
        }
    }
}
