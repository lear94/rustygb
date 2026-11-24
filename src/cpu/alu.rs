//! Arithmetic and logic on the register file, flags included.

use super::registers::{flag_if, Registers, FLAG_C, FLAG_H, FLAG_N, FLAG_Z, R16};

const NIBBLE: u8 = 0x0F;
const HL_HALF_MASK: u16 = 0x0FFF;
const BCD_LOW: u8 = 0x06;
const BCD_HIGH: u8 = 0x60;
const SIGN_BIT: u8 = 0x80;

impl Registers {
    /// Executes ADD/ADC/SUB/SBC/AND/XOR/OR/CP, numbered as in opcode bits 3-5.
    pub(super) fn alu(&mut self, operation: u8, val: u8) {
        let carry = u8::from(self.has_flag(FLAG_C));
        match operation & 7 {
            0 => self.add(val, 0),
            1 => self.add(val, carry),
            2 => self.a = self.subtract(val, 0),
            3 => self.a = self.subtract(val, carry),
            4 => self.logic(self.a & val, FLAG_H),
            5 => self.logic(self.a ^ val, 0),
            6 => self.logic(self.a | val, 0),
            _ => {
                self.subtract(val, 0);
            }
        }
    }

    fn add(&mut self, val: u8, carry: u8) {
        let (partial, carry_1) = self.a.overflowing_add(val);
        let (result, carry_2) = partial.overflowing_add(carry);
        self.f = flag_if(FLAG_Z, result == 0)
            | flag_if(FLAG_H, (self.a & NIBBLE) + (val & NIBBLE) + carry > NIBBLE)
            | flag_if(FLAG_C, carry_1 || carry_2);
        self.a = result;
    }

    /// Sets the flags of `A - val - carry` and returns it; A is the caller's to update.
    fn subtract(&mut self, val: u8, carry: u8) -> u8 {
        let (partial, borrow_1) = self.a.overflowing_sub(val);
        let (result, borrow_2) = partial.overflowing_sub(carry);
        self.f = FLAG_N
            | flag_if(FLAG_Z, result == 0)
            | flag_if(FLAG_H, self.a & NIBBLE < (val & NIBBLE) + carry)
            | flag_if(FLAG_C, borrow_1 || borrow_2);
        result
    }

    fn logic(&mut self, result: u8, half: u8) {
        self.a = result;
        self.f = half | flag_if(FLAG_Z, result == 0);
    }

    pub(super) fn increment(&mut self, val: u8) -> u8 {
        let result = val.wrapping_add(1);
        self.f = (self.f & FLAG_C)
            | flag_if(FLAG_Z, result == 0)
            | flag_if(FLAG_H, result & NIBBLE == 0);
        result
    }

    pub(super) fn decrement(&mut self, val: u8) -> u8 {
        let result = val.wrapping_sub(1);
        self.f = (self.f & FLAG_C)
            | FLAG_N
            | flag_if(FLAG_Z, result == 0)
            | flag_if(FLAG_H, val & NIBBLE == 0);
        result
    }

    pub(super) fn add_hl(&mut self, val: u16) {
        let hl = self.pair(R16::Hl);
        let (result, carry) = hl.overflowing_add(val);
        self.f = (self.f & FLAG_Z)
            | flag_if(
                FLAG_H,
                (hl & HL_HALF_MASK) + (val & HL_HALF_MASK) > HL_HALF_MASK,
            )
            | flag_if(FLAG_C, carry);
        self.set_pair(R16::Hl, result);
    }

    /// Returns SP plus a signed offset; H and C come from the unsigned low byte.
    pub(super) fn sp_plus(&mut self, offset: u8) -> u16 {
        let [_, low] = self.sp.to_be_bytes();
        self.f = flag_if(FLAG_H, (low & NIBBLE) + (offset & NIBBLE) > NIBBLE)
            | flag_if(FLAG_C, low.overflowing_add(offset).1);
        self.sp.wrapping_add_signed(sign_extend(offset))
    }

