# RustyGB 🦀

**RustyGB** is a Game Boy (DMG) emulator written entirely in **Rust** from scratch.

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
* **CPU (SM83):** Full instruction set; every memory access advances the machine by one M-cycle.
* **PPU (Graphics):**
    * Scanline renderer (160x144) with background, window and sprites (8x8 & 8x16), including the 10-sprite limit and DMG priority.
    * VBlank and STAT interrupts.
* **APU (Audio):** Square 1 (sweep), Square 2, Wave and Noise with envelopes and length counters, mixed to mono.
* **Timer:** DIV, TIMA, TMA with "falling edge" behavior and delayed reload.
* **OAM DMA**, HALT bug and STOP.

### Accuracy
Passes blargg `cpu_instrs`, `instr_timing`, `mem_timing` and `halt_bug`, and `dmg-acid2`.
Known limits: mid-line register changes, variable mode 3 length, VRAM/OAM blocking
by the PPU, T-cycle wave RAM access, the DMG OAM bug and DMA bus conflicts are not modelled.

### Cartridge Support
* **ROM Only:** 32KB games.
* **MBC1:** Bank switching logic.
* **MBC3:** Bank switching (no real-time clock).
* **MBC5:** Modern Game Boy games support.
* **SRAM:** Battery-backed saves as raw `.sav` dumps, written atomically on exit, on ROM change and every ~10 s. An RTC footer from another emulator is preserved; a save of any other size is refused, never overwritten.

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
wasm-pack build wasm --release --target web --out-dir ../pkg --out-name rusty_gb
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
├── main.rs       Desktop entry point
├── native/       Desktop front-end (window, audio, pacing, saves)
├── cpu/          Sharp SM83 CPU
├── bus/          System bus and OAM DMA
├── ppu/          Pixel Processing Unit
├── apu/          Audio Processing Unit
├── cartridge/    ROM-only / MBC1 / MBC3 / MBC5 mappers
├── save.rs       Crash-safe save files
├── serial.rs     Serial port (no link partner)
├── joypad.rs     Backend-agnostic joypad
└── timer.rs      DIV/TIMA/TMA/TAC
wasm/             wasm-bindgen JS bindings
web/              Browser front-end for the WebAssembly build
```