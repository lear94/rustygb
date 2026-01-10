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

### Building from Source
```bash
git clone [https://github.com/lear94/rustygb.git](https://github.com/lear94/rustygb.git)
cd rustygb
cargo build --release