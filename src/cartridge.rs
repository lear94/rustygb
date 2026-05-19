//! Cartridge models (memory-bank controllers).
//!
//! Supported mappers:
//!
//! | Header byte `0x147` | Implementation |
//! |---------------------|----------------|
//! | `0x00`              | [`RomOnly`]    |
//! | `0x01..=0x03`       | [`Mbc1`]       |
//! | `0x0F..=0x13`       | [`Mbc3`] (no RTC) |
//! | `0x19..=0x1E`       | [`Mbc5`]       |
//!
//! Battery-backed SRAM is auto-saved next to the ROM with the `.sav`
//! extension. The format is a raw dump of the cartridge RAM so it is
//! compatible with most other mainstream emulators.

use anyhow::{bail, Context, Result};
use std::fs;
use std::path::Path;

/// Generic interface implemented by every cartridge model. The bus talks
/// to the cartridge exclusively through this trait, so adding a new MBC is
/// just a matter of implementing it and wiring it up in [`from_bytes`].
pub trait Cartridge {
    /// Read a byte from the cartridge address space.
    fn read(&self, addr: u16) -> u8;

    /// Write a byte to the cartridge address space (mapper registers + RAM).
    fn write(&mut self, addr: u16, val: u8);

    /// Persist battery-backed RAM to `save_path`. No-op when the cartridge
    /// has no RAM or no writes have happened since the last save.
    fn save_state(&self, save_path: &str) -> Result<()>;
}

// ---------------------------------------------------------------------------
// ROM-only cartridges (32 KiB, no banking, no RAM).
// ---------------------------------------------------------------------------

struct RomOnly {
    rom: Vec<u8>,
}

impl Cartridge for RomOnly {
    fn read(&self, addr: u16) -> u8 {
        self.rom.get(addr as usize).copied().unwrap_or(0xFF)
    }

    fn write(&mut self, _: u16, _: u8) {}

    fn save_state(&self, _: &str) -> Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// MBC1.
// ---------------------------------------------------------------------------

struct Mbc1 {
    rom: Vec<u8>,
    ram: Vec<u8>,
    /// Lower ROM-bank register (5 bits).
    rb: usize,
    /// Upper bits — RAM bank in mode 1, or ROM bank bits 5-6 in mode 0.
    ab: usize,
    ram_on: bool,
    mode: u8,
    dirty: bool,
    rom_mask: usize,
    ram_mask: usize,
}

impl Cartridge for Mbc1 {
    fn read(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x3FFF => {
                // In banking mode 1 the upper bits also affect the 0x0000-0x3FFF
                // window, which lets large carts expose an alternate bank 0.
                let bank = if self.mode == 1 {
                    (self.ab << 5) & self.rom_mask
                } else {
                    0
                };
                let offset = bank * 0x4000 + addr as usize;
                self.rom[offset & (self.rom.len() - 1)]
            }
            0x4000..=0x7FFF => {
                let mut bank = self.rb | (self.ab << 5);
                // MBC1 quirk: when the lower 5 bits are zero the controller
                // forces a 1 in the resulting bank number.
                if (self.rb & 0x1F) == 0 {
                    bank |= 1;
                }
                bank &= self.rom_mask;

                let offset = bank * 0x4000 + (addr as usize - 0x4000);
                self.rom[offset & (self.rom.len() - 1)]
            }
            0xA000..=0xBFFF => {
                if !self.ram_on || self.ram.is_empty() {
                    return 0xFF;
                }
                let bank = if self.mode == 1 {
                    self.ab & self.ram_mask
                } else {
                    0
                };
                let offset = bank * 0x2000 + (addr as usize - 0xA000);
                self.ram.get(offset).copied().unwrap_or(0xFF)
            }
            _ => 0xFF,
        }
    }

    fn write(&mut self, addr: u16, val: u8) {
        match addr {
            0x0000..=0x1FFF => self.ram_on = (val & 0x0F) == 0x0A,
            0x2000..=0x3FFF => {
                self.rb = (val & 0x1F) as usize;
            }
            0x4000..=0x5FFF => {
                self.ab = (val & 0x03) as usize;
            }
            0x6000..=0x7FFF => self.mode = val & 1,
            0xA000..=0xBFFF => {
                if !self.ram_on || self.ram.is_empty() {
                    return;
                }
                let bank = if self.mode == 1 {
                    self.ab & self.ram_mask
                } else {
                    0
                };
                let offset = bank * 0x2000 + (addr as usize - 0xA000);
                if offset < self.ram.len() {
                    self.ram[offset] = val;
                    self.dirty = true;
                }
            }
            _ => {}
        }
    }

