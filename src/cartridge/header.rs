//! Cartridge header decoding (`0x0100-0x014F`).

use super::mapper::Mapper;
use super::CartridgeError;

const HEADER_END: usize = 0x150;
const MAPPER_ADDR: usize = 0x147;
const RAM_SIZE_ADDR: usize = 0x149;
const MBC2_CODES: [u8; 2] = [0x05, 0x06];
const BATTERY_CODES: [u8; 8] = [0x03, 0x06, 0x09, 0x0F, 0x10, 0x13, 0x1B, 0x1E];
const RTC_CODES: [u8; 2] = [0x0F, 0x10];
/// Half-bytes built into the MBC2.
const MBC2_RAM_LEN: usize = 512;
const KIB: usize = 1024;

const MBC3_MAX_ROM_LEN: usize = 2048 * KIB;
const MBC30_RAM_CODE: u8 = 5;

/// Multicarts are 1 MiB collections whose games start every 16 banks.
const MULTICART_LEN: usize = 1024 * KIB;
const MULTICART_GAME_OFFSET: usize = 0x4_0000;
/// The boot logo proves a second game starts there.
const LOGO_START: usize = 0x104;
const LOGO_END: usize = 0x134;

/// Cartridge hardware described by the header.
pub(super) struct Header {
    pub(super) mapper: Mapper,
    pub(super) has_battery: bool,
    pub(super) has_rtc: bool,
    pub(super) ram_len: usize,
}

impl Header {
    pub(super) fn parse(rom: &[u8]) -> Result<Self, CartridgeError> {
        let too_small = CartridgeError::RomTooSmall {
            len: rom.len(),
            min: HEADER_END,
        };
        let Some(header) = rom.get(..HEADER_END) else {
            return Err(too_small);
        };
        let (Some(&mapper_code), Some(&ram_code)) =
            (header.get(MAPPER_ADDR), header.get(RAM_SIZE_ADDR))
        else {
            return Err(too_small);
        };
        let mapper = Mapper::from_code(
            mapper_code,
            is_multicart(rom),
            is_mbc30(rom.len(), ram_code),
        )
        .ok_or(CartridgeError::UnsupportedMapper(mapper_code))?;
        let ram_len = if MBC2_CODES.contains(&mapper_code) {
            MBC2_RAM_LEN
        } else {
            ram_len(ram_code).ok_or(CartridgeError::UnsupportedRamSize(ram_code))?
        };
        Ok(Self {
            mapper,
            has_battery: BATTERY_CODES.contains(&mapper_code),
            has_rtc: RTC_CODES.contains(&mapper_code),
            ram_len,
        })
    }
}

/// Only the MBC30 reaches beyond 2 MiB of ROM or 32 KiB of RAM.
fn is_mbc30(rom_len: usize, ram_code: u8) -> bool {
    rom_len > MBC3_MAX_ROM_LEN || ram_code == MBC30_RAM_CODE
}

fn is_multicart(rom: &[u8]) -> bool {
    let logo = rom.get(LOGO_START..LOGO_END);
    let second = rom.get(MULTICART_GAME_OFFSET + LOGO_START..MULTICART_GAME_OFFSET + LOGO_END);
    rom.len() == MULTICART_LEN
        && logo == second
        && logo.is_some_and(|logo| logo.iter().any(|&byte| byte != 0))
}

fn ram_len(code: u8) -> Option<usize> {
    match code {
        0 => Some(0),
        1 => Some(2 * KIB),
        2 => Some(8 * KIB),
        3 => Some(32 * KIB),
        4 => Some(128 * KIB),
        5 => Some(64 * KIB),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(mapper: u8, ram: u8) -> Vec<u8> {
        let mut rom = vec![0; HEADER_END];
        rom[MAPPER_ADDR] = mapper;
        rom[RAM_SIZE_ADDR] = ram;
        rom
    }

    #[test]
    fn parse_short_image_is_rejected() {
        for len in [0, HEADER_END - 1] {
            assert_eq!(
                Header::parse(&vec![0; len]).err(),
                Some(CartridgeError::RomTooSmall {
                    len,
                    min: HEADER_END
                })
            );
        }
    }

    #[test]
    fn parse_unknown_codes_are_rejected() {
        assert_eq!(
            Header::parse(&image(0xFF, 0)).err(),
            Some(CartridgeError::UnsupportedMapper(0xFF))
        );
        assert_eq!(
            Header::parse(&image(0x03, 6)).err(),
            Some(CartridgeError::UnsupportedRamSize(6))
        );
    }

    #[test]
    fn parse_features_follow_mapper_code() {
        for (code, has_battery, has_rtc, ram_len) in [
            (0x00, false, false, 8 * KIB),
            (0x03, true, false, 8 * KIB),
            (0x06, true, false, MBC2_RAM_LEN),
            (0x09, true, false, 8 * KIB),
            (0x0F, true, true, 8 * KIB),
            (0x10, true, true, 8 * KIB),
            (0x12, false, false, 8 * KIB),
            (0x1E, true, false, 8 * KIB),
        ] {
            let header = Header::parse(&image(code, 2)).expect("supported codes");
            assert_eq!(
                (header.has_battery, header.has_rtc, header.ram_len),
                (has_battery, has_rtc, ram_len),
                "{code:#x}"
            );
        }
    }

    #[test]
    fn is_multicart_requires_size_and_second_logo() {
        let mut rom = vec![0; MULTICART_LEN];
        assert!(!is_multicart(&rom), "blank logos prove nothing");
        rom[LOGO_START] = 1;
        assert!(!is_multicart(&rom));
        rom[MULTICART_GAME_OFFSET + LOGO_START] = 1;
        assert!(is_multicart(&rom));
        rom.push(0);
        assert!(!is_multicart(&rom));
    }

    #[test]
    fn is_mbc30_exceeds_either_mbc3_size() {
        for (rom_len, ram_code, expected) in [
            (MBC3_MAX_ROM_LEN, 3, false),
            (MBC3_MAX_ROM_LEN + 1, 3, true),
            (0, MBC30_RAM_CODE, true),
        ] {
            assert_eq!(is_mbc30(rom_len, ram_code), expected);
        }
    }

    #[test]
    fn ram_len_maps_known_codes() {
        for (code, len) in [
            (0, Some(0)),
            (1, Some(0x800)),
            (2, Some(0x2000)),
            (3, Some(0x8000)),
            (4, Some(0x2_0000)),
            (5, Some(0x1_0000)),
            (6, None),
        ] {
            assert_eq!(ram_len(code), len);
        }
    }
}
