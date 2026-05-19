//! System bus / "motherboard".
//!
//! Owns every peripheral attached to the SM83 CPU (cartridge, PPU, APU,
//! timer, joypad, internal RAM) and routes every memory access through the
//! DMG memory map:
//!
//! | Range             | Target                       |
//! |-------------------|------------------------------|
//! | `0x0000-0x7FFF`   | Cartridge ROM (banked)       |
//! | `0x8000-0x9FFF`   | PPU VRAM                     |
//! | `0xA000-0xBFFF`   | Cartridge external RAM       |
//! | `0xC000-0xDFFF`   | Work RAM                     |
//! | `0xE000-0xFDFF`   | Echo RAM (mirror of WRAM)    |
//! | `0xFE00-0xFE9F`   | OAM                          |
//! | `0xFF00`          | Joypad                       |
//! | `0xFF01-0xFF02`   | Serial (stubbed)             |
//! | `0xFF04-0xFF07`   | Timer                        |
//! | `0xFF0F`          | Interrupt flag (IF)          |
//! | `0xFF10-0xFF3F`   | APU                          |
//! | `0xFF40-0xFF4B`   | PPU registers + DMA          |
//! | `0xFF80-0xFFFE`   | High RAM                     |
//! | `0xFFFF`          | Interrupt enable (IE)        |

use crate::apu::Apu;
use crate::cartridge::Cartridge;
use crate::joypad::Joypad;
use crate::ppu::Ppu;
use crate::timer::Timer;
use anyhow::Result;

const WRAM_SIZE: usize = 8 * 1024;
const HRAM_SIZE: usize = 127;

/// Top-level emulated machine. Holds every peripheral and arbitrates the
/// CPU's memory traffic.
pub struct Motherboard {
    cartridge: Box<dyn Cartridge>,
    pub ppu: Ppu,
    pub apu: Apu,
    pub timer: Timer,
    pub joypad: Joypad,

    wram: [u8; WRAM_SIZE],
    hram: [u8; HRAM_SIZE],

    ie: u8,
    if_reg: u8,

    serial_sb: u8,
    serial_sc: u8,
}

impl Motherboard {
    /// Build a fresh motherboard around the provided cartridge. The audio
    /// sample rate selects the APU output frequency in Hz.
    pub fn new(cartridge: Box<dyn Cartridge>, sample_rate: u32) -> Self {
        Self {
            cartridge,
            ppu: Ppu::new(),
            apu: Apu::new(sample_rate),
            timer: Timer::new(),
            joypad: Joypad::new(),
            wram: [0; WRAM_SIZE],
            hram: [0; HRAM_SIZE],
            ie: 0,
            // Bits 5-7 of IF are always high on real hardware.
            if_reg: 0xE1,
            serial_sb: 0,
            serial_sc: 0,
        }
    }

    /// Advance every peripheral by `cycles` T-cycles. Called by the CPU
    /// after each instruction executes.
    pub fn tick(&mut self, cycles: u8) {
        self.timer.tick(cycles, &mut self.if_reg);
        self.ppu.tick(cycles, &mut self.if_reg);
        self.apu.tick(cycles);

        // Serial port stub: many titles (notably Tetris) wait for the
        // transfer-complete flag (SC bit 7) to clear before proceeding.
        // Simulate an instantaneous transfer and raise the serial IRQ.
        if (self.serial_sc & 0x80) != 0 {
            self.serial_sc &= 0x7F;
            self.if_reg |= 0x08;
        }

        if self.joypad.interrupt {
            self.if_reg |= 0x10;
            self.joypad.interrupt = false;
        }
    }

    /// Persist the cartridge's external RAM to `<base_path>.sav`.
    pub fn save_external_ram(&self, base_path: &str) -> Result<()> {
        let save_path = std::path::Path::new(base_path).with_extension("sav");
        self.cartridge.save_state(save_path.to_str().unwrap())
    }

    /// CPU-side byte read. Unmapped addresses return open-bus (`0xFF`).
    pub fn read_byte(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x7FFF => self.cartridge.read(addr),
            0x8000..=0x9FFF => self.ppu.read(addr),
            0xA000..=0xBFFF => self.cartridge.read(addr),
            0xC000..=0xDFFF => self.wram[(addr - 0xC000) as usize],
            0xE000..=0xFDFF => self.wram[(addr - 0xE000) as usize],
            0xFE00..=0xFE9F => self.ppu.read(addr),
            0xFF00 => self.joypad.read(),
            0xFF01 => self.serial_sb,
            0xFF02 => self.serial_sc | 0x7E,
            0xFF04..=0xFF07 => self.timer.read(addr),
            0xFF0F => self.if_reg | 0xE0,
            0xFF10..=0xFF3F => self.apu.read(addr),
            0xFF40..=0xFF4B => self.ppu.read(addr),
            0xFF80..=0xFFFE => self.hram[(addr - 0xFF80) as usize],
            0xFFFF => self.ie,
            _ => 0xFF,
        }
    }

    /// CPU-side byte write. Writes to unmapped regions are silently dropped.
    pub fn write_byte(&mut self, addr: u16, val: u8) {
        match addr {
            0x0000..=0x7FFF => self.cartridge.write(addr, val),
            0x8000..=0x9FFF => self.ppu.write(addr, val),
            0xA000..=0xBFFF => self.cartridge.write(addr, val),
            0xC000..=0xDFFF => self.wram[(addr - 0xC000) as usize] = val,
            0xE000..=0xFDFF => self.wram[(addr - 0xE000) as usize] = val,
            0xFE00..=0xFE9F => self.ppu.write(addr, val),
            0xFF00 => self.joypad.write(val),
            0xFF01 => self.serial_sb = val,
            0xFF02 => self.serial_sc = val,
            0xFF04..=0xFF07 => self.timer.write(addr, val),
            // IF is only 5 bits wide; bits 5-7 always read back as 1.
            0xFF0F => self.if_reg = val | 0xE0,
            0xFF10..=0xFF3F => self.apu.write(addr, val),
            0xFF40..=0xFF45 => self.ppu.write(addr, val),
            0xFF46 => self.dma(val),
            0xFF47..=0xFF4B => self.ppu.write(addr, val),
            0xFF80..=0xFFFE => self.hram[(addr - 0xFF80) as usize] = val,
            0xFFFF => self.ie = val,
            _ => {}
        }
    }

    /// OAM DMA transfer: copy 160 bytes from `val << 8` to the PPU OAM.
    fn dma(&mut self, val: u8) {
        let base = (val as u16) << 8;
        for i in 0..160u16 {
            let byte = self.read_byte(base + i);
            self.ppu.write_oam(i as usize, byte);
        }
    }

    /// Little-endian 16-bit read.
    pub fn read_word(&self, addr: u16) -> u16 {
        (self.read_byte(addr + 1) as u16) << 8 | self.read_byte(addr) as u16
    }

    /// Little-endian 16-bit write.
    pub fn write_word(&mut self, addr: u16, val: u16) {
        self.write_byte(addr, val as u8);
        self.write_byte(addr + 1, (val >> 8) as u8);
    }
}
