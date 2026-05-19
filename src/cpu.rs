//! Sharp SM83 CPU (Game Boy's "Z80-like" core).
//!
//! Implements the full 245-opcode primary instruction set plus the 256
//! `0xCB`-prefixed bit-manipulation opcodes. Each [`Cpu::step`] decodes,
//! executes and reports the number of T-cycles consumed by a single
//! instruction (or by an interrupt dispatch), which the bus then forwards
//! to every peripheral via [`crate::bus::Motherboard::tick`].
//!
//! The implementation prioritises clarity: registers are kept as separate
//! `u8` fields, helpers expose `get_*`/`set_*` views over the 16-bit
//! register pairs, and the dispatcher is a single `match` against the
//! opcode byte.

use crate::bus::Motherboard;

const FLAG_Z: u8 = 0x80;
const FLAG_N: u8 = 0x40;
const FLAG_H: u8 = 0x20;
const FLAG_C: u8 = 0x10;

/// SM83 CPU state.
pub struct Cpu {
    pub a: u8,
    pub f: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub h: u8,
    pub l: u8,
    pub sp: u16,
    pub pc: u16,
    /// Interrupt Master Enable.
    pub ime: bool,
    /// Set while the CPU is in `HALT` waiting for an interrupt.
    pub halted: bool,
    /// `EI` enables interrupts after the following instruction completes;
    /// this flag tracks the one-instruction delay.
    pub ei_pending: bool,
}

impl Cpu {
    pub fn new() -> Self {
        Self {
            a: 0,
            f: 0,
            b: 0,
            c: 0,
            d: 0,
            e: 0,
            h: 0,
            l: 0,
            sp: 0,
            pc: 0,
            ime: false,
            halted: false,
            ei_pending: false,
        }
    }

    /// Initialise registers to the post-boot ROM state for a DMG. This
    /// lets us skip executing the (copyrighted) Nintendo boot ROM and
    /// jump straight into the cartridge entry point at `0x0100`.
    pub fn reset_to_boot(&mut self) {
        self.pc = 0x0100;
        self.a = 0x01;
        self.f = 0xB0;
        self.b = 0x00;
        self.c = 0x13;
        self.d = 0x00;
        self.e = 0xD8;
        self.h = 0x01;
        self.l = 0x4D;
        self.sp = 0xFFFE;
    }

    /// Execute one CPU step. Returns the number of T-cycles consumed.
    /// Handles interrupt dispatch, `HALT` wake-up and the deferred `EI`.
    pub fn step(&mut self, bus: &mut Motherboard) -> u8 {
        let ie = bus.read_byte(0xFFFF);
        let if_reg = bus.read_byte(0xFF0F);
        let pending = ie & if_reg & 0x1F;

        if self.halted {
            if pending != 0 {
                self.halted = false;
            } else {
                return 4;
            }
        }

        if self.ime && pending != 0 {
            return self.service_interrupt(bus, pending, if_reg);
        }

        if self.ei_pending {
            self.ime = true;
            self.ei_pending = false;
        }

        let op = self.next_byte(bus);
        self.execute(op, bus)
    }

    /// Service the lowest-numbered pending interrupt: acknowledge it in
    /// `IF`, push `PC` and jump to the corresponding vector.
    fn service_interrupt(&mut self, bus: &mut Motherboard, pending: u8, if_reg: u8) -> u8 {
        self.ime = false;
        let bit = pending.trailing_zeros() as u16;
        bus.write_byte(0xFF0F, if_reg & !(1 << bit));
        self.push_word(bus, self.pc);
        self.pc = 0x40 + bit * 8;
        20
    }

    /// Decode and execute a single primary opcode. The `0xCB` prefix
    /// dispatches to [`Cpu::execute_cb`].
    fn execute(&mut self, op: u8, bus: &mut Motherboard) -> u8 {
        match op {
            0x00 => 4,

            // LD r, n8
            0x06 => {
                let v = self.next_byte(bus);
                self.b = v;
                8
            }
            0x0E => {
                let v = self.next_byte(bus);
                self.c = v;
                8
            }
            0x16 => {
                let v = self.next_byte(bus);
                self.d = v;
                8
            }
            0x1E => {
                let v = self.next_byte(bus);
                self.e = v;
                8
            }
            0x26 => {
                let v = self.next_byte(bus);
                self.h = v;
                8
            }
            0x2E => {
                let v = self.next_byte(bus);
                self.l = v;
                8
            }
            0x3E => {
                let v = self.next_byte(bus);
                self.a = v;
                8
            }
            0x36 => {
                let v = self.next_byte(bus);
                bus.write_byte(self.get_hl(), v);
                12
            }

            // LD r, r' (and LD (HL), r / LD r, (HL))
            0x40..=0x75 | 0x77..=0x7F => self.exec_ld_r_r(op, bus),

            // LD rr, n16
            0x01 => {
                let lo = self.next_byte(bus);
                let hi = self.next_byte(bus);
                self.c = lo;
                self.b = hi;
                12
            }
            0x11 => {
                let lo = self.next_byte(bus);
                let hi = self.next_byte(bus);
                self.e = lo;
                self.d = hi;
                12
            }
            0x21 => {
                let lo = self.next_byte(bus);
                let hi = self.next_byte(bus);
                self.l = lo;
                self.h = hi;
                12
            }
            0x31 => {
                let v = self.next_word(bus);
                self.sp = v;
                12
            }

            // LD A, (rr) / LD A, (a16)
            0x0A => {
                self.a = bus.read_byte(self.get_bc());
                8
            }
            0x1A => {
                self.a = bus.read_byte(self.get_de());
                8
            }
            0xFA => {
                let a = self.next_word(bus);
                self.a = bus.read_byte(a);
                16
            }

            // LD (rr), A / LD (a16), A
            0x02 => {
                bus.write_byte(self.get_bc(), self.a);
                8
            }
            0x12 => {
                bus.write_byte(self.get_de(), self.a);
                8
            }
            0xEA => {
                let a = self.next_word(bus);
                bus.write_byte(a, self.a);
                16
            }

            // LD (HL+), A / LD A, (HL+) / LD (HL-), A / LD A, (HL-)
            0x22 => {
                bus.write_byte(self.get_hl(), self.a);
                self.set_hl(self.get_hl().wrapping_add(1));
                8
            }
            0x2A => {
                self.a = bus.read_byte(self.get_hl());
                self.set_hl(self.get_hl().wrapping_add(1));
                8
            }
            0x32 => {
                bus.write_byte(self.get_hl(), self.a);
                self.set_hl(self.get_hl().wrapping_sub(1));
                8
            }
            0x3A => {
                self.a = bus.read_byte(self.get_hl());
                self.set_hl(self.get_hl().wrapping_sub(1));
                8
            }

            // PUSH rr
            0xC5 => {
                self.push_word(bus, self.get_bc());
                16
            }
            0xD5 => {
                self.push_word(bus, self.get_de());
                16
            }
            0xE5 => {
                self.push_word(bus, self.get_hl());
                16
            }
            0xF5 => {
                self.push_word(bus, (self.a as u16) << 8 | self.f as u16);
                16
            }

            // POP rr
            0xC1 => {
                let v = self.pop_word(bus);
                self.b = (v >> 8) as u8;
                self.c = v as u8;
                12
            }
            0xD1 => {
                let v = self.pop_word(bus);
                self.d = (v >> 8) as u8;
                self.e = v as u8;
                12
            }
            0xE1 => {
                let v = self.pop_word(bus);
                self.h = (v >> 8) as u8;
                self.l = v as u8;
                12
            }
            // POP AF: bottom nibble of F is hard-wired to zero.
            0xF1 => {
                let v = self.pop_word(bus);
                self.a = (v >> 8) as u8;
                self.f = (v as u8) & 0xF0;
                12
            }

            // Jumps
            0xC3 => {
                self.pc = self.next_word(bus);
                16
            }
            0xE9 => {
                self.pc = self.get_hl();
                4
            }
            0x18 => {
                let o = self.next_byte(bus) as i8;
                self.pc = self.pc.wrapping_add(o as u16);
                12
            }
            0x20 => self.jr_cond(bus, !self.flag_z()),
            0x28 => self.jr_cond(bus, self.flag_z()),
            0x30 => self.jr_cond(bus, !self.flag_c()),
            0x38 => self.jr_cond(bus, self.flag_c()),
            0xC2 => self.jp_cond(bus, !self.flag_z()),
            0xCA => self.jp_cond(bus, self.flag_z()),
            0xD2 => self.jp_cond(bus, !self.flag_c()),
            0xDA => self.jp_cond(bus, self.flag_c()),

            // CALL / conditional CALL
            0xCD => {
                let d = self.next_word(bus);
                self.push_word(bus, self.pc);
                self.pc = d;
                24
            }
            0xC4 => self.call_cond(bus, !self.flag_z()),
            0xCC => self.call_cond(bus, self.flag_z()),
            0xD4 => self.call_cond(bus, !self.flag_c()),
            0xDC => self.call_cond(bus, self.flag_c()),

            // RET / RETI / conditional RET
            0xC9 => {
                self.pc = self.pop_word(bus);
                16
            }
            0xD9 => {
                self.pc = self.pop_word(bus);
                self.ime = true;
                16
            }
            0xC0 => self.ret_cond(bus, !self.flag_z()),
            0xC8 => self.ret_cond(bus, self.flag_z()),
            0xD0 => self.ret_cond(bus, !self.flag_c()),
            0xD8 => self.ret_cond(bus, self.flag_c()),

            // RST n
            0xC7 | 0xCF | 0xD7 | 0xDF | 0xE7 | 0xEF | 0xF7 | 0xFF => {
                self.push_word(bus, self.pc);
                self.pc = (op - 0xC7) as u16;
                16
            }

            // Rotations on A (do not set Z).
            0x07 => {
                let c = self.a >> 7;
                self.a = (self.a << 1) | c;
                self.f = if c != 0 { FLAG_C } else { 0 };
                4
            }
            0x0F => {
                let c = self.a & 1;
                self.a = (self.a >> 1) | (c << 7);
                self.f = if c != 0 { FLAG_C } else { 0 };
                4
            }
            0x17 => {
                let c = self.a >> 7;
                let carry_in = if (self.f & FLAG_C) != 0 { 1 } else { 0 };
                self.a = (self.a << 1) | carry_in;
                self.f = if c != 0 { FLAG_C } else { 0 };
                4
            }
            0x1F => {
                let c = self.a & 1;
                let carry_in = if (self.f & FLAG_C) != 0 { 0x80 } else { 0 };
                self.a = (self.a >> 1) | carry_in;
                self.f = if c != 0 { FLAG_C } else { 0 };
                4
            }

            // 8-bit arithmetic on r
            0x80..=0x87 => {
                let v = self.get_r(op & 7, bus);
                self.add(v);
                4
            }
            0x88..=0x8F => {
                let v = self.get_r(op & 7, bus);
                self.adc(v);
                4
            }
            0x90..=0x97 => {
                let v = self.get_r(op & 7, bus);
                self.sub(v);
                4
            }
            0x98..=0x9F => {
                let v = self.get_r(op & 7, bus);
                self.sbc(v);
                4
            }
            0xA0..=0xA7 => {
                let v = self.get_r(op & 7, bus);
                self.and(v);
                4
            }
            0xA8..=0xAF => {
                let v = self.get_r(op & 7, bus);
                self.xor(v);
                4
            }
            0xB0..=0xB7 => {
                let v = self.get_r(op & 7, bus);
                self.or(v);
                4
            }
            0xB8..=0xBF => {
                let v = self.get_r(op & 7, bus);
                self.cp(v);
                4
            }

            // 8-bit arithmetic on n8
            0xC6 => {
                let v = self.next_byte(bus);
                self.add(v);
                8
            }
            0xCE => {
                let v = self.next_byte(bus);
                self.adc(v);
                8
            }
            0xD6 => {
                let v = self.next_byte(bus);
                self.sub(v);
                8
            }
            0xDE => {
                let v = self.next_byte(bus);
                self.sbc(v);
                8
            }
            0xE6 => {
                let v = self.next_byte(bus);
                self.and(v);
                8
            }
            0xEE => {
                let v = self.next_byte(bus);
                self.xor(v);
                8
            }
            0xF6 => {
                let v = self.next_byte(bus);
                self.or(v);
                8
            }
            0xFE => {
                let v = self.next_byte(bus);
                self.cp(v);
                8
            }

            // INC r / DEC r / INC (HL) / DEC (HL)
            0x04 | 0x0C | 0x14 | 0x1C | 0x24 | 0x2C | 0x3C | 0x34 | 0x05 | 0x0D | 0x15 | 0x1D
            | 0x25 | 0x2D | 0x3D | 0x35 => self.exec_inc_dec(op, bus),

            // ADD HL, rr
            0x09 => {
                let v = self.get_bc();
                self.add_hl(v);
                8
            }
            0x19 => {
                let v = self.get_de();
                self.add_hl(v);
                8
            }
            0x29 => {
                let v = self.get_hl();
                self.add_hl(v);
                8
            }
            0x39 => {
                let v = self.sp;
                self.add_hl(v);
                8
            }

            // INC rr / DEC rr
            0x03 => {
                let v = self.get_bc().wrapping_add(1);
                self.set_bc(v);
                8
            }
            0x13 => {
                let v = self.get_de().wrapping_add(1);
                self.set_de(v);
                8
            }
            0x23 => {
                let v = self.get_hl().wrapping_add(1);
                self.set_hl(v);
                8
            }
            0x33 => {
                self.sp = self.sp.wrapping_add(1);
                8
            }
            0x0B => {
                let v = self.get_bc().wrapping_sub(1);
                self.set_bc(v);
                8
            }
            0x1B => {
                let v = self.get_de().wrapping_sub(1);
                self.set_de(v);
                8
            }
            0x2B => {
                let v = self.get_hl().wrapping_sub(1);
                self.set_hl(v);
                8
            }
            0x3B => {
                self.sp = self.sp.wrapping_sub(1);
                8
            }

            0x27 => {
                self.daa();
                4
            }
            0x2F => {
                self.a = !self.a;
                self.set_flag(FLAG_N, true);
                self.set_flag(FLAG_H, true);
                4
            }
            0x3F => {
                let c = !self.flag_c();
                self.set_flag(FLAG_C, c);
                self.set_flag(FLAG_N, false);
                self.set_flag(FLAG_H, false);
                4
            }
            0x37 => {
                self.set_flag(FLAG_C, true);
                self.set_flag(FLAG_N, false);
                self.set_flag(FLAG_H, false);
                4
            }

            // LD HL, SP+e8 / ADD SP, e8
            0xF8 => {
                let offset = self.next_byte(bus) as i8 as u16;
                let result = self.sp.wrapping_add(offset);
                self.f = 0;
                if (self.sp & 0x0F) + (offset & 0x0F) > 0x0F {
                    self.set_flag(FLAG_H, true);
                }
                if (self.sp & 0xFF) + (offset & 0xFF) > 0xFF {
                    self.set_flag(FLAG_C, true);
                }
                self.set_hl(result);
                12
            }
            0xE8 => {
                let offset = self.next_byte(bus) as i8 as u16;
                let result = self.sp.wrapping_add(offset);
                self.f = 0;
                if (self.sp & 0x0F) + (offset & 0x0F) > 0x0F {
                    self.set_flag(FLAG_H, true);
                }
                if (self.sp & 0xFF) + (offset & 0xFF) > 0xFF {
                    self.set_flag(FLAG_C, true);
                }
                self.sp = result;
                16
            }
            0xF9 => {
                self.sp = self.get_hl();
                8
            }

            // High-RAM page accesses (LDH).
            0xE0 => {
                let v = self.next_byte(bus);
                bus.write_byte(0xFF00 + v as u16, self.a);
                12
            }
            0xF0 => {
                let v = self.next_byte(bus);
                self.a = bus.read_byte(0xFF00 + v as u16);
                12
            }
            0xE2 => {
                bus.write_byte(0xFF00 + self.c as u16, self.a);
                8
            }
            0xF2 => {
                self.a = bus.read_byte(0xFF00 + self.c as u16);
                8
            }

            // STOP — treat as a 2-byte NOP (CGB speed switch is not modelled).
            0x10 => {
                let _ = self.next_byte(bus);
                4
            }

            // HALT / DI / EI
            0x76 => {
                self.halted = true;
                4
            }
            0xF3 => {
                self.ime = false;
                4
            }
            0xFB => {
                self.ei_pending = true;
                4
            }

            // CB-prefixed instructions
            0xCB => {
                let cb_op = self.next_byte(bus);
                self.execute_cb(cb_op, bus)
            }

            // Undefined opcodes — behave as a 1-cycle NOP.
            _ => 4,
        }
    }

