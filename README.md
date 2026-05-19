# RustyGB 🦀

**RustyGB** is a high-performance, cycle-accurate Game Boy (DMG) emulator written entirely in **Rust** from scratch.

This project is a **clean-room implementation** created solely for **educational purposes** to study hardware architecture and emulation development.

> **⚠️ IMPORTANT:** This emulator **does not** contain any Nintendo proprietary code, BIOS, firmware, or copyrighted assets. It mimics the hardware behavior through original code written in Rust.

## ⚖️ Legal Disclaimer & Anti-Piracy Policy

**Please read carefully before using:**

1.  **No Nintendo Code:** No code from the original Game Boy BIOS or official Nintendo software has been used, reverse-engineered from leaked sources, or included in this repository. All logic is based on public hardware documentation and independent research.
2.  **No ROMs Provided:** This repository **does not and will never** provide, distribute, or link to commercial ROM files (game images).
3.  **Use Legally Owned Games:** Users are expected to dump their own cartridges from games they legally possess. We do not condone piracy.
4.  **Trademarks:** "Game Boy", "Pokemon", "Tetris", and "Zelda" are trademarks of their respective owners (Nintendo Co., Ltd., The Tetris Company, etc.). This project is not affiliated with, endorsed by, or sponsored by Nintendo.

---

## Features 🚀

### Core Emulation
* **CPU (SM83):** Full implementation of the instruction set, built from scratch with cycle-accurate timing.
* **PPU (Graphics):**
    * Pixel-perfect rendering (160x144).
    * Supports Background map, Window overlay, and Sprites (8x8 & 8x16).
    * Precise VBlank/HBlank timing and LCD status interrupts.
* **APU (Audio):**
    * 4-Channel Audio: Square 1 (Sweep), Square 2, Wave, and Noise.
    * Full volume envelope and frequency sweep implementation.
    * Audio sync algorithm to prevent crackling on modern hardware.
* **Timer:** Accurate DIV, TIMA, TMA implementation including "falling edge" behavior (critical for *Tetris* RNG).

### Cartridge Support
* **ROM Only:** 32KB games.
* **MBC1:** Bank switching logic.
* **MBC3:** Advanced banking.
* **MBC5:** Modern Game Boy games support.
* **SRAM:** Battery-backed saves (`.sav` files are compatible with other major emulators).

---

## Installation & Usage 🎮

### Prerequisites
You need to have **Rust** installed. If you don't have it, get it at [rustup.rs](https://rustup.rs/).

On Linux you also need ALSA development headers for the audio backend:

```bash
sudo apt install pkg-config libasound2-dev
```

### Building the native binary
```bash
git clone https://github.com/lear94/rustygb.git
cd rustygb
cargo build --release
./target/release/rusty_gb path/to/game.gb
```

You can also drag-and-drop a `.gb` file onto the running window.

### Building the WebAssembly version

The same Rust core is also exposed as a `wasm-bindgen` module so the
emulator runs in any modern browser. Install
[`wasm-pack`](https://rustwasm.github.io/wasm-pack/) and build with:

```bash
wasm-pack build --release --target web --out-dir pkg
```

Then serve the repository root with any static HTTP server and open
`/web/` in the browser:

```bash
python3 -m http.server 8000
# → http://localhost:8000/web/
```

See [`web/README.md`](web/README.md) for the full WebAssembly workflow.

## Project layout

```
src/
├── lib.rs        High-level GameBoy wrapper + module re-exports
├── main.rs       Native (winit + pixels + cpal) front-end
├── wasm.rs       wasm-bindgen JS bindings (compiled only on wasm32)
├── cpu.rs        Sharp SM83 CPU
├── bus.rs        System bus / motherboard
├── ppu.rs        Pixel Processing Unit
├── apu.rs        Audio Processing Unit
├── cartridge.rs  ROM-only / MBC1 / MBC3 / MBC5 mappers
├── joypad.rs     Backend-agnostic joypad
└── timer.rs      DIV/TIMA/TMA/TAC
web/              Browser front-end for the WebAssembly build
```