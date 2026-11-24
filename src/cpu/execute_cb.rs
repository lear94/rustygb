//! `0xCB` opcodes plus register-to-register loads and INC/DEC.

use super::{Cpu, FLAG_C, FLAG_H, FLAG_N, FLAG_Z};
use crate::bus::Motherboard;

impl Cpu {
    /// Decode and execute a `0xCB`-prefixed bit-manipulation opcode.
    pub(super) fn execute_cb(&mut self, op: u8, bus: &mut Motherboard) -> u8 {
        let r_idx = op & 0x07;
        let mut v = self.get_r(r_idx, bus);
        let group = op >> 6;
        let bit = (op >> 3) & 0x07;

        match group {
            0 => {
                v = self.cb_shift_rotate(bit, v);
                self.set_r(r_idx, v, bus);
                if r_idx == 6 {
                    16
                } else {
                    8
                }
            }
            1 => {
                // BIT b, r — only updates flags.
                let result = v & (1 << bit);
                self.f = (self.f & FLAG_C) | FLAG_H | if result == 0 { FLAG_Z } else { 0 };
                if r_idx == 6 {
                    12
                } else {
                    8
                }
            }
            2 => {
                v &= !(1 << bit);
                self.set_r(r_idx, v, bus);
                if r_idx == 6 {
                    16
                } else {
                    8
                }
            }
            _ => {
                v |= 1 << bit;
                self.set_r(r_idx, v, bus);
                if r_idx == 6 {
                    16
                } else {
                    8
                }
            }
        }
    }

    pub(super) fn cb_shift_rotate(&mut self, op: u8, v: u8) -> u8 {
        let (result, carry) = match op {
            0 => {
                let c = v >> 7;
                let r = (v << 1) | c;
                (r, c != 0)
            } // RLC
            1 => {
                let c = v & 1;
                let r = (v >> 1) | (c << 7);
                (r, c != 0)
            } // RRC
            2 => {
                let c = v >> 7;
                let r = (v << 1) | if self.flag_c() { 1 } else { 0 };
                (r, c != 0)
            } // RL
            3 => {
                let c = v & 1;
                let r = (v >> 1) | if self.flag_c() { 0x80 } else { 0 };
                (r, c != 0)
            } // RR
            4 => {
                let c = v >> 7;
                (v << 1, c != 0)
            } // SLA
            5 => {
                let c = v & 1;
                ((v as i8 >> 1) as u8, c != 0)
            } // SRA
            6 => (v.rotate_right(4), false), // SWAP
            _ => {
                let c = v & 1;
                (v >> 1, c != 0)
            } // SRL
        };
        self.f = if result == 0 { FLAG_Z } else { 0 };
        if carry {
            self.f |= FLAG_C;
        }
        result
    }

    pub(super) fn exec_ld_r_r(&mut self, op: u8, bus: &mut Motherboard) -> u8 {
        // 0x76 is HALT, not LD (HL),(HL).
        if op == 0x76 {
            return 4;
        }
        let src = self.get_r(op & 7, bus);
        self.set_r((op >> 3) & 7, src, bus);
        if (op & 7) == 6 || ((op >> 3) & 7) == 6 {
            8
        } else {
            4
        }
    }

    pub(super) fn exec_inc_dec(&mut self, op: u8, bus: &mut Motherboard) -> u8 {
        let r_idx = (op >> 3) & 7;
        let v = self.get_r(r_idx, bus);
        let (result, is_dec) = if (op & 1) == 0 {
            (v.wrapping_add(1), false)
        } else {
            (v.wrapping_sub(1), true)
        };
        let mut flags = self.f & FLAG_C;
        if result == 0 {
            flags |= FLAG_Z;
        }
        if is_dec {
            flags |= FLAG_N;
            if (v & 0x0F) == 0 {
                flags |= FLAG_H;
            }
        } else if (v & 0x0F) == 0x0F {
            flags |= FLAG_H;
        }
        self.f = flags;
        self.set_r(r_idx, result, bus);
        if r_idx == 6 {
            12
        } else {
            4
        }
    }
}
