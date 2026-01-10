use anyhow::{bail, Context, Result};
use std::fs;
use std::path::Path;

pub trait Cartridge {
    fn read(&self, a: u16) -> u8;
    fn write(&mut self, a: u16, v: u8);
    fn save_state(&self, save_path: &str) -> Result<()>;
}

struct RomOnly {
    rom: Vec<u8>,
}

impl Cartridge for RomOnly {
    fn read(&self, a: u16) -> u8 {
        if (a as usize) < self.rom.len() {
            self.rom[a as usize]
        } else {
            0xFF
        }
    }
    fn write(&mut self, _: u16, _: u8) {}
    fn save_state(&self, _: &str) -> Result<()> {
        Ok(())
    }
}

struct Mbc1 {
    rom: Vec<u8>,
    ram: Vec<u8>,
    rb: usize, // ROM Bank Register
    ab: usize, // RAM Bank Register (or upper ROM bits)
    ram_on: bool,
    mode: u8,
    dirty: bool,
    rom_mask: usize, // Safety Mask
    ram_mask: usize, // Safety Mask
}

impl Cartridge for Mbc1 {
    fn read(&self, a: u16) -> u8 {
        match a {
            // Bank 0 (Fixed, or switchable in Mode 1)
            0..=0x3FFF => {
                // In Mode 1, upper bits (ab) also affect bank 0000-3FFF.
                // For Zelda (512KB), 'ab' (bits 5-6) will always be 0 after masking,
                // so this behaves correctly as bank 0.
                let bank = if self.mode == 1 {
                    (self.ab << 5) & self.rom_mask
                } else {
                    0
                };
                let addr = bank * 0x4000 + (a as usize);
                self.rom[addr & (self.rom.len() - 1)]
            }
            // Bank X (Switchable)
            0x4000..=0x7FFF => {
                let mut bank = self.rb | (self.ab << 5);
                // MBC1 Hardware Correction: If bits 0-4 are 0, they are read as 1
                if (self.rb & 0x1F) == 0 {
                    bank |= 1;
                }

                // Critical Mask: Ensures the bank exists on this cartridge
                bank &= self.rom_mask;

                let addr = bank * 0x4000 + (a as usize - 0x4000);
                self.rom[addr & (self.rom.len() - 1)] // Double OOB protection
            }
            // External RAM
            0xA000..=0xBFFF => {
                if !self.ram_on || self.ram.is_empty() {
                    return 0xFF;
                }
                // If Mode 0, only RAM bank 0 is used. If Mode 1, 'ab' is used.
                let bank = if self.mode == 1 {
                    self.ab & self.ram_mask
                } else {
                    0
                };
                let addr = bank * 0x2000 + (a as usize - 0xA000);
                if addr < self.ram.len() {
                    self.ram[addr]
                } else {
                    0xFF
                }
            }
            _ => 0xFF,
        }
    }

    fn write(&mut self, a: u16, v: u8) {
        match a {
            0..=0x1FFF => self.ram_on = (v & 0xF) == 0xA,
            0x2000..=0x3FFF => {
                // Write lower 5 bits of ROM bank
                self.rb = (v & 0x1F) as usize;
                // Note: 0->1 conversion happens on read (Hardware accurate)
            }
            0x4000..=0x5FFF => {
                // Write upper 2 bits (ab).
                // In Mode 0: These are bits 5-6 of ROM.
                // In Mode 1: These are bits 0-1 of RAM.
                self.ab = (v & 3) as usize;
            }
            0x6000..=0x7FFF => self.mode = v & 1,
            0xA000..=0xBFFF => {
                if self.ram_on && !self.ram.is_empty() {
                    let bank = if self.mode == 1 {
                        self.ab & self.ram_mask
                    } else {
                        0
                    };
                    let addr = bank * 0x2000 + (a as usize - 0xA000);
                    if addr < self.ram.len() {
                        self.ram[addr] = v;
                        self.dirty = true;
                    }
                }
            }
            _ => {}
        }
    }

    fn save_state(&self, path: &str) -> Result<()> {
        if self.dirty && !self.ram.is_empty() {
            fs::write(path, &self.ram).context("Failed to save MBC1 RAM")?;
            println!("DEBUG: MBC1 SRAM Saved.");
        }
        Ok(())
    }
}

struct Mbc3 {
    rom: Vec<u8>,
    ram: Vec<u8>,
    rom_bank: usize,
    ram_bank: usize,
    ram_on: bool,
    _rtc_select: u8,
    dirty: bool,
    rom_mask: usize,
}

impl Cartridge for Mbc3 {
    fn read(&self, a: u16) -> u8 {
        match a {
            0x0000..=0x3FFF => self.rom[a as usize],
            0x4000..=0x7FFF => {
                let bank = if self.rom_bank == 0 { 1 } else { self.rom_bank };
                let addr = (bank & self.rom_mask) * 0x4000 + (a as usize - 0x4000);
                self.rom[addr & (self.rom.len() - 1)]
            }
            0xA000..=0xBFFF => {
                if !self.ram_on {
                    return 0xFF;
                }
                if self.ram_bank <= 0x03 {
                    let addr = self.ram_bank * 0x2000 + (a as usize - 0xA000);
                    if addr < self.ram.len() {
                        self.ram[addr]
                    } else {
                        0xFF
                    }
                } else {
                    0x00
                }
            }
            _ => 0xFF,
        }
    }

