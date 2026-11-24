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

use crate::bus::Motherboard;
use crate::hardware::T_CYCLES_PER_M_CYCLE;
use registers::{R16, R8};

const INTERRUPT_VECTOR_BASE: u16 = 0x40;
const INTERRUPT_VECTOR_STRIDE: u16 = 8;

/// What the CPU does between instructions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Activity {
    #[default]
    Running,
    /// Waiting for an interrupt request.
    Halted,
    /// Waiting for a button, with the system clock off.
    Stopped,
    /// Hung by an undefined opcode until reset.
    Locked,
}

/// Progress of an EI, which takes effect after the following instruction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum EnableDelay {
    #[default]
    Idle,
    Requested,
    /// The current instruction is the one following EI.
    Applied,
}

/// SM83 CPU state.
#[derive(Debug, Default)]
pub struct Cpu {
    /// Register file.
    pub regs: Registers,
    /// Interrupt master enable.
    pub ime: bool,
    activity: Activity,
    enable_delay: EnableDelay,
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
        self.m_cycles * u32::from(T_CYCLES_PER_M_CYCLE)
    }

    fn advance(&mut self, bus: &mut Motherboard) {
        match self.activity {
            Activity::Stopped if !bus.joypad.any_selected_pressed() => {
                // Time passes for the host only.
                bus.tick_stopped();
                self.m_cycles += 1;
                return;
            }
            Activity::Locked => return self.idle(bus),
            Activity::Stopped => self.activity = Activity::Running,
            Activity::Running | Activity::Halted => {}
        }
        // Hardware samples interrupts during the opcode fetch, then discards the opcode.
        let pending_before = bus.pending_interrupts();
        let op = self.read(self.regs.pc, bus);
        let pending_after = bus.pending_interrupts();
        if self.activity == Activity::Halted {
            // HALT keeps fetching, so the cycle that wakes it is already the fetch, but it
            // samples earlier in it than a running CPU does.
            if pending_before | (pending_after & bus.early_requests()) == 0 {
                return;
            }
            self.activity = Activity::Running;
        }
        let pending = pending_after != 0;
        if self.ime && pending {
            return self.dispatch_interrupt(bus);
        }
        self.enable_delay = match self.enable_delay {
            EnableDelay::Requested => {
                self.ime = true;
                EnableDelay::Applied
            }
            _ => EnableDelay::Idle,
        };
        self.advance_pc();
        self.execute(op, bus);
    }

    fn dispatch_interrupt(&mut self, bus: &mut Motherboard) {
        self.ime = false;
        self.enable_delay = EnableDelay::Idle;
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
        let pending = bus.pending_interrupts() != 0;
        if pending && self.enable_delay == EnableDelay::Applied {
            // Hardware quirk: the handler of EI + HALT returns to the HALT itself.
            self.regs.pc = self.regs.pc.wrapping_sub(1);
        } else if pending && !self.ime {
            self.halt_bug = true;
        } else {
            self.activity = Activity::Halted;
        }
    }

    fn stop(&mut self, bus: &mut Motherboard) {
        self.regs.pc = self.regs.pc.wrapping_add(1);
        bus.timer.reset_divider();
        self.activity = Activity::Stopped;
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