    /// Decode and execute a `0xCB`-prefixed bit-manipulation opcode.
    fn execute_cb(&mut self, op: u8, bus: &mut Motherboard) -> u8 {
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

    fn cb_shift_rotate(&mut self, op: u8, v: u8) -> u8 {
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
            6 => ((v << 4) | (v >> 4), false), // SWAP
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

    fn exec_ld_r_r(&mut self, op: u8, bus: &mut Motherboard) -> u8 {
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

    fn exec_inc_dec(&mut self, op: u8, bus: &mut Motherboard) -> u8 {
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

    // ----- Arithmetic helpers (operate on A, update flags) ---------------

    fn add(&mut self, v: u8) {
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

    fn adc(&mut self, v: u8) {
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

    fn sub(&mut self, v: u8) {
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

    fn sbc(&mut self, v: u8) {
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

    fn and(&mut self, v: u8) {
        self.a &= v;
        self.f = FLAG_H | if self.a == 0 { FLAG_Z } else { 0 };
    }

    fn or(&mut self, v: u8) {
        self.a |= v;
        self.f = if self.a == 0 { FLAG_Z } else { 0 };
    }

    fn xor(&mut self, v: u8) {
        self.a ^= v;
        self.f = if self.a == 0 { FLAG_Z } else { 0 };
    }

    fn cp(&mut self, v: u8) {
        self.f = FLAG_N
            | if self.a == v { FLAG_Z } else { 0 }
            | if (self.a & 0x0F) < (v & 0x0F) {
                FLAG_H
            } else {
                0
            }
            | if self.a < v { FLAG_C } else { 0 };
    }

    fn add_hl(&mut self, v: u16) {
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
    fn daa(&mut self) {
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

    // ----- Flag and register helpers -------------------------------------

    fn flag_z(&self) -> bool {
        self.f & FLAG_Z != 0
    }
    fn flag_n(&self) -> bool {
        self.f & FLAG_N != 0
    }
    fn flag_h(&self) -> bool {
        self.f & FLAG_H != 0
    }
    fn flag_c(&self) -> bool {
        self.f & FLAG_C != 0
    }

    fn set_flag(&mut self, flag: u8, on: bool) {
        if on {
            self.f |= flag;
        } else {
            self.f &= !flag;
        }
    }

    fn get_bc(&self) -> u16 {
        (self.b as u16) << 8 | self.c as u16
    }
    fn get_de(&self) -> u16 {
        (self.d as u16) << 8 | self.e as u16
    }
    fn get_hl(&self) -> u16 {
        (self.h as u16) << 8 | self.l as u16
    }
    fn set_bc(&mut self, v: u16) {
        self.b = (v >> 8) as u8;
        self.c = v as u8;
    }
    fn set_de(&mut self, v: u16) {
        self.d = (v >> 8) as u8;
        self.e = v as u8;
    }
    fn set_hl(&mut self, v: u16) {
        self.h = (v >> 8) as u8;
        self.l = v as u8;
    }

    fn next_byte(&mut self, bus: &mut Motherboard) -> u8 {
        let v = bus.read_byte(self.pc);
        self.pc = self.pc.wrapping_add(1);
        v
    }

    fn next_word(&mut self, bus: &mut Motherboard) -> u16 {
        let lo = self.next_byte(bus);
        let hi = self.next_byte(bus);
        (hi as u16) << 8 | lo as u16
    }

    fn push_word(&mut self, bus: &mut Motherboard, v: u16) {
        self.sp = self.sp.wrapping_sub(2);
        bus.write_word(self.sp, v);
    }

    fn pop_word(&mut self, bus: &mut Motherboard) -> u16 {
        let v = bus.read_word(self.sp);
        self.sp = self.sp.wrapping_add(2);
        v
    }

    /// Read register `i` using the standard SM83 numbering
    /// (0=B, 1=C, 2=D, 3=E, 4=H, 5=L, 6=(HL), 7=A).
    fn get_r(&self, i: u8, bus: &Motherboard) -> u8 {
        match i {
            0 => self.b,
            1 => self.c,
            2 => self.d,
            3 => self.e,
            4 => self.h,
            5 => self.l,
            6 => bus.read_byte(self.get_hl()),
            7 => self.a,
            _ => 0,
        }
    }

    /// Write register `i` (same numbering as [`Cpu::get_r`]).
    fn set_r(&mut self, i: u8, v: u8, bus: &mut Motherboard) {
        match i {
            0 => self.b = v,
            1 => self.c = v,
            2 => self.d = v,
            3 => self.e = v,
            4 => self.h = v,
            5 => self.l = v,
            6 => bus.write_byte(self.get_hl(), v),
            7 => self.a = v,
            _ => {}
        }
    }

    fn jr_cond(&mut self, bus: &mut Motherboard, cond: bool) -> u8 {
        let offset = self.next_byte(bus) as i8;
        if cond {
            self.pc = self.pc.wrapping_add(offset as u16);
            12
        } else {
            8
        }
    }

    fn jp_cond(&mut self, bus: &mut Motherboard, cond: bool) -> u8 {
        let dest = self.next_word(bus);
        if cond {
            self.pc = dest;
            16
        } else {
            12
        }
    }

    fn call_cond(&mut self, bus: &mut Motherboard, cond: bool) -> u8 {
        let dest = self.next_word(bus);
        if cond {
            self.push_word(bus, self.pc);
            self.pc = dest;
            24
        } else {
            12
        }
    }

    fn ret_cond(&mut self, bus: &mut Motherboard, cond: bool) -> u8 {
        if cond {
            self.pc = self.pop_word(bus);
            20
        } else {
            8
        }
    }
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}
