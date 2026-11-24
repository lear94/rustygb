//! Keyboard state mapped to buttons.

use rusty_gb::Button;
use winit::event::{ElementState, KeyboardInput, VirtualKeyCode};

const KEY_MAP: [(VirtualKeyCode, Button); BUTTON_COUNT] = [
    (VirtualKeyCode::Z, Button::A),
    (VirtualKeyCode::X, Button::B),
    (VirtualKeyCode::Space, Button::Select),
    (VirtualKeyCode::Return, Button::Start),
    (VirtualKeyCode::Right, Button::Right),
    (VirtualKeyCode::Left, Button::Left),
    (VirtualKeyCode::Up, Button::Up),
    (VirtualKeyCode::Down, Button::Down),
];
const BUTTON_COUNT: usize = 8;

/// Held buttons, plus taps too short to span a frame.
#[derive(Debug, Default)]
pub(super) struct Keys {
    held: [bool; BUTTON_COUNT],
    tapped: [bool; BUTTON_COUNT],
}

impl Keys {
    pub(super) fn update(&mut self, input: KeyboardInput) {
        let pressed = input.state == ElementState::Pressed;
        let slots = self.held.iter_mut().zip(&mut self.tapped).zip(KEY_MAP);
        for ((held, tapped), (key, _)) in slots {
            if input.virtual_keycode == Some(key) {
                *held = pressed;
                *tapped |= pressed;
            }
        }
    }

    /// Releases every key, as when the window loses focus.
    pub(super) fn release_all(&mut self) {
        *self = Self::default();
    }

    /// Returns the buttons for the next frame; every tap lasts at least one.
    pub(super) fn next_frame(&mut self) -> [(Button, bool); BUTTON_COUNT] {
        let mut slots = self.held.iter().zip(std::mem::take(&mut self.tapped));
        KEY_MAP.map(|(_, button)| {
            let pressed = slots.next().is_some_and(|(&held, tapped)| held || tapped);
            (button, pressed)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(keys: &mut Keys, code: VirtualKeyCode, state: ElementState) {
        #[allow(deprecated, reason = "winit offers no other constructor")]
        keys.update(KeyboardInput {
            scancode: 0,
            state,
            virtual_keycode: Some(code),
            modifiers: winit::event::ModifiersState::empty(),
        });
    }

    fn pressed(keys: &mut Keys) -> Vec<Button> {
        let frame = keys.next_frame().into_iter();
        frame.filter(|(_, on)| *on).map(|(b, _)| b).collect()
    }

    #[test]
    fn next_frame_holds_until_release() {
        let mut keys = Keys::default();
        key(&mut keys, VirtualKeyCode::Up, ElementState::Pressed);
        assert_eq!(pressed(&mut keys), [Button::Up]);
        assert_eq!(pressed(&mut keys), [Button::Up]);
        key(&mut keys, VirtualKeyCode::Up, ElementState::Released);
        assert_eq!(pressed(&mut keys), []);
    }

    #[test]
    fn next_frame_short_tap_lasts_one_frame() {
        let mut keys = Keys::default();
        key(&mut keys, VirtualKeyCode::Z, ElementState::Pressed);
        key(&mut keys, VirtualKeyCode::Z, ElementState::Released);
        assert_eq!(pressed(&mut keys), [Button::A]);
        assert_eq!(pressed(&mut keys), []);
    }

    #[test]
    fn release_all_clears_held_keys() {
        let mut keys = Keys::default();
        key(&mut keys, VirtualKeyCode::X, ElementState::Pressed);
        keys.release_all();
        assert_eq!(pressed(&mut keys), []);
    }

    #[test]
    fn update_unmapped_key_is_ignored() {
        let mut keys = Keys::default();
        key(&mut keys, VirtualKeyCode::Q, ElementState::Pressed);
        assert_eq!(pressed(&mut keys), []);
    }
}
