//! Reasons a cartridge image is rejected.

use std::error::Error;
use std::fmt;

/// Error building a cartridge.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CartridgeError {
    /// The image is too short to hold a header.
    RomTooSmall {
        /// Image length in bytes.
        len: usize,
        /// Minimum accepted length in bytes.
        min: usize,
    },
    /// The header names a controller that is not emulated.
    UnsupportedMapper(
        /// Header byte `0x147`.
        u8,
    ),
    /// The header names an unknown RAM size.
    UnsupportedRamSize(
        /// Header byte `0x149`.
        u8,
    ),
    /// The save image does not match the cartridge RAM.
    SaveSizeMismatch {
        /// Cartridge RAM length in bytes.
        expected: usize,
        /// Save image length in bytes.
        found: usize,
    },
}

impl fmt::Display for CartridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RomTooSmall { len, min } => {
                write!(f, "ROM has {len} bytes, at least {min} required")
            }
            Self::UnsupportedMapper(code) => write!(f, "unsupported mapper {code:#04X}"),
            Self::UnsupportedRamSize(code) => write!(f, "unsupported RAM size {code:#04X}"),
            Self::SaveSizeMismatch { expected, found } => {
                write!(f, "save has {found} bytes, cartridge RAM has {expected}")
            }
        }
    }
}

impl Error for CartridgeError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_carries_the_offending_values() {
        let cases = [
            (
                CartridgeError::RomTooSmall { len: 1, min: 2 },
                "ROM has 1 bytes, at least 2 required",
            ),
            (
                CartridgeError::UnsupportedMapper(0xFE),
                "unsupported mapper 0xFE",
            ),
            (
                CartridgeError::UnsupportedRamSize(9),
                "unsupported RAM size 0x09",
            ),
            (
                CartridgeError::SaveSizeMismatch {
                    expected: 3,
                    found: 4,
                },
                "save has 4 bytes, cartridge RAM has 3",
            ),
        ];
        for (error, text) in cases {
            assert_eq!(error.to_string(), text);
        }
    }
}
