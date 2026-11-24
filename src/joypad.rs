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

/// Active-low input lines.
const LINES_MASK: u8 = 0x0F;
const SELECT_DIRECTIONS: u8 = 0x10;
const SELECT_ACTIONS: u8 = 0x20;
/// Bits that do not exist read as 1.
const UNUSED_BITS: u8 = 0xC0;

/// Logical Game Boy button. The variant order is not significant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    /// D-pad right.
    Right,
    /// D-pad left.
    Left,
    /// D-pad up.
    Up,
    /// D-pad down.
    Down,
    /// A button.
    A,
    /// B button.
    B,
    /// Select button.
    Select,
    /// Start button.
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
    /// Creates a joypad with every button released.
    #[must_use]
    pub fn new() -> Self {
        Self {
            rows: [LINES_MASK; 2],
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
        let [actions, directions] = &mut self.rows;
        let (row, mask) = match button {
            Button::A => (actions, 0x01),
            Button::B => (actions, 0x02),
            Button::Select => (actions, 0x04),
            Button::Start => (actions, 0x08),
            Button::Right => (directions, 0x01),
            Button::Left => (directions, 0x02),
            Button::Up => (directions, 0x04),
            Button::Down => (directions, 0x08),
        };
        let was_released = *row & mask != 0;
        if pressed {
            *row &= !mask;
        } else {
            *row |= mask;
        }
        if pressed && was_released {
            self.interrupt = true;
        }
    }

    /// Reads P1: unused bits are 1 and the column-select bits read back.
    #[must_use]
    pub fn read(&self) -> u8 {
        let [actions, directions] = self.rows;
        let mut lines = LINES_MASK;
        if self.column_select & SELECT_DIRECTIONS == 0 {
            lines &= directions;
        }
        if self.column_select & SELECT_ACTIONS == 0 {
            lines &= actions;
        }
        UNUSED_BITS | self.column_select | lines
    }

    /// Returns whether a pressed button pulls one of the selected lines low.
    #[must_use]
    pub fn any_selected_pressed(&self) -> bool {
        self.read() & LINES_MASK != LINES_MASK
    }

    /// CPU write to P1: only the two column-select bits are writable.
    pub fn write(&mut self, value: u8) {
        self.column_select = value & (SELECT_ACTIONS | SELECT_DIRECTIONS);
    }
}

impl Default for Joypad {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Button; 8] = [
        Button::A,
        Button::B,
        Button::Select,
        Button::Start,
        Button::Right,
        Button::Left,
        Button::Up,
        Button::Down,
    ];

    #[test]
    fn read_reports_select_and_unused_bits() {
        for (select, expected) in [(0x00, 0xCF), (0x10, 0xDF), (0x20, 0xEF), (0xFF, 0xFF)] {
            let mut joypad = Joypad::new();
            joypad.write(select);
            assert_eq!(joypad.read(), expected);
        }
    }

    #[test]
    fn set_button_pulls_line_of_selected_row() {
        for (idx, button) in ALL.into_iter().enumerate() {
            let mut joypad = Joypad::new();
            joypad.set_button(button, true);
            let (selected, other) = if idx < 4 { (0x10, 0x20) } else { (0x20, 0x10) };
            joypad.write(selected);
            assert_eq!(joypad.read() & 0x0F, 0x0F & !(1 << (idx % 4)), "{button:?}");
            assert!(joypad.any_selected_pressed());
            joypad.write(other);
            assert!(!joypad.any_selected_pressed());
            joypad.set_button(button, false);
            joypad.write(selected);
            assert!(!joypad.any_selected_pressed());
        }
    }

    #[test]
    fn set_button_interrupts_on_press_edge_only() {
        let mut joypad = Joypad::new();
        for (pressed, expected) in [(false, false), (true, true), (true, false), (false, false)] {
            joypad.set_button(Button::A, pressed);
            assert_eq!(std::mem::take(&mut joypad.interrupt), expected);
        }
    }
}
