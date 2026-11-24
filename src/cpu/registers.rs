//! SM83 register file and operand decoding.

/// Zero flag.
pub const FLAG_Z: u8 = 0x80;
/// Subtract flag.
pub const FLAG_N: u8 = 0x40;
/// Half-carry flag.
pub const FLAG_H: u8 = 0x20;
/// Carry flag.
pub const FLAG_C: u8 = 0x10;
/// The low nibble of F does not exist.
const F_MASK: u8 = 0xF0;

/// Returns `flag` when `set`, otherwise no flag.
pub(super) const fn flag_if(flag: u8, set: bool) -> u8 {
    if set {
        flag
    } else {
        0
    }
}

/// 8-bit operand encoded in the low three bits of an opcode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum R8 {
    B,
    C,
    D,
    E,
    H,
    L,
    HlMem,
    A,
}

impl R8 {
    pub(super) const fn from_bits(bits: u8) -> Self {
        match bits & 7 {
            0 => Self::B,
            1 => Self::C,
            2 => Self::D,
            3 => Self::E,
            4 => Self::H,
            5 => Self::L,
            6 => Self::HlMem,
            _ => Self::A,
        }
    }
}

/// Register pair; slot 3 of the encoding is SP or AF depending on the opcode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum R16 {
    Bc,
    De,
    Hl,
    Sp,
    Af,
}

impl R16 {
    pub(super) const fn from_bits(bits: u8, slot_3: Self) -> Self {
        match bits & 3 {
            0 => Self::Bc,
            1 => Self::De,
            2 => Self::Hl,
            _ => slot_3,
        }
    }
}

/// SM83 register file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Registers {
    /// Accumulator.
    pub a: u8,
    /// Flags; the low nibble is always zero.
    pub(crate) f: u8,
    /// Register B.
    pub b: u8,
    /// Register C.
    pub c: u8,
    /// Register D.
    pub d: u8,
    /// Register E.
    pub e: u8,
    /// Register H.
    pub h: u8,
    /// Register L.
    pub l: u8,
    /// Stack pointer.
    pub sp: u16,
    /// Program counter.
    pub pc: u16,
}

impl Registers {
    /// State the DMG boot ROM leaves behind, so it need not be executed.
    pub(super) const POST_BOOT: Self = Self {
        a: 0x01,
        f: 0xB0,
        b: 0x00,
        c: 0x13,
        d: 0x00,
        e: 0xD8,
        h: 0x01,
        l: 0x4D,
        sp: 0xFFFE,
        pc: 0x0100,
    };

    /// Returns the flags register; its low nibble is always zero.
    #[must_use]
    pub fn flags(&self) -> u8 {
        self.f
    }

    pub(super) fn pair(&self, pair: R16) -> u16 {
        match pair {
            R16::Bc => u16::from_be_bytes([self.b, self.c]),
            R16::De => u16::from_be_bytes([self.d, self.e]),
            R16::Hl => u16::from_be_bytes([self.h, self.l]),
            R16::Af => u16::from_be_bytes([self.a, self.f]),
            R16::Sp => self.sp,
        }
    }

    pub(super) fn set_pair(&mut self, pair: R16, val: u16) {
        let [high, low] = val.to_be_bytes();
        match pair {
            R16::Bc => (self.b, self.c) = (high, low),
            R16::De => (self.d, self.e) = (high, low),
            R16::Hl => (self.h, self.l) = (high, low),
            R16::Af => (self.a, self.f) = (high, low & F_MASK),
            R16::Sp => self.sp = val,
        }
    }

    pub(super) fn has_flag(&self, flag: u8) -> bool {
        self.f & flag != 0
    }

    /// Evaluates the condition in bits 3-4 of an opcode: NZ, Z, NC, C.
    pub(super) fn condition(&self, bits: u8) -> bool {
        match bits & 3 {
            0 => !self.has_flag(FLAG_Z),
            1 => self.has_flag(FLAG_Z),
            2 => !self.has_flag(FLAG_C),
            _ => self.has_flag(FLAG_C),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_if_selects_flag() {
        assert_eq!((flag_if(FLAG_Z, true), flag_if(FLAG_Z, false)), (FLAG_Z, 0));
    }

    #[test]
    fn set_pair_round_trips() {
        let mut regs = Registers::default();
        for pair in [R16::Bc, R16::De, R16::Hl, R16::Sp] {
            regs.set_pair(pair, 0x12FF);
            assert_eq!(regs.pair(pair), 0x12FF);
        }
    }

    #[test]
    fn set_pair_af_drops_low_nibble() {
        let mut regs = Registers::default();
        regs.set_pair(R16::Af, 0x12FF);
        assert_eq!(regs.pair(R16::Af), 0x12F0);
    }

    #[test]
    fn from_bits_decodes_operands() {
        assert_eq!(R8::from_bits(0xFE), R8::HlMem);
        assert_eq!(R8::from_bits(0x07), R8::A);
        assert_eq!(R16::from_bits(2, R16::Sp), R16::Hl);
        assert_eq!(R16::from_bits(3, R16::Af), R16::Af);
    }

    #[test]
    fn condition_each_encoding_follows_flags() {
        for (f, expected) in [
            (0, [true, false, true, false]),
            (FLAG_Z, [false, true, true, false]),
            (FLAG_C, [true, false, false, true]),
        ] {
            let regs = Registers {
                f,
                ..Registers::default()
            };
            assert_eq!([0, 1, 2, 3].map(|bits| regs.condition(bits)), expected);
        }
    }
}