    /// Executes RLCA/RRCA/RLA/RRA/DAA/CPL/SCF/CCF, numbered as in opcode bits 3-5.
    pub(super) fn accumulator_op(&mut self, operation: u8) {
        match operation & 7 {
            rotation @ 0..=3 => {
                self.a = self.shift(rotation, self.a);
                // Unlike their CB-prefixed forms, these always clear Z.
                self.f &= FLAG_C;
            }
            4 => self.decimal_adjust(),
            5 => {
                self.a = !self.a;
                self.f |= FLAG_N | FLAG_H;
            }
            6 => self.f = (self.f & FLAG_Z) | FLAG_C,
            _ => self.f = (self.f & FLAG_Z) | flag_if(FLAG_C, !self.has_flag(FLAG_C)),
        }
    }

    /// Executes RLC/RRC/RL/RR/SLA/SRA/SWAP/SRL, numbered as in opcode bits 3-5.
    pub(super) fn shift(&mut self, operation: u8, val: u8) -> u8 {
        let carry_in = u8::from(self.has_flag(FLAG_C));
        let (high, low) = (val & SIGN_BIT != 0, val & 1 != 0);
        let (result, carry) = match operation & 7 {
            0 => (val.rotate_left(1), high),
            1 => (val.rotate_right(1), low),
            2 => ((val << 1) | carry_in, high),
            3 => ((val >> 1) | carry_in.rotate_right(1), low),
            4 => (val << 1, high),
            5 => ((val >> 1) | (val & SIGN_BIT), low),
            6 => (val.rotate_right(4), false),
            _ => (val >> 1, low),
        };
        self.f = flag_if(FLAG_Z, result == 0) | flag_if(FLAG_C, carry);
        result
    }

    pub(super) fn test_bit(&mut self, bit: u8, val: u8) {
        let clear = val & (1 << (bit & 7)) == 0;
        self.f = (self.f & FLAG_C) | FLAG_H | flag_if(FLAG_Z, clear);
    }

    fn decimal_adjust(&mut self) {
        let subtracting = self.has_flag(FLAG_N);
        let low = self.has_flag(FLAG_H) || (!subtracting && self.a & NIBBLE > 9);
        let carry = self.has_flag(FLAG_C) || (!subtracting && self.a > 0x99);
        let adjustment = flag_if(BCD_LOW, low) | flag_if(BCD_HIGH, carry);
        self.a = if subtracting {
            self.a.wrapping_sub(adjustment)
        } else {
            self.a.wrapping_add(adjustment)
        };
        self.f = (self.f & FLAG_N) | flag_if(FLAG_Z, self.a == 0) | flag_if(FLAG_C, carry);
    }
}

