use crate::apu::Apu;
use crate::cartridge::Cartridge;
use crate::joypad::Joypad;
use crate::ppu::Ppu;
use crate::timer::Timer;
use anyhow::Result;

pub struct Motherboard {
    cartridge: Box<dyn Cartridge>,
    pub ppu: Ppu,
    pub apu: Apu,
    pub timer: Timer,
    pub joypad: Joypad,
    wram: [u8; 8192],
    hram: [u8; 127],
    ie: u8,
    if_reg: u8,
    serial_sb: u8,
    serial_sc: u8,
}

impl Motherboard {
    pub fn new(cartridge: Box<dyn Cartridge>, sample_rate: u32) -> Self {
        Self {
            cartridge,
            ppu: Ppu::new(),
            apu: Apu::new(sample_rate),
            timer: Timer::new(),
            joypad: Joypad::new(),
            wram: [0; 8192],
            hram: [0; 127],
            ie: 0,
            if_reg: 0xE1, // Typical initial value (Bits 5-7 always 1)
            serial_sb: 0,
            serial_sc: 0,
        }
    }

    pub fn tick(&mut self, cycles: u8) {
        self.timer.tick(cycles, &mut self.if_reg);
        self.ppu.tick(cycles, &mut self.if_reg);
        self.apu.tick(cycles);

        // Serial Port Stub: Tetris expects bit 7 to be cleared after transfer.
        // If the game requests transfer (Bit 7 of SC), simulate instant completion.
        if (self.serial_sc & 0x80) != 0 {
            self.serial_sc &= 0x7F; // Clear transfer bit
            self.if_reg |= 8; // Request Serial Interrupt (Bit 3)
        }

        if self.joypad.interrupt {
            self.if_reg |= 0x10;
            self.joypad.interrupt = false;
        }
    }

    pub fn save_external_ram(&self, base_path: &str) -> Result<()> {
        let save_path = std::path::Path::new(base_path).with_extension("sav");
        self.cartridge.save_state(save_path.to_str().unwrap())
    }

    pub fn read_byte(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x7FFF => self.cartridge.read(addr),
            0x8000..=0x9FFF => self.ppu.read(addr),
            0xA000..=0xBFFF => self.cartridge.read(addr),
            0xC000..=0xDFFF => self.wram[(addr - 0xC000) as usize],
            0xE000..=0xFDFF => self.wram[(addr - 0xE000) as usize], // Echo RAM
            0xFE00..=0xFE9F => self.ppu.read(addr),
            0xFF00 => self.joypad.read(),

            0xFF01 => self.serial_sb,
            0xFF02 => self.serial_sc | 0x7E,

            0xFF04..=0xFF07 => self.timer.read(addr),

            // IF Register: Upper bits 5, 6, 7 always return 1 on real hardware
            0xFF0F => self.if_reg | 0xE0,

            0xFF10..=0xFF3F => self.apu.read(addr),
            0xFF40..=0xFF4B => self.ppu.read(addr),
            0xFF80..=0xFFFE => self.hram[(addr - 0xFF80) as usize],
            0xFFFF => self.ie,
            _ => 0xFF,
        }
    }

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

            // Writing to IF only modifies lower 5 bits.
            // Components (PPU/Timer) OR into this, so be careful not to clear pending interrupts accidentally.
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

    fn dma(&mut self, val: u8) {
        let base = (val as u16) << 8;
        for i in 0..160 {
            let b = self.read_byte(base + i);
            self.ppu.write_oam(i as usize, b);
        }
    }

    pub fn read_word(&self, addr: u16) -> u16 {
        (self.read_byte(addr + 1) as u16) << 8 | self.read_byte(addr) as u16
    }

    pub fn write_word(&mut self, addr: u16, val: u16) {
        self.write_byte(addr, (val & 0xFF) as u8);
        self.write_byte(addr + 1, (val >> 8) as u8);
    }
}
