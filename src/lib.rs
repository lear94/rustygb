//! Portable Game Boy (DMG) emulator core.
//!
//! The same crate is used by:
//!
//! * the native binary defined in `src/main.rs` (winit + pixels + cpal)
//! * the WebAssembly front-end built with `wasm-pack` (see `wasm/`)
//! * downstream tests and tooling that want to drive the emulator
//!   programmatically
//!
//! ## Quick start
//!
//! ```no_run
//! use rusty_gb::{Button, GameBoy};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let rom = std::fs::read("game.gb")?;
//! let mut gb = GameBoy::from_rom(rom, /* sample_rate */ 48_000)?;
//!
//! gb.set_button(Button::Start, true);
//! gb.run_frame();           // run 70_224 T-cycles ≈ 1 frame
//! let pixels = gb.frame();  // 160×144 RGBA
//! # Ok(())
//! # }
//! ```
//!
//! The core is intentionally pull-based: the host calls [`GameBoy::run_frame`]
//! at its preferred cadence, then drains [`GameBoy::frame`] and
//! [`GameBoy::drain_audio`] to display video and play audio.

pub mod apu;
pub mod bus;
pub mod cartridge;
pub mod cpu;
mod hardware;
pub mod joypad;
pub mod ppu;
#[cfg(not(target_arch = "wasm32"))]
pub mod save;
pub mod serial;
pub mod timer;

pub use cartridge::CartridgeError;
pub use hardware::{CLOCK_HZ, CYCLES_PER_FRAME};
pub use joypad::Button;
pub use ppu::{SCREEN_HEIGHT, SCREEN_WIDTH};

use crate::bus::Motherboard;
use crate::cartridge::Cartridge;
use crate::cpu::Cpu;

/// High-level emulator wrapper that ties a [`Cpu`] to its [`Motherboard`].
///
/// This is the recommended entry point for embedders that don't need to
/// pump the CPU and the bus separately.
pub struct GameBoy {
    cpu: Cpu,
    bus: Motherboard,
    /// T-cycles the last frame ran over its budget.
    overshoot: u32,
}

impl GameBoy {
    /// Build an emulator from a raw ROM image. `sample_rate` is the APU
    /// output rate in Hz (typically the audio device's native rate).
    ///
    /// # Errors
    /// Same as [`Cartridge::from_bytes`].
    pub fn from_rom(rom: Vec<u8>, sample_rate: u32) -> Result<Self, CartridgeError> {
        Self::from_rom_with_save(rom, None, sample_rate)
    }

    /// Build an emulator from a ROM and a previously saved SRAM image.
    ///
    /// # Errors
    /// Same as [`Cartridge::from_bytes`].
    pub fn from_rom_with_save(
        rom: Vec<u8>,
        saved_ram: Option<Vec<u8>>,
        sample_rate: u32,
    ) -> Result<Self, CartridgeError> {
        let cartridge = Cartridge::from_bytes(rom, saved_ram)?;
        Ok(Self {
            cpu: Cpu::new(),
            bus: Motherboard::new(cartridge, sample_rate),
            overshoot: 0,
        })
    }

    /// Run the CPU for one full frame's worth of T-cycles.
    pub fn run_frame(&mut self) {
        let mut cycles = self.overshoot;
        while cycles < CYCLES_PER_FRAME {
            cycles += self.cpu.step(&mut self.bus);
        }
        // Carried over, or video and audio would drift.
        self.overshoot = cycles - CYCLES_PER_FRAME;
    }

    /// Borrow the current 160×144 RGBA frame buffer.
    #[must_use]
    pub fn frame(&self) -> &[u8] {
        self.bus.ppu.frame()
    }

    /// Drain up to `out.len()` mixed audio samples into `out` and return
    /// how many were written.
    pub fn drain_audio(&mut self, out: &mut [f32]) -> usize {
        self.bus.apu.drain_samples(out)
    }

    /// Number of audio samples currently buffered by the APU.
    #[must_use]
    pub fn buffered_audio(&self) -> usize {
        self.bus.apu.buffered_samples()
    }

    /// Set the pressed/released state of a single button.
    pub fn set_button(&mut self, button: Button, pressed: bool) {
        self.bus.joypad.set_button(button, pressed);
    }

    /// Borrow the inserted cartridge, e.g. to persist its battery RAM.
    pub fn cartridge(&mut self) -> &mut Cartridge {
        self.bus.cartridge()
    }

    /// Direct access to the underlying motherboard (advanced use).
    pub fn bus(&mut self) -> &mut Motherboard {
        &mut self.bus
    }

    /// Direct access to the underlying CPU (advanced use).
    pub fn cpu(&mut self) -> &mut Cpu {
        &mut self.cpu
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Machine running `program` at the entry point.
    fn game(program: &[u8]) -> GameBoy {
        let mut rom = vec![0; 0x8000];
        rom[0x100..0x100 + program.len()].copy_from_slice(program);
        GameBoy::from_rom(rom, 48_000).expect("valid ROM")
    }

    #[test]
    fn run_frame_long_run_does_not_drift() {
        let mut gb = game(&[0xCD, 0x05, 0x01, 0x18, 0xFB, 0xC9]);
        let mut scratch = vec![0.0; 8192];
        let samples: usize = (0..600)
            .map(|_| {
                gb.run_frame();
                gb.drain_audio(&mut scratch)
            })
            .sum();
        assert_eq!(samples, 600 * 70_224 * 48_000 / 4_194_304);
    }

    #[test]
    fn run_frame_stopped_machine_returns() {
        let mut gb = game(&[0x10, 0x00]);
        gb.run_frame();
        assert!(gb.buffered_audio() > 700);
    }

    #[test]
    fn set_button_reaches_joypad() {
        let mut gb = game(&[0x18, 0xFE]);
        gb.set_button(Button::A, true);
        assert_eq!(gb.bus().read_byte(0xFF00) & 0x0F, 0x0E);
    }

    #[test]
    fn from_rom_invalid_is_error() {
        assert!(GameBoy::from_rom(Vec::new(), 48_000).is_err());
    }
}
