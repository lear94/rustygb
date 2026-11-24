//! Primary opcode dispatch: 16-bit arithmetic, flags, LDH and control.

use super::{Cpu, FLAG_C, FLAG_H, FLAG_N};
use crate::bus::Motherboard;

impl Cpu {
    /// Execute the opcodes [`Cpu::execute`] does not handle itself.
    pub(super) fn execute_misc(&mut self, op: u8, bus: &mut Motherboard) -> u8 {
        match op {
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
}
