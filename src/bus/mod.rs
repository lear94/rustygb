//! System bus: owns every peripheral and routes CPU accesses through the memory map.

mod dma;
#[cfg(test)]
mod tests;

use crate::apu::Apu;
use crate::cartridge::Cartridge;
use crate::joypad::Joypad;
use crate::ppu::Ppu;
use crate::timer::Timer;
use dma::OamDma;

/// T-cycles per M-cycle, the granularity of bus accesses.
pub const T_CYCLES_PER_M_CYCLE: u32 = 4;
/// Same span for the peripherals counting in bytes.
const TICK_T_CYCLES: u8 = 4;
const WRAM_LEN: usize = 8 * 1024;
const WRAM_MASK: u16 = 0x1FFF;
const HRAM_LEN: usize = 127;
const HRAM_BASE: u16 = 0xFF80;
/// Value seen on the bus when nothing drives it.
const OPEN_BUS: u8 = 0xFF;

const IRQ_TIMER: u8 = 0x04;
const IRQ_SERIAL: u8 = 0x08;
const IRQ_JOYPAD: u8 = 0x10;
const IRQ_COUNT: u8 = 5;
const IRQ_MASK: u8 = 0x1F;
/// IF bits that do not exist read as 1.
const IF_UNUSED: u8 = 0xE0;
/// The DMG boot ROM leaves a V-Blank request pending.
const POST_BOOT_IF: u8 = 0xE1;

const SERIAL_START: u8 = 0x80;
const SERIAL_UNUSED: u8 = 0x7E;

/// Emulated machine without its CPU.
pub struct Motherboard {
    cartridge: Cartridge,
    /// Video.
    pub ppu: Ppu,
    /// Audio.
    pub apu: Apu,
    /// Divider and timer.
    pub timer: Timer,
    /// Buttons.
    pub joypad: Joypad,
    dma: OamDma,
    wram: [u8; WRAM_LEN],
    hram: [u8; HRAM_LEN],
    ie: u8,
    if_reg: u8,
    serial_sb: u8,
    serial_sc: u8,
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
            dma: OamDma::new(),
            wram: [0; WRAM_LEN],
            hram: [0; HRAM_LEN],
            ie: 0,
            if_reg: POST_BOOT_IF,
            serial_sb: 0,
            serial_sc: 0,
        }
    }

    /// Advances every peripheral by one M-cycle.
    pub fn tick(&mut self) {
        if self.timer.tick() {
            self.if_reg |= IRQ_TIMER;
        }
        if let Some(copy) = self.dma.tick() {
            let byte = self.read_unblocked(copy.source);
            self.ppu.write_oam(usize::from(copy.oam_idx), byte);
        }
        self.ppu.tick(TICK_T_CYCLES, &mut self.if_reg);
        self.apu.tick();

        // No link partner: transfers complete at once.
        if self.serial_sc & SERIAL_START != 0 {
            self.serial_sc &= !SERIAL_START;
            self.if_reg |= IRQ_SERIAL;
        }
        if std::mem::take(&mut self.joypad.interrupt) {
            self.if_reg |= IRQ_JOYPAD;
        }
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
            0xFE00..=0xFE9F if self.dma.is_busy() => OPEN_BUS,
            _ => self.read_unblocked(addr),
        }
    }

    /// Writes a byte as the CPU does; unmapped addresses are ignored.
    pub fn write_byte(&mut self, addr: u16, val: u8) {
        match addr {
            0x0000..=0x7FFF | 0xA000..=0xBFFF => self.cartridge.write(addr, val),
            0x8000..=0x9FFF => self.ppu.write(addr, val),
            0xC000..=0xFDFF => {
                if let Some(cell) = self.wram.get_mut(usize::from(addr & WRAM_MASK)) {
                    *cell = val;
                }
            }
            0xFE00..=0xFE9F if !self.dma.is_busy() => self.ppu.write(addr, val),
            0xFE00..=0xFEFF => {}
            0xFF00..=0xFFFF => self.write_high(addr, val),
        }
    }

    fn read_unblocked(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x7FFF | 0xA000..=0xBFFF => self.cartridge.read(addr),
            0x8000..=0x9FFF | 0xFE00..=0xFE9F => self.ppu.read(addr),
            0xC000..=0xFDFF => self
                .wram
                .get(usize::from(addr & WRAM_MASK))
                .copied()
                .unwrap_or(OPEN_BUS),
            0xFEA0..=0xFEFF => OPEN_BUS,
            0xFF00..=0xFFFF => self.read_high(addr),
        }
    }

    fn read_high(&self, addr: u16) -> u8 {
        match addr {
            0xFF00 => self.joypad.read(),
            0xFF01 => self.serial_sb,
            0xFF02 => self.serial_sc | SERIAL_UNUSED,
            0xFF04..=0xFF07 => self.timer.read(addr),
            0xFF0F => self.if_reg | IF_UNUSED,
            0xFF10..=0xFF3F => self.apu.read(addr),
            0xFF46 => self.dma.register(),
            0xFF40..=0xFF4B => self.ppu.read(addr),
            0xFFFF => self.ie,
            _ => self.hram.get(hram_idx(addr)).copied().unwrap_or(OPEN_BUS),
        }
    }

    fn write_high(&mut self, addr: u16, val: u8) {
        match addr {
            0xFF00 => self.joypad.write(val),
            0xFF01 => self.serial_sb = val,
            0xFF02 => self.serial_sc = val,
            0xFF04..=0xFF07 => self.timer.write(addr, val),
            0xFF0F => self.if_reg = val | IF_UNUSED,
            0xFF10..=0xFF3F => self.apu.write(addr, val),
            0xFF46 => self.dma.start(val),
            0xFF40..=0xFF4B => self.ppu.write(addr, val),
            0xFFFF => self.ie = val,
            _ => {
                if let Some(cell) = self.hram.get_mut(hram_idx(addr)) {
                    *cell = val;
                }
            }
        }
    }
}

/// Index into high RAM; addresses outside it wrap to an index beyond its length.
fn hram_idx(addr: u16) -> usize {
    usize::from(addr.wrapping_sub(HRAM_BASE))
}
