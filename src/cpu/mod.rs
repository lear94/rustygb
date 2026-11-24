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

mod alu;
mod execute;
mod execute_cb;
mod execute_misc;

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
