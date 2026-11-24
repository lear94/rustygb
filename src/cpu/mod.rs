//! SM83 CPU: instruction stepping, interrupts and low-power states.
//!
//! Every memory access and internal delay advances the machine by one M-cycle, so
//! peripherals observe accesses at the cycle they happen.

mod alu;
mod execute;
mod registers;
#[cfg(test)]
mod tests;

pub use registers::{Registers, FLAG_C, FLAG_H, FLAG_N, FLAG_Z};

use crate::bus::{Motherboard, T_CYCLES_PER_M_CYCLE};
use registers::{R16, R8};

const INTERRUPT_VECTOR_BASE: u16 = 0x40;
const INTERRUPT_VECTOR_STRIDE: u16 = 8;

/// SM83 CPU state.
#[derive(Debug, Default)]
pub struct Cpu {
    /// Register file.
    pub regs: Registers,
    /// Interrupt master enable.
    pub ime: bool,
    halted: bool,
    stopped: bool,
    /// EI takes effect after the following instruction.
    ei_pending: bool,
    /// The next fetch does not advance PC.
    halt_bug: bool,
    /// M-cycles consumed by the current step.
    m_cycles: u32,
}

impl Cpu {
    /// Creates a CPU in the state the DMG boot ROM leaves behind.
    #[must_use]
    pub fn new() -> Self {
        Self {
            regs: Registers::POST_BOOT,
            ..Self::default()
        }
    }

    /// Executes one instruction, interrupt dispatch or idle cycle.
    ///
    /// # Returns
    /// T-cycles consumed.
    pub fn step(&mut self, bus: &mut Motherboard) -> u32 {
        self.m_cycles = 0;
        self.advance(bus);
        self.m_cycles * T_CYCLES_PER_M_CYCLE
    }

    fn advance(&mut self, bus: &mut Motherboard) {
        if self.stopped {
            if !bus.joypad.any_selected_pressed() {
                // The clock is off: time passes for the host only.
                self.m_cycles += 1;
                return;
            }
            self.stopped = false;
        }
        if self.halted {
            if bus.pending_interrupts() == 0 {
                return self.idle(bus);
            }
            self.halted = false;
        }
        // Hardware samples interrupts during the opcode fetch, then discards the opcode.
        let op = self.read(self.regs.pc, bus);
        if self.ime && bus.pending_interrupts() != 0 {
            return self.dispatch_interrupt(bus);
        }
        if std::mem::take(&mut self.ei_pending) {
            self.ime = true;
        }
        self.advance_pc();
        self.execute(op, bus);
    }

    fn dispatch_interrupt(&mut self, bus: &mut Motherboard) {
        self.ime = false;
        self.idle(bus);
        let [high, low] = self.regs.pc.to_be_bytes();
        self.push_byte(high, bus);
        // The high push may overwrite IE, so the source is only chosen here; with none
        // left, hardware jumps to 0x0000.
        self.regs.pc = bus.acknowledge_interrupt().map_or(0, |bit| {
            INTERRUPT_VECTOR_BASE + u16::from(bit) * INTERRUPT_VECTOR_STRIDE
        });
        self.push_byte(low, bus);
        self.idle(bus);
    }

    fn halt(&mut self, bus: &Motherboard) {
        if !self.ime && bus.pending_interrupts() != 0 {
            self.halt_bug = true;
        } else {
            self.halted = true;
        }
    }

    fn stop(&mut self, bus: &mut Motherboard) {
        self.regs.pc = self.regs.pc.wrapping_add(1);
        bus.timer.reset_divider();
        self.stopped = true;
    }

    fn idle(&mut self, bus: &mut Motherboard) {
        bus.tick();
        self.m_cycles += 1;
    }

    fn read(&mut self, addr: u16, bus: &mut Motherboard) -> u8 {
        self.idle(bus);
        bus.read_byte(addr)
    }

    fn write(&mut self, addr: u16, val: u8, bus: &mut Motherboard) {
        self.idle(bus);
        bus.write_byte(addr, val);
    }

    fn fetch(&mut self, bus: &mut Motherboard) -> u8 {
        let val = self.read(self.regs.pc, bus);
        self.advance_pc();
        val
    }

    fn advance_pc(&mut self) {
        if !std::mem::take(&mut self.halt_bug) {
            self.regs.pc = self.regs.pc.wrapping_add(1);
        }
    }

    fn fetch_word(&mut self, bus: &mut Motherboard) -> u16 {
        let low = self.fetch(bus);
        let high = self.fetch(bus);
        u16::from_be_bytes([high, low])
    }

    fn push_byte(&mut self, val: u8, bus: &mut Motherboard) {
        self.regs.sp = self.regs.sp.wrapping_sub(1);
        self.write(self.regs.sp, val, bus);
    }

    fn push(&mut self, val: u16, bus: &mut Motherboard) {
        let [high, low] = val.to_be_bytes();
        self.push_byte(high, bus);
        self.push_byte(low, bus);
    }

    fn pop_byte(&mut self, bus: &mut Motherboard) -> u8 {
        let val = self.read(self.regs.sp, bus);
        self.regs.sp = self.regs.sp.wrapping_add(1);
        val
    }

    fn pop(&mut self, bus: &mut Motherboard) -> u16 {
        let low = self.pop_byte(bus);
        let high = self.pop_byte(bus);
        u16::from_be_bytes([high, low])
    }

    fn load(&mut self, operand: R8, bus: &mut Motherboard) -> u8 {
        match operand {
            R8::B => self.regs.b,
            R8::C => self.regs.c,
            R8::D => self.regs.d,
            R8::E => self.regs.e,
            R8::H => self.regs.h,
            R8::L => self.regs.l,
            R8::A => self.regs.a,
            R8::HlMem => self.read(self.regs.pair(R16::Hl), bus),
        }
    }

    fn store(&mut self, operand: R8, val: u8, bus: &mut Motherboard) {
        match operand {
            R8::B => self.regs.b = val,
            R8::C => self.regs.c = val,
            R8::D => self.regs.d = val,
            R8::E => self.regs.e = val,
            R8::H => self.regs.h = val,
            R8::L => self.regs.l = val,
            R8::A => self.regs.a = val,
            R8::HlMem => self.write(self.regs.pair(R16::Hl), val, bus),
        }
    }
}