    fn write(&mut self, a: u16, v: u8) {
        match a {
            0x0000..=0x1FFF => self.ram_on = (v & 0x0F) == 0x0A,
            0x2000..=0x3FFF => {
                let b = v as usize & 0x7F; // MBC3 uses 7 bits
                self.rom_bank = if b == 0 { 1 } else { b };
            }
            0x4000..=0x5FFF => {
                if v <= 0x03 {
                    self.ram_bank = v as usize;
                } else if v >= 0x08 && v <= 0x0C {
                    self.ram_bank = v as usize;
                }
            }
            0x6000..=0x7FFF => { /* RTC Latch */ }
            0xA000..=0xBFFF => {
                if self.ram_on && self.ram_bank <= 0x03 {
                    let addr = self.ram_bank * 0x2000 + (a as usize - 0xA000);
                    if addr < self.ram.len() {
                        self.ram[addr] = v;
                        self.dirty = true;
                    }
                }
            }
            _ => {}
        }
    }

    fn save_state(&self, path: &str) -> Result<()> {
        if self.dirty && !self.ram.is_empty() {
            fs::write(path, &self.ram).context("Failed to save MBC3 RAM")?;
        }
        Ok(())
    }
}

struct Mbc5 {
    rom: Vec<u8>,
    ram: Vec<u8>,
    rb: usize,
    ab: usize,
    ram_on: bool,
    dirty: bool,
    rom_mask: usize,
}

impl Cartridge for Mbc5 {
    fn read(&self, a: u16) -> u8 {
        match a {
            0x0000..=0x3FFF => self.rom[a as usize],
            0x4000..=0x7FFF => {
                let addr = (self.rb & self.rom_mask) * 0x4000 + (a as usize - 0x4000);
                self.rom[addr & (self.rom.len() - 1)]
            }
            0xA000..=0xBFFF => {
                if self.ram_on && !self.ram.is_empty() {
                    self.ram[self.ab * 0x2000 + (a as usize - 0xA000)]
                } else {
                    0xFF
                }
            }
            _ => 0xFF,
        }
    }

    fn write(&mut self, a: u16, v: u8) {
        match a {
            0x0000..=0x1FFF => self.ram_on = (v & 0xF) == 0xA,
            0x2000..=0x2FFF => self.rb = (self.rb & 0x100) | v as usize,
            0x3000..=0x3FFF => self.rb = (self.rb & 0xFF) | ((v as usize & 1) << 8),
            0x4000..=0x5FFF => self.ab = (v & 0xF) as usize,
            0xA000..=0xBFFF => {
                if self.ram_on && !self.ram.is_empty() {
                    self.ram[self.ab * 0x2000 + (a as usize - 0xA000)] = v;
                    self.dirty = true;
                }
            }
            _ => {}
        }
    }

    fn save_state(&self, path: &str) -> Result<()> {
        if self.dirty && !self.ram.is_empty() {
            fs::write(path, &self.ram).context("Failed to save MBC5 RAM")?;
        }
        Ok(())
    }
}

pub fn load_cartridge(path: &str) -> Result<Box<dyn Cartridge>> {
    let d = fs::read(path).context("Failed to read ROM file")?;
    if d.len() < 0x150 {
        bail!("ROM too small");
    }

    // Mask Calculation
    // Calculate how many banks the ROM has to avoid overflow
    let rom_banks = d.len() / 0x4000;
    let rom_mask = if rom_banks > 0 { rom_banks - 1 } else { 0 };

    println!(
        "DIAGNOSTIC: ROM Size: {} bytes, Banks: {}, Mask: {:#X}",
        d.len(),
        rom_banks,
        rom_mask
    );

    // Auto-detect RAM
    let ram_size_id = d[0x149];
    let ram_size = match ram_size_id {
        0x00 => 0,
        0x01 => 0x800,
        0x02 => 0x2000,
        0x03 => 0x8000,
        0x04 => 0x20000,
        0x05 => 0x10000,
        _ => 0x2000,
    };

    // RAM Mask (0 if only 1 bank of 8KB)
    let ram_banks = ram_size / 0x2000;
    let ram_mask = if ram_banks > 0 { ram_banks - 1 } else { 0 };

    let save_path = Path::new(path).with_extension("sav");
    let saved_ram = if save_path.exists() {
        fs::read(&save_path).unwrap_or_else(|_| vec![])
    } else {
        vec![]
    };

    let init_ram = |current_ram: Vec<u8>, size: usize| -> Vec<u8> {
        if !current_ram.is_empty() && current_ram.len() == size {
            current_ram
        } else {
            vec![0; size]
        }
    };

    let mapper = d[0x147];
    match mapper {
        0x00 => Ok(Box::new(RomOnly { rom: d })),
        0x01..=0x03 => Ok(Box::new(Mbc1 {
            rom: d,
            ram: init_ram(saved_ram, ram_size),
            rb: 1,
            ab: 0,
            ram_on: false,
            mode: 0,
            dirty: false,
            rom_mask,
            ram_mask,
        })),
        0x0F..=0x13 => Ok(Box::new(Mbc3 {
            rom: d,
            ram: init_ram(saved_ram, ram_size),
            rom_bank: 1,
            ram_bank: 0,
            ram_on: false,
            _rtc_select: 0,
            dirty: false,
            rom_mask,
        })),
        0x19..=0x1E => Ok(Box::new(Mbc5 {
            rom: d,
            ram: init_ram(saved_ram, ram_size),
            rb: 1,
            ab: 0,
            ram_on: false,
            dirty: false,
            rom_mask,
        })),
        _ => bail!("Unsupported mapper: {:#04X}", mapper),
    }
}