/// Widens a two's-complement byte.
pub(super) fn sign_extend(byte: u8) -> i16 {
    i16::from(i8::from_ne_bytes([byte]))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADD: u8 = 0;
    const ADC: u8 = 1;
    const SUB: u8 = 2;
    const SBC: u8 = 3;
    const CP: u8 = 7;
    const DAA: u8 = 4;

    fn regs(a: u8, f: u8) -> Registers {
        Registers {
            a,
            f,
            ..Registers::default()
        }
    }

    #[test]
    fn alu_results_and_flags() {
        for (operation, a, f, val, a_out, f_out) in [
            (ADD, 0x0F, FLAG_C, 0x01, 0x10, FLAG_H),
            (ADD, 0xFF, 0, 0x01, 0x00, FLAG_Z | FLAG_H | FLAG_C),
            (ADC, 0xFF, FLAG_C, 0xFF, 0xFF, FLAG_H | FLAG_C),
            (ADC, 0x00, FLAG_C, 0xFF, 0x00, FLAG_Z | FLAG_H | FLAG_C),
            (SUB, 0x10, 0, 0x01, 0x0F, FLAG_N | FLAG_H),
            (
                SBC,
                0x00,
                FLAG_C,
                0xFF,
                0x00,
                FLAG_Z | FLAG_N | FLAG_H | FLAG_C,
            ),
            (SBC, 0x01, FLAG_C, 0x00, 0x00, FLAG_Z | FLAG_N),
            (4, 0xF0, FLAG_C, 0x0F, 0x00, FLAG_Z | FLAG_H),
            (5, 0xFF, FLAG_C, 0x0F, 0xF0, 0),
            (6, 0x00, FLAG_C, 0x00, 0x00, FLAG_Z),
            (CP, 0x01, 0, 0x02, 0x01, FLAG_N | FLAG_H | FLAG_C),
        ] {
            let mut regs = regs(a, f);
            regs.alu(operation, val);
            assert_eq!((regs.a, regs.f), (a_out, f_out), "op {operation} {a:#x}");
        }
    }

    #[test]
    fn decimal_adjust_matches_bcd_addition_and_subtraction() {
        let bcd = |n: u8| (n / 10 * 16) + n % 10;
        for x in 0..100 {
            for y in 0..100 {
                let mut sum = regs(bcd(x), 0);
                sum.alu(ADD, bcd(y));
                sum.accumulator_op(DAA);
                assert_eq!(sum.a, bcd((x + y) % 100), "{x}+{y}");
                assert_eq!(sum.has_flag(FLAG_C), x + y > 99);
                let mut difference = regs(bcd(x), 0);
                difference.alu(SUB, bcd(y));
                difference.accumulator_op(DAA);
                assert_eq!(difference.a, bcd((100 + x - y) % 100), "{x}-{y}");
            }
        }
    }

    #[test]
    fn increment_decrement_keep_carry() {
        let mut regs = regs(0, FLAG_C);
        assert_eq!(regs.increment(0xFF), 0);
        assert_eq!(regs.f, FLAG_Z | FLAG_H | FLAG_C);
        assert_eq!(regs.decrement(0), 0xFF);
        assert_eq!(regs.f, FLAG_N | FLAG_H | FLAG_C);
    }

    #[test]
    fn add_hl_keeps_zero_flag() {
        let mut regs = regs(0, FLAG_Z);
        regs.set_pair(R16::Hl, 0x0FFF);
        regs.add_hl(1);
        assert_eq!((regs.pair(R16::Hl), regs.f), (0x1000, FLAG_Z | FLAG_H));
        regs.add_hl(0xF000);
        assert_eq!((regs.pair(R16::Hl), regs.f), (0, FLAG_Z | FLAG_C));
    }

    #[test]
    fn sp_plus_flags_come_from_low_byte() {
        for (sp, offset, result, f) in [
            (0x00FF, 0x01, 0x0100, FLAG_H | FLAG_C),
            (0x0000, 0xFF, 0xFFFF, 0),
            (0xFFF8, 0x08, 0x0000, FLAG_H | FLAG_C),
        ] {
            let mut regs = Registers {
                sp,
                ..Registers::default()
            };
            assert_eq!((regs.sp_plus(offset), regs.f), (result, f));
        }
    }

    #[test]
    fn shift_results_and_flags() {
        for (operation, val, f, result, f_out) in [
            (0, 0x80, 0, 0x01, FLAG_C),
            (1, 0x01, 0, 0x80, FLAG_C),
            (2, 0x80, FLAG_C, 0x01, FLAG_C),
            (3, 0x01, FLAG_C, 0x80, FLAG_C),
            (4, 0x80, 0, 0x00, FLAG_Z | FLAG_C),
            (5, 0x81, 0, 0xC0, FLAG_C),
            (6, 0x12, FLAG_C, 0x21, 0),
            (7, 0x81, 0, 0x40, FLAG_C),
        ] {
            let mut regs = regs(0, f);
            assert_eq!((regs.shift(operation, val), regs.f), (result, f_out));
        }
    }

    #[test]
    fn accumulator_op_flags() {
        for (operation, a, f, a_out, f_out) in [
            (2, 0x80, FLAG_Z, 0x00, FLAG_C),
            (5, 0x0F, FLAG_C, 0xF0, FLAG_N | FLAG_H | FLAG_C),
            (6, 0, FLAG_Z | FLAG_N, 0, FLAG_Z | FLAG_C),
            (7, 0, FLAG_C | FLAG_H, 0, 0),
        ] {
            let mut regs = regs(a, f);
            regs.accumulator_op(operation);
            assert_eq!((regs.a, regs.f), (a_out, f_out));
        }
    }

    #[test]
    fn test_bit_sets_zero_when_clear() {
        let mut regs = regs(0, FLAG_C | FLAG_N);
        regs.test_bit(7, 0x7F);
        assert_eq!(regs.f, FLAG_Z | FLAG_H | FLAG_C);
        regs.test_bit(0, 0x01);
        assert_eq!(regs.f, FLAG_H | FLAG_C);
    }

    #[test]
    fn sign_extend_covers_extremes() {
        assert_eq!([0x7F, 0x80, 0xFF].map(sign_extend), [127, -128, -1]);
    }
}
