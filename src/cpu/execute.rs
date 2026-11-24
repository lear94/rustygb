//! Opcode decoding and execution, one function per opcode column.
//!
//! Opcodes are split as `xx yyy zzz`, with `yyy = pp q`.

use super::alu::sign_extend;
use super::registers::{R16, R8};
use super::{Activity, Cpu, EnableDelay};
use crate::bus::Motherboard;

/// Base of the I/O and high-RAM page reached by LDH.
const HIGH_PAGE: u8 = 0xFF;
/// The encoding of LD (HL),(HL) is HALT instead.
const HALT_OPCODE: u8 = 0x76;
/// RST targets sit 8 bytes apart, so the masked opcode is the address.
const RST_MASK: u8 = 0x38;

impl Cpu {
    pub(super) fn execute(&mut self, op: u8, bus: &mut Motherboard) {
        let (y, z) = ((op >> 3) & 7, op & 7);
        match op >> 6 {
            0 => self.execute_block_0(y, z, bus),
            1 if op == HALT_OPCODE => self.halt(bus),
            1 => {
                let val = self.load(R8::from_bits(z), bus);
                self.store(R8::from_bits(y), val, bus);
            }
            2 => {
                let val = self.load(R8::from_bits(z), bus);
                self.regs.alu(y, val);
            }
            _ => self.execute_block_3(op, bus),
        }
    }

    fn execute_block_0(&mut self, y: u8, z: u8, bus: &mut Motherboard) {
        let operand = R8::from_bits(y);
        match z {
            0 => self.execute_relative(y, bus),
            1 => self.execute_pair_load_add(y, bus),
            2 => self.execute_indirect(y, bus),
            3 => {
                let pair = R16::from_bits(y >> 1, R16::Sp);
                let delta = if y & 1 == 0 { 1 } else { -1 };
                let val = self.regs.pair(pair).wrapping_add_signed(delta);
                self.regs.set_pair(pair, val);
                self.idle(bus);
            }
            4 => {
                let val = self.load(operand, bus);
                let result = self.regs.increment(val);
                self.store(operand, result, bus);
            }
            5 => {
                let val = self.load(operand, bus);
                let result = self.regs.decrement(val);
                self.store(operand, result, bus);
            }
            6 => {
                let val = self.fetch(bus);
                self.store(operand, val, bus);
            }
            _ => self.regs.accumulator_op(y),
        }
    }

    fn execute_relative(&mut self, y: u8, bus: &mut Motherboard) {
        match y {
            0 => {}
            1 => {
                let addr = self.fetch_word(bus);
                let [high, low] = self.regs.sp.to_be_bytes();
                self.write(addr, low, bus);
                self.write(addr.wrapping_add(1), high, bus);
            }
            2 => self.stop(bus),
            _ => {
                let offset = sign_extend(self.fetch(bus));
                if y == 3 || self.regs.condition(y) {
                    self.regs.pc = self.regs.pc.wrapping_add_signed(offset);
                    self.idle(bus);
                }
            }
        }
    }

    fn execute_pair_load_add(&mut self, y: u8, bus: &mut Motherboard) {
        let pair = R16::from_bits(y >> 1, R16::Sp);
        if y & 1 == 0 {
            let val = self.fetch_word(bus);
            self.regs.set_pair(pair, val);
        } else {
            self.regs.add_hl(self.regs.pair(pair));
            self.idle(bus);
        }
    }

    fn execute_indirect(&mut self, y: u8, bus: &mut Motherboard) {
        let (pair, step) = match y >> 1 {
            0 => (R16::Bc, 0),
            1 => (R16::De, 0),
            2 => (R16::Hl, 1),
            _ => (R16::Hl, -1),
        };
        let addr = self.regs.pair(pair);
        if y & 1 == 0 {
            self.write(addr, self.regs.a, bus);
        } else {
            self.regs.a = self.read(addr, bus);
        }
        self.regs.set_pair(pair, addr.wrapping_add_signed(step));
    }

    fn execute_block_3(&mut self, op: u8, bus: &mut Motherboard) {
        let y = (op >> 3) & 7;
        match op & 7 {
            0 => self.execute_ret_cond_or_high(y, bus),
            1 => self.execute_pop_or_ret(y, bus),
            2 => self.execute_jump_cond_or_absolute(y, bus),
            3 => self.execute_control(y, bus),
            4 => {
                if y <= 3 {
                    self.call_if(self.regs.condition(y), bus);
                } else {
                    self.activity = Activity::Locked;
                }
            }
            5 => self.execute_push_or_call(y, bus),
            6 => {
                let val = self.fetch(bus);
                self.regs.alu(y, val);
            }
            _ => {
                self.idle(bus);
                self.push(self.regs.pc, bus);
                self.regs.pc = u16::from(op & RST_MASK);
            }
        }
    }

