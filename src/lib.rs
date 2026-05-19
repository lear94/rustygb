//! # RustyGB
//!
//! A portable, dependency-light Game Boy (DMG) emulator core written in
//! Rust. The same crate is used by:
//!
//! * the native binary defined in `src/main.rs` (winit + pixels + cpal)
//! * the WebAssembly front-end built with `wasm-pack` (see [`wasm`])
//! * downstream tests and tooling that want to drive the emulator
//!   programmatically
//!
//! ## Quick start
//!
//! ```no_run
//! use rusty_gb::{Button, GameBoy};
//!
//! let rom = std::fs::read("game.gb").unwrap();
//! let mut gb = GameBoy::from_rom(rom, /* sample_rate */ 48_000).unwrap();
//!
//! gb.set_button(Button::Start, true);
//! gb.run_frame();           // run 70_224 T-cycles ≈ 1 frame
//! let pixels = gb.frame();  // 160×144 RGBA
//! ```
//!
//! The core is intentionally pull-based: the host calls [`GameBoy::run_frame`]
//! at its preferred cadence, then drains [`GameBoy::frame`] and
//! [`GameBoy::drain_audio`] to display video and play audio.

pub mod apu;
pub mod bus;
pub mod cartridge;
pub mod cpu;
pub mod joypad;
pub mod ppu;
pub mod timer;

#[cfg(target_arch = "wasm32")]
pub mod wasm;

pub use joypad::Button;
pub use ppu::{SCREEN_HEIGHT, SCREEN_WIDTH};

use anyhow::Result;

use crate::bus::Motherboard;
use crate::cartridge::from_bytes;
use crate::cpu::Cpu;

/// Number of T-cycles in one full DMG frame (≈ 59.73 Hz refresh).
pub const CYCLES_PER_FRAME: u32 = 70_224;

/// High-level emulator wrapper that ties a [`Cpu`] to its [`Motherboard`].
///
/// This is the recommended entry point for embedders that don't need to
/// pump the CPU and the bus separately.
pub struct GameBoy {
    cpu: Cpu,
    bus: Motherboard,
}

impl GameBoy {
    /// Build an emulator from a raw ROM image. `sample_rate` is the APU
    /// output rate in Hz (typically the audio device's native rate).
    pub fn from_rom(rom: Vec<u8>, sample_rate: u32) -> Result<Self> {
        let cart = from_bytes(rom, None)?;
        Self::with_cartridge(cart, sample_rate)
    }

    /// Build an emulator from a ROM and a previously saved SRAM image.
    pub fn from_rom_with_save(
        rom: Vec<u8>,
        saved_ram: Option<Vec<u8>>,
        sample_rate: u32,
    ) -> Result<Self> {
        let cart = from_bytes(rom, saved_ram)?;
        Self::with_cartridge(cart, sample_rate)
    }

    fn with_cartridge(cart: Box<dyn cartridge::Cartridge>, sample_rate: u32) -> Result<Self> {
        let bus = Motherboard::new(cart, sample_rate);
        let mut cpu = Cpu::new();
        cpu.reset_to_boot();
        Ok(Self { cpu, bus })
    }

    /// Run the CPU for one full frame's worth of T-cycles.
    pub fn run_frame(&mut self) {
        let mut cycles: u32 = 0;
        while cycles < CYCLES_PER_FRAME {
            let step = self.cpu.step(&mut self.bus) as u32;
            self.bus.tick(step as u8);
            cycles += step;
        }
    }

    /// Borrow the current 160×144 RGBA frame buffer.
    pub fn frame(&self) -> &[u8] {
        &self.bus.ppu.buffer
    }

    /// Drain up to `out.len()` mixed audio samples into `out` and return
    /// how many were written.
    pub fn drain_audio(&mut self, out: &mut [f32]) -> usize {
        self.bus.apu.drain_samples(out)
    }

    /// Number of audio samples currently buffered by the APU.
    pub fn buffered_audio(&self) -> usize {
        self.bus.apu.buffered_samples()
    }

    /// Set the pressed/released state of a single button.
    pub fn set_button(&mut self, button: Button, pressed: bool) {
        self.bus.joypad.set_button(button, pressed);
    }

    /// Persist battery-backed SRAM next to `<rom_path>.sav`.
    pub fn save_external_ram(&self, rom_path: &str) -> Result<()> {
        self.bus.save_external_ram(rom_path)
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
