//! DMG timer block (DIV, TIMA, TMA, TAC).
//!
//! The timer counts T-cycles from the system clock and exposes four
//! registers at `0xFF04-0xFF07`. TIMA increments on the falling edge of a
//! selected DIV bit; an overflow latches TMA back into TIMA and raises the
//! timer interrupt one M-cycle later — a quirk that *Tetris* relies on to
//! seed its RNG.

pub struct Timer {
    /// 16-bit free-running counter. The CPU only sees its upper byte
    /// through `0xFF04`, but the lower bits drive the TIMA multiplexer.
    pub div: u16,
    tima: u8,
    tma: u8,
    tac: u8,

    /// Cached state of the previous tick's multiplexer output, used to
    /// detect the falling edge that drives TIMA.
    last_bit: bool,

    /// One-cycle delay buffer: when TIMA overflows the reload to TMA and
    /// the interrupt request are postponed by a single M-cycle.
    overflow: bool,
}

impl Timer {
    pub fn new() -> Self {
        Self {
            div: 0,
            tima: 0,
            tma: 0,
            tac: 0,
            last_bit: false,
            overflow: false,
        }
    }

    /// Advance the timer by `cycles` T-cycles. Sets bit 2 of `if_reg` when
    /// TIMA overflows (after the hardware-accurate one-cycle delay).
    pub fn tick(&mut self, cycles: u8, if_reg: &mut u8) {
        let inc = cycles as u16;

        if self.overflow {
            self.tima = self.tma;
            *if_reg |= 0x04;
            self.overflow = false;
        }

        self.div = self.div.wrapping_add(inc);

        // TAC bits 0-1 pick which DIV bit gates the TIMA increment:
        // 00 → bit 9, 01 → bit 3, 10 → bit 5, 11 → bit 7.
        let bit_idx = match self.tac & 0x03 {
            0 => 9,
            1 => 3,
            2 => 5,
            _ => 7,
        };

        let timer_enabled = (self.tac & 0x04) != 0;
        let new_bit = timer_enabled && ((self.div >> bit_idx) & 1) != 0;

        if self.last_bit && !new_bit {
            let (result, overflowed) = self.tima.overflowing_add(1);
            self.tima = result;
            if overflowed {
                self.overflow = true;
            }
        }
        self.last_bit = new_bit;
    }

    /// Read a timer register. Unmapped addresses return `0xFF`.
    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0xFF04 => (self.div >> 8) as u8,
            0xFF05 => self.tima,
            0xFF06 => self.tma,
            0xFF07 => self.tac | 0xF8,
            _ => 0xFF,
        }
    }

    /// Write to a timer register. Any write to `DIV` (`0xFF04`) resets the
    /// entire 16-bit counter back to zero, as on real hardware.
    pub fn write(&mut self, addr: u16, val: u8) {
        match addr {
            0xFF04 => self.div = 0,
            0xFF05 => self.tima = val,
            0xFF06 => self.tma = val,
            0xFF07 => self.tac = val,
            _ => {}
        }
    }
}

impl Default for Timer {
    fn default() -> Self {
        Self::new()
    }
}
