//! Joypad (P1/JOYP) emulation.
//!
//! The Game Boy exposes its eight physical buttons through a single
//! multiplexed register at `0xFF00`. Two column-select lines pick whether
//! the four returned bits report the direction pad or the action buttons.
//! Pressed inputs read as `0`, released inputs as `1`.
//!
//! This module is intentionally free of any front-end dependency: callers
//! drive it through [`Joypad::set_button`], which makes the same core usable
//! by the native binary, a WebAssembly build, or unit tests.

/// Logical Game Boy button. The variant order is not significant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Right,
    Left,
    Up,
    Down,
    A,
    B,
    Select,
    Start,
}

/// Joypad register state. Owns the current pressed/released mask for
/// each of the two button rows and tracks whether a high-to-low edge
/// must raise the joypad interrupt on the next bus tick.
pub struct Joypad {
    /// Active-low button masks. Index 0 is the action row (A/B/Select/Start),
    /// index 1 is the direction row (Right/Left/Up/Down).
    rows: [u8; 2],
    /// Mirror of the column-select bits last written by the CPU.
    column_select: u8,
    /// Set when any button transitions from released to pressed.
    /// Consumed (and cleared) by the bus during its next tick.
    pub interrupt: bool,
}

impl Joypad {
    /// Build a joypad with every button released.
    pub fn new() -> Self {
        Self {
            rows: [0x0F, 0x0F],
            column_select: 0,
            interrupt: false,
        }
    }

    /// Update the internal mask for a single button.
    ///
    /// When `pressed` is `true` the matching bit is cleared (active-low).
    /// A new press (released → pressed transition) latches the joypad
    /// interrupt request so the bus can forward it to the CPU.
    pub fn set_button(&mut self, button: Button, pressed: bool) {
        let (row, mask) = match button {
            Button::A => (0, 0x01),
            Button::B => (0, 0x02),
            Button::Select => (0, 0x04),
            Button::Start => (0, 0x08),
            Button::Right => (1, 0x01),
            Button::Left => (1, 0x02),
            Button::Up => (1, 0x04),
            Button::Down => (1, 0x08),
        };

        let previous = self.rows[row];
        if pressed {
            self.rows[row] &= !mask;
        } else {
            self.rows[row] |= mask;
        }

        let newly_pressed = (previous & mask) != 0 && (self.rows[row] & mask) == 0;
        if newly_pressed {
            self.interrupt = true;
        }
    }

    /// Read the multiplexed P1 register as the CPU would see it.
    /// Bits 6 and 7 are unused and always read as `1`.
    pub fn read(&self) -> u8 {
        let mut value = 0xCF;
        if (self.column_select & 0x10) == 0 {
            value &= self.rows[1];
        }
        if (self.column_select & 0x20) == 0 {
            value &= self.rows[0];
        }
        value
    }

    /// CPU write to P1: only the two column-select bits are writable.
    pub fn write(&mut self, value: u8) {
        self.column_select = value & 0x30;
    }
}

impl Default for Joypad {
    fn default() -> Self {
        Self::new()
    }
}
