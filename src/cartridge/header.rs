//! Cartridge header decoding (`0x0100-0x014F`).

use super::mapper::Mapper;
use super::CartridgeError;

const HEADER_END: usize = 0x150;
const MAPPER_ADDR: usize = 0x147;
const RAM_SIZE_ADDR: usize = 0x149;

pub(super) struct Header {
    pub(super) mapper: Mapper,
    pub(super) has_battery: bool,
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
        let mapper =
            Mapper::from_code(mapper_code).ok_or(CartridgeError::UnsupportedMapper(mapper_code))?;
        Ok(Self {
            mapper,
            has_battery: has_battery(mapper_code),
            ram_len: ram_len(ram_code).ok_or(CartridgeError::UnsupportedRamSize(ram_code))?,
        })
    }
}

fn has_battery(mapper_code: u8) -> bool {
    matches!(mapper_code, 0x03 | 0x0F | 0x10 | 0x13 | 0x1B | 0x1E)
}

fn ram_len(code: u8) -> Option<usize> {
    match code {
        0x00 => Some(0),
        0x01 => Some(0x800),
        0x02 => Some(0x2000),
        0x03 => Some(0x8000),
        0x04 => Some(0x2_0000),
        0x05 => Some(0x1_0000),
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
    fn parse_battery_follows_mapper_code() {
        for (code, expected) in [(0x00, false), (0x02, false), (0x03, true), (0x1E, true)] {
            let header = Header::parse(&image(code, 2)).expect("supported codes");
            assert_eq!(header.has_battery, expected);
        }
    }

    #[test]
    fn ram_len_maps_every_code() {
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