    fn save_state(&self, path: &str) -> Result<()> {
        if self.dirty && !self.ram.is_empty() {
            fs::write(path, &self.ram).context("Failed to save MBC1 RAM")?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// MBC3 (RTC registers are accepted but not emulated).
// ---------------------------------------------------------------------------

struct Mbc3 {
    rom: Vec<u8>,
    ram: Vec<u8>,
    rom_bank: usize,
    ram_bank: usize,
    ram_on: bool,
    dirty: bool,
    rom_mask: usize,
}

impl Cartridge for Mbc3 {
    fn read(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x3FFF => self.rom[addr as usize],
            0x4000..=0x7FFF => {
                let bank = if self.rom_bank == 0 { 1 } else { self.rom_bank };
                let offset = (bank & self.rom_mask) * 0x4000 + (addr as usize - 0x4000);
                self.rom[offset & (self.rom.len() - 1)]
            }
            0xA000..=0xBFFF => {
                if !self.ram_on {
                    return 0xFF;
                }
                if self.ram_bank <= 0x03 {
                    let offset = self.ram_bank * 0x2000 + (addr as usize - 0xA000);
                    self.ram.get(offset).copied().unwrap_or(0xFF)
                } else {
                    // RTC register read — RTC is not emulated yet.
                    0x00
                }
            }
            _ => 0xFF,
        }
    }

    fn write(&mut self, addr: u16, val: u8) {
        match addr {
            0x0000..=0x1FFF => self.ram_on = (val & 0x0F) == 0x0A,
            0x2000..=0x3FFF => {
                let bank = val as usize & 0x7F;
                self.rom_bank = if bank == 0 { 1 } else { bank };
            }
            0x4000..=0x5FFF => {
                if val <= 0x03 || (0x08..=0x0C).contains(&val) {
                    self.ram_bank = val as usize;
                }
            }
            0x6000..=0x7FFF => {
                // RTC latch — accepted to keep games happy, not implemented.
            }
            0xA000..=0xBFFF => {
                if self.ram_on && self.ram_bank <= 0x03 {
                    let offset = self.ram_bank * 0x2000 + (addr as usize - 0xA000);
                    if offset < self.ram.len() {
                        self.ram[offset] = val;
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

// ---------------------------------------------------------------------------
// MBC5.
// ---------------------------------------------------------------------------

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
    fn read(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x3FFF => self.rom[addr as usize],
            0x4000..=0x7FFF => {
                let offset = (self.rb & self.rom_mask) * 0x4000 + (addr as usize - 0x4000);
                self.rom[offset & (self.rom.len() - 1)]
            }
            0xA000..=0xBFFF => {
                if self.ram_on && !self.ram.is_empty() {
                    let offset = self.ab * 0x2000 + (addr as usize - 0xA000);
                    self.ram.get(offset).copied().unwrap_or(0xFF)
                } else {
                    0xFF
                }
            }
            _ => 0xFF,
        }
    }

    fn write(&mut self, addr: u16, val: u8) {
        match addr {
            0x0000..=0x1FFF => self.ram_on = (val & 0x0F) == 0x0A,
            0x2000..=0x2FFF => self.rb = (self.rb & 0x100) | val as usize,
            0x3000..=0x3FFF => self.rb = (self.rb & 0xFF) | ((val as usize & 1) << 8),
            0x4000..=0x5FFF => self.ab = (val & 0x0F) as usize,
            0xA000..=0xBFFF => {
                if self.ram_on && !self.ram.is_empty() {
                    let offset = self.ab * 0x2000 + (addr as usize - 0xA000);
                    if offset < self.ram.len() {
                        self.ram[offset] = val;
                        self.dirty = true;
                    }
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

// ---------------------------------------------------------------------------
// Construction helpers.
// ---------------------------------------------------------------------------

/// Build a cartridge from a raw ROM image and optional previously-saved RAM.
///
/// This is the host-agnostic entry point — both the native binary and the
/// WebAssembly front-end go through it. Pass `Some(bytes)` in `saved_ram`
/// to restore battery-backed saves, otherwise the RAM is zero-initialised.
pub fn from_bytes(rom: Vec<u8>, saved_ram: Option<Vec<u8>>) -> Result<Box<dyn Cartridge>> {
    if rom.len() < 0x150 {
        bail!("ROM too small (must contain a full header)");
    }

    let rom_banks = rom.len() / 0x4000;
    let rom_mask = rom_banks.saturating_sub(1);

    let ram_size = ram_size_from_header(rom[0x149]);
    let ram_banks = ram_size / 0x2000;
    let ram_mask = ram_banks.saturating_sub(1);

    let ram = match saved_ram {
        Some(bytes) if bytes.len() == ram_size => bytes,
        _ => vec![0; ram_size],
    };

    let mapper = rom[0x147];
    match mapper {
        0x00 => Ok(Box::new(RomOnly { rom })),
        0x01..=0x03 => Ok(Box::new(Mbc1 {
            rom,
            ram,
            rb: 1,
            ab: 0,
            ram_on: false,
            mode: 0,
            dirty: false,
            rom_mask,
            ram_mask,
        })),
        0x0F..=0x13 => Ok(Box::new(Mbc3 {
            rom,
            ram,
            rom_bank: 1,
            ram_bank: 0,
            ram_on: false,
            dirty: false,
            rom_mask,
        })),
        0x19..=0x1E => Ok(Box::new(Mbc5 {
            rom,
            ram,
            rb: 1,
            ab: 0,
            ram_on: false,
            dirty: false,
            rom_mask,
        })),
        _ => bail!("Unsupported mapper: {:#04X}", mapper),
    }
}

/// Read a ROM from disk and build the appropriate cartridge. If a `.sav`
/// file sits next to the ROM, its contents are loaded as the initial RAM.
pub fn load_cartridge(path: &str) -> Result<Box<dyn Cartridge>> {
    let rom = fs::read(path).context("Failed to read ROM file")?;
    let save_path = Path::new(path).with_extension("sav");
    let saved_ram = if save_path.exists() {
        fs::read(&save_path).ok()
    } else {
        None
    };
    from_bytes(rom, saved_ram)
}

/// Map the header RAM-size byte (offset `0x149`) to the actual RAM size
/// in bytes. Unknown values default to 8 KiB to match common emulators.
fn ram_size_from_header(byte: u8) -> usize {
    match byte {
        0x00 => 0,
        0x01 => 0x800,
        0x02 => 0x2000,
        0x03 => 0x8000,
        0x04 => 0x20000,
        0x05 => 0x10000,
        _ => 0x2000,
    }
}
