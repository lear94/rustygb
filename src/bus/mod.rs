//! System bus: owns every peripheral and routes CPU accesses through the memory map.

mod dma;
#[cfg(test)]
mod tests;

use crate::apu::Apu;
use crate::cartridge::Cartridge;
use crate::hardware::OPEN_BUS;
use crate::joypad::Joypad;
use crate::memory_map::{
    DIV, DMA, ECHO_END, EXTERNAL_RAM_END, EXTERNAL_RAM_START, HIGH_PAGE_END, HIGH_PAGE_START,
    HRAM_END, HRAM_START, IE, IF, LCDC, NR10, OAM_END, OAM_START, P1, ROM_END, ROM_FIXED_START, SB,
    SC, TAC, UNUSABLE_END, UNUSABLE_START, VRAM_END, VRAM_START, WAVE_END, WRAM_START, WX,
};
use crate::ppu::Ppu;
use crate::serial::Serial;
use crate::timer::Timer;
use dma::OamDma;

const WRAM_LEN: usize = 8 * 1024;
const WRAM_MASK: u16 = 0x1FFF;
const _: () = assert!(WRAM_MASK as usize == WRAM_LEN - 1);
const HRAM_LEN: usize = 127;
const HRAM_BASE: u16 = 0xFF80;

const IRQ_TIMER: u8 = 0x04;
const IRQ_SERIAL: u8 = 0x08;
const IRQ_JOYPAD: u8 = 0x10;
const IRQ_COUNT: u8 = 5;
const IRQ_MASK: u8 = 0x1F;
/// IF bits that do not exist read as 1.
const IF_UNUSED: u8 = 0xE0;
/// The DMG boot ROM leaves a V-Blank request pending.
const POST_BOOT_IF: u8 = 0xE1;

/// Emulated machine without its CPU.
pub struct Motherboard {
    cartridge: Cartridge,
    /// Video.
    pub(crate) ppu: Ppu,
    /// Audio.
    pub(crate) apu: Apu,
    /// Divider and timer.
    pub(crate) timer: Timer,
    /// Buttons.
    pub(crate) joypad: Joypad,
    serial: Serial,
    dma: OamDma,
    wram: [u8; WRAM_LEN],
    hram: [u8; HRAM_LEN],
    ie: u8,
    if_reg: u8,
}

impl Motherboard {
    /// Builds a machine around `cartridge`, producing audio at `sample_rate_hz`.
    #[must_use]
    pub fn new(cartridge: Cartridge, sample_rate_hz: u32) -> Self {
        Self {
            cartridge,
            ppu: Ppu::new(),
            apu: Apu::new(sample_rate_hz),
            timer: Timer::new(),
            joypad: Joypad::new(),
            serial: Serial::new(),
            dma: OamDma::new(),
            wram: [0; WRAM_LEN],
            hram: [0; HRAM_LEN],
            ie: 0,
            if_reg: POST_BOOT_IF,
        }
    }

    /// Advances every peripheral by one M-cycle.
    pub fn tick(&mut self) {
        self.cartridge.tick();
        let tick = self.timer.tick();
        if tick.interrupt {
            self.if_reg |= IRQ_TIMER;
        }
        if tick.serial_clock && self.serial.clock() {
            self.if_reg |= IRQ_SERIAL;
        }
        if let Some(copy) = self.dma.tick() {
            let byte = self.read_unblocked(copy.source);
            self.ppu.write_oam(usize::from(copy.oam_idx), byte);
        }
        self.if_reg |= self.ppu.tick();
        self.apu.tick(tick.audio_clock);

        if self.joypad.take_interrupt() {
            self.if_reg |= IRQ_JOYPAD;
        }
    }

    /// Lets one M-cycle of host time pass while STOP holds the system clock.
    pub(crate) fn tick_stopped(&mut self) {
        self.apu.tick_stopped();
    }

    /// Borrows the inserted cartridge.
    pub fn cartridge(&mut self) -> &mut Cartridge {
        &mut self.cartridge
    }