    fn execute_ret_cond_or_high(&mut self, y: u8, bus: &mut Motherboard) {
        match y {
            4 => {
                let low = self.fetch(bus);
                self.write(u16::from_be_bytes([HIGH_PAGE, low]), self.regs.a, bus);
            }
            5 => {
                let offset = self.fetch(bus);
                self.regs.sp = self.regs.sp_plus(offset);
                self.idle(bus);
                self.idle(bus);
            }
            6 => {
                let low = self.fetch(bus);
                self.regs.a = self.read(u16::from_be_bytes([HIGH_PAGE, low]), bus);
            }
            7 => {
                let offset = self.fetch(bus);
                let result = self.regs.sp_plus(offset);
                self.regs.set_pair(R16::Hl, result);
                self.idle(bus);
            }
            _ => {
                self.idle(bus);
                if self.regs.condition(y) {
                    self.ret(bus);
                }
            }
        }
    }

    fn execute_pop_or_ret(&mut self, y: u8, bus: &mut Motherboard) {
        match y {
            1 => self.ret(bus),
            3 => {
                self.ret(bus);
                self.ime = true;
            }
            5 => self.regs.pc = self.regs.pair(R16::Hl),
            7 => {
                self.regs.sp = self.regs.pair(R16::Hl);
                self.idle(bus);
            }
            _ => {
                let val = self.pop(bus);
                self.regs.set_pair(R16::from_bits(y >> 1, R16::Af), val);
            }
        }
    }

    fn execute_jump_cond_or_absolute(&mut self, y: u8, bus: &mut Motherboard) {
        match y {
            4 => self.write(self.c_page_addr(), self.regs.a, bus),
            5 => {
                let addr = self.fetch_word(bus);
                self.write(addr, self.regs.a, bus);
            }
            6 => self.regs.a = self.read(self.c_page_addr(), bus),
            7 => {
                let addr = self.fetch_word(bus);
                self.regs.a = self.read(addr, bus);
            }
            _ => self.jump_if(self.regs.condition(y), bus),
        }
    }

    fn execute_control(&mut self, y: u8, bus: &mut Motherboard) {
        match y {
            0 => self.jump_if(true, bus),
            1 => {
                let op = self.fetch(bus);
                self.execute_cb(op, bus);
            }
            6 => {
                self.ime = false;
                self.enable_delay = EnableDelay::Idle;
            }
            7 => self.enable_delay = EnableDelay::Requested,
            _ => self.activity = Activity::Locked,
        }
    }

    fn execute_push_or_call(&mut self, y: u8, bus: &mut Motherboard) {
        if y & 1 == 0 {
            self.idle(bus);
            self.push(self.regs.pair(R16::from_bits(y >> 1, R16::Af)), bus);
        } else if y == 1 {
            self.call_if(true, bus);
        } else {
            self.activity = Activity::Locked;
        }
    }

    fn execute_cb(&mut self, op: u8, bus: &mut Motherboard) {
        let operand = R8::from_bits(op);
        let bit = (op >> 3) & 7;
        let val = self.load(operand, bus);
        let result = match op >> 6 {
            0 => self.regs.shift(bit, val),
            1 => return self.regs.test_bit(bit, val),
            2 => val & !(1 << bit),
            _ => val | (1 << bit),
        };
        self.store(operand, result, bus);
    }

    fn c_page_addr(&self) -> u16 {
        u16::from_be_bytes([HIGH_PAGE, self.regs.c])
    }

    fn jump_if(&mut self, taken: bool, bus: &mut Motherboard) {
        let target = self.fetch_word(bus);
        if taken {
            self.regs.pc = target;
            self.idle(bus);
        }
    }

    fn call_if(&mut self, taken: bool, bus: &mut Motherboard) {
        let target = self.fetch_word(bus);
        if taken {
            self.idle(bus);
            self.push(self.regs.pc, bus);
            self.regs.pc = target;
        }
    }

    fn ret(&mut self, bus: &mut Motherboard) {
        self.regs.pc = self.pop(bus);
        self.idle(bus);
    }
}
