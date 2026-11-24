//! Accumulator and 16-bit arithmetic with flag updates.

use super::{Cpu, FLAG_C, FLAG_H, FLAG_N, FLAG_Z};

impl Cpu {
    pub(super) fn add(&mut self, v: u8) {
        let r = self.a.wrapping_add(v);
        self.f = if r == 0 { FLAG_Z } else { 0 }
            | if (self.a & 0x0F) + (v & 0x0F) > 0x0F {
                FLAG_H
            } else {
                0
            }
            | if (self.a as u16) + (v as u16) > 0xFF {
                FLAG_C
            } else {
                0
            };
        self.a = r;
    }

    pub(super) fn adc(&mut self, v: u8) {
        let carry = if self.flag_c() { 1 } else { 0 };
        let r = self.a.wrapping_add(v).wrapping_add(carry);
        self.f = if r == 0 { FLAG_Z } else { 0 }
            | if (self.a & 0x0F) + (v & 0x0F) + carry > 0x0F {
                FLAG_H
            } else {
                0
            }
            | if (self.a as u16) + (v as u16) + (carry as u16) > 0xFF {
                FLAG_C
            } else {
                0
            };
        self.a = r;
    }

    pub(super) fn sub(&mut self, v: u8) {
        let r = self.a.wrapping_sub(v);
        self.f = FLAG_N
            | if r == 0 { FLAG_Z } else { 0 }
            | if (self.a & 0x0F) < (v & 0x0F) {
                FLAG_H
            } else {
                0
            }
            | if self.a < v { FLAG_C } else { 0 };
        self.a = r;
    }

    pub(super) fn sbc(&mut self, v: u8) {
        let carry = if self.flag_c() { 1 } else { 0 };
        let r = self.a.wrapping_sub(v).wrapping_sub(carry);
        self.f = FLAG_N
            | if r == 0 { FLAG_Z } else { 0 }
            | if (self.a as u16) < (v as u16) + (carry as u16) {
                FLAG_C
            } else {
                0
            }
            | if (self.a & 0x0F) < (v & 0x0F) + carry {
                FLAG_H
            } else {
                0
            };
        self.a = r;
    }

    pub(super) fn and(&mut self, v: u8) {
        self.a &= v;
        self.f = FLAG_H | if self.a == 0 { FLAG_Z } else { 0 };
    }

    pub(super) fn or(&mut self, v: u8) {
        self.a |= v;
        self.f = if self.a == 0 { FLAG_Z } else { 0 };
    }

    pub(super) fn xor(&mut self, v: u8) {
        self.a ^= v;
        self.f = if self.a == 0 { FLAG_Z } else { 0 };
    }

    pub(super) fn cp(&mut self, v: u8) {
        self.f = FLAG_N
            | if self.a == v { FLAG_Z } else { 0 }
            | if (self.a & 0x0F) < (v & 0x0F) {
                FLAG_H
            } else {
                0
            }
            | if self.a < v { FLAG_C } else { 0 };
    }

    pub(super) fn add_hl(&mut self, v: u16) {
        let hl = self.get_hl();
        let result = hl.wrapping_add(v);
        self.f = (self.f & FLAG_Z)
            | if (hl & 0x0FFF) + (v & 0x0FFF) > 0x0FFF {
                FLAG_H
            } else {
                0
            }
            | if (hl as u32) + (v as u32) > 0xFFFF {
                FLAG_C
            } else {
                0
            };
        self.set_hl(result);
    }

    /// Binary-Coded-Decimal adjustment after an ADD/ADC/SUB/SBC on A.
    pub(super) fn daa(&mut self) {
        let mut a = self.a;
        let mut adjustment: u8 = 0;
        if self.flag_h() || (!self.flag_n() && (a & 0x0F) > 9) {
            adjustment |= 0x06;
        }
        if self.flag_c() || (!self.flag_n() && a > 0x99) {
            adjustment |= 0x60;
            self.set_flag(FLAG_C, true);
        }
        a = if self.flag_n() {
            a.wrapping_sub(adjustment)
        } else {
            a.wrapping_add(adjustment)
        };
        self.f = (self.f & FLAG_C)
            | if self.flag_n() { FLAG_N } else { 0 }
            | if a == 0 { FLAG_Z } else { 0 };
        self.a = a;
    }
}