    /// Returns the requested and enabled interrupts as an IF/IE bit mask.
    #[must_use]
    pub fn pending_interrupts(&self) -> u8 {
        self.ie & self.if_reg & IRQ_MASK
    }

    /// Clears the highest-priority pending interrupt and returns its bit number.
    pub(crate) fn acknowledge_interrupt(&mut self) -> Option<u8> {
        let pending = self.pending_interrupts();
        let bit = (0..IRQ_COUNT).find(|bit| pending & (1 << bit) != 0)?;
        self.if_reg &= !(1 << bit);
        Some(bit)
    }

    /// Reads a byte as the CPU sees it; unmapped addresses are open bus.
    #[must_use]
    pub fn read_byte(&self, addr: u16) -> u8 {
        match addr {
            OAM_START..=OAM_END if self.dma.is_busy() => OPEN_BUS,
            _ => self.read_unblocked(addr),
        }
    }

    /// Writes a byte as the CPU does; unmapped addresses are ignored.
    pub fn write_byte(&mut self, addr: u16, val: u8) {
        match addr {
            ROM_FIXED_START..=ROM_END | EXTERNAL_RAM_START..=EXTERNAL_RAM_END => {
                self.cartridge.write(addr, val);
            }
            VRAM_START..=VRAM_END => self.ppu.write(addr, val),
            WRAM_START..=ECHO_END => {
                if let Some(cell) = self.wram.get_mut(usize::from(addr & WRAM_MASK)) {
                    *cell = val;
                }
            }
            OAM_START..=OAM_END if !self.dma.is_busy() => self.ppu.write(addr, val),
            OAM_START..=UNUSABLE_END => {}
            HIGH_PAGE_START..=HIGH_PAGE_END => self.write_high(addr, val),
        }
    }

    fn read_unblocked(&self, addr: u16) -> u8 {
        match addr {
            ROM_FIXED_START..=ROM_END | EXTERNAL_RAM_START..=EXTERNAL_RAM_END => {
                self.cartridge.read(addr)
            }
            VRAM_START..=VRAM_END | OAM_START..=OAM_END => self.ppu.read(addr),
            WRAM_START..=ECHO_END => self
                .wram
                .get(usize::from(addr & WRAM_MASK))
                .copied()
                .unwrap_or(OPEN_BUS),
            // DMG decodes no device here and drives zeros.
            UNUSABLE_START..=UNUSABLE_END => 0,
            HIGH_PAGE_START..=HIGH_PAGE_END => self.read_high(addr),
        }
    }

    fn read_high(&self, addr: u16) -> u8 {
        match addr {
            NR10..=WAVE_END => self.apu.read(addr),
            DMA => self.dma.register(),
            LCDC..=WX => self.ppu.read(addr),
            HRAM_START..=HRAM_END => self
                .hram
                .get(usize::from(addr - HRAM_BASE))
                .copied()
                .unwrap_or(OPEN_BUS),
            IE => self.ie,
            _ => self.read_io(addr),
        }
    }

    fn read_io(&self, addr: u16) -> u8 {
        match addr {
            P1 => self.joypad.read(),
            SB => self.serial.data(),
            SC => self.serial.control(),
            DIV..=TAC => self.timer.read(addr),
            IF => self.if_reg | IF_UNUSED,
            _ => OPEN_BUS,
        }
    }

    fn write_high(&mut self, addr: u16, val: u8) {
        match addr {
            NR10..=WAVE_END => self.apu.write(addr, val),
            DMA => self.dma.start(val),
            LCDC..=WX => self.ppu.write(addr, val),
            HRAM_START..=HRAM_END => {
                if let Some(cell) = self.hram.get_mut(usize::from(addr - HRAM_BASE)) {
                    *cell = val;
                }
            }
            IE => self.ie = val,
            _ => self.write_io(addr, val),
        }
    }

    fn write_io(&mut self, addr: u16, val: u8) {
        match addr {
            P1 => self.joypad.write(val),
            SB => self.serial.write_data(val),
            SC => self.serial.write_control(val),
            DIV..=TAC => self.timer.write(addr, val),
            IF => self.if_reg = val | IF_UNUSED,
            _ => {}
        }
    }
}
