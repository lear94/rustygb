//! Primary opcode dispatch: loads, jumps, rotations and A arithmetic.

use super::{Cpu, FLAG_C};
use crate::bus::Motherboard;

impl Cpu {
    /// Decode and execute a single primary opcode. The `0xCB` prefix
    /// dispatches to [`Cpu::execute_cb`].
    pub(super) fn execute(&mut self, op: u8, bus: &mut Motherboard) -> u8 {
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
            _ => self.execute_misc(op, bus),
        }
    }
}
