//! WebAssembly bindings.
//!
//! Compiled only for `wasm32-*` targets, this module exposes a thin
//! `wasm-bindgen` wrapper around [`crate::GameBoy`] so the emulator can be
//! driven from JavaScript:
//!
//! ```js
//! import init, { WasmGameBoy, WasmButton } from "./pkg/rusty_gb.js";
//!
//! await init();
//! const gb = WasmGameBoy.new(romBytes, 48000);
//! gb.set_button(WasmButton.Start, true);
//! gb.run_frame();
//! ctx.putImageData(new ImageData(new Uint8ClampedArray(gb.frame()), 160, 144), 0, 0);
//! ```

use wasm_bindgen::prelude::*;

use crate::{Button, GameBoy};

/// JS-facing mirror of [`crate::Button`]. `wasm-bindgen` does not support
/// re-exporting external enums directly, so we duplicate the variants and
/// translate at the boundary.
#[wasm_bindgen]
#[derive(Clone, Copy)]
pub enum WasmButton {
    Right,
    Left,
    Up,
    Down,
    A,
    B,
    Select,
    Start,
}

impl From<WasmButton> for Button {
    fn from(b: WasmButton) -> Self {
        match b {
            WasmButton::Right => Button::Right,
            WasmButton::Left => Button::Left,
            WasmButton::Up => Button::Up,
            WasmButton::Down => Button::Down,
            WasmButton::A => Button::A,
            WasmButton::B => Button::B,
            WasmButton::Select => Button::Select,
            WasmButton::Start => Button::Start,
        }
    }
}

/// Install the better panic hook so Rust panics show up in the browser
/// console instead of the opaque "unreachable executed" trap. Safe to
/// call more than once.
#[wasm_bindgen(start)]
pub fn _start() {
    console_error_panic_hook::set_once();
}

/// Game Boy emulator instance owned by JavaScript.
#[wasm_bindgen]
pub struct WasmGameBoy {
    inner: GameBoy,
}

#[wasm_bindgen]
impl WasmGameBoy {
    /// Build a new emulator from a ROM byte array and an audio sample rate.
    #[wasm_bindgen(constructor)]
    pub fn new(rom: Vec<u8>, sample_rate: u32) -> Result<WasmGameBoy, JsValue> {
        GameBoy::from_rom(rom, sample_rate)
            .map(|gb| WasmGameBoy { inner: gb })
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Run the CPU for one full DMG frame (~70 224 T-cycles).
    pub fn run_frame(&mut self) {
        self.inner.run_frame();
    }

    /// Get the current 160×144 RGBA8 frame as a freshly-allocated `Uint8Array`.
    pub fn frame(&self) -> Vec<u8> {
        self.inner.frame().to_vec()
    }

    /// Drain up to `max` queued audio samples and return them. An empty
    /// vector is returned when no samples are available.
    pub fn drain_audio(&mut self, max: usize) -> Vec<f32> {
        let mut buf = vec![0.0; max];
        let n = self.inner.drain_audio(&mut buf);
        buf.truncate(n);
        buf
    }

    /// Number of audio samples currently buffered.
    pub fn buffered_audio(&self) -> usize {
        self.inner.buffered_audio()
    }

    /// Set the pressed/released state of a single button.
    pub fn set_button(&mut self, button: WasmButton, pressed: bool) {
        self.inner.set_button(button.into(), pressed);
    }
}
