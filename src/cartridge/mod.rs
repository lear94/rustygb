//! Cartridge: ROM image, external RAM and the bank controller mapping them.

mod error;
mod header;
mod mapper;

pub use error::CartridgeError;

use header::Header;
use mapper::Mapper;

/// Value seen on the bus when nothing drives it.
const OPEN_BUS: u8 = 0xFF;
const ROM_BANK_SHIFT: u32 = 14;
const ROM_BANK_MASK: u16 = (1 << ROM_BANK_SHIFT) - 1;
/// Every cartridge maps at least two ROM banks.
const MIN_ROM_LEN: usize = 2 << ROM_BANK_SHIFT;
/// 512 banks, the MBC5 limit and the largest of any supported controller.
const MAX_ROM_LEN: usize = 512 << ROM_BANK_SHIFT;
const RAM_BANK_SHIFT: u32 = 13;
const RAM_BANK_MASK: u16 = (1 << RAM_BANK_SHIFT) - 1;
/// RTC footers appended by other emulators (32- and 64-bit timestamps).
const RTC_FOOTER_LENS: [usize; 2] = [44, 48];

/// Cartridge inserted in the emulated console.
pub struct Cartridge {
    rom: Vec<u8>,
    ram: Vec<u8>,
    /// Foreign RTC bytes, written back untouched so other emulators keep their clock.
    rtc_footer: Vec<u8>,
    mapper: Mapper,
    has_battery: bool,
    dirty: bool,
}

impl Cartridge {
    /// Builds a cartridge from a ROM image and an optional save image.
    ///
    /// # Arguments
    /// * `saved_ram` - raw RAM dump, optionally followed by an RTC footer;
    ///   ignored when the cartridge has no battery-backed RAM.
    ///
    /// # Errors
    /// * [`CartridgeError::RomTooSmall`] - the image has no full header.
    /// * [`CartridgeError::RomTooLarge`] - the image exceeds 8 MiB.
    /// * [`CartridgeError::UnsupportedMapper`] - unknown controller in the header.
    /// * [`CartridgeError::UnsupportedRamSize`] - unknown RAM size in the header.
    /// * [`CartridgeError::SaveSizeMismatch`] - the save does not fit the RAM.
    ///
    /// # Complexity
    /// O(rom + ram) time and space.
    pub fn from_bytes(
        mut rom: Vec<u8>,
        saved_ram: Option<Vec<u8>>,
    ) -> Result<Self, CartridgeError> {
        let header = Header::parse(&rom)?;
        if rom.len() > MAX_ROM_LEN {
            return Err(CartridgeError::RomTooLarge {
                len: rom.len(),
                max: MAX_ROM_LEN,
            });
        }
        // Bank decoding masks address lines, so the image must be a power of two;
        // unpopulated addresses read as open bus.
        rom.resize(rom.len().next_power_of_two().max(MIN_ROM_LEN), OPEN_BUS);
        let mut cartridge = Self {
            rom,
            ram: vec![0; header.ram_len],
            rtc_footer: Vec::new(),
            mapper: header.mapper,
            has_battery: header.has_battery,
            dirty: false,
        };
        if let Some(save) = saved_ram.filter(|_| cartridge.is_persistent()) {
            cartridge.restore(save)?;
        }
        Ok(cartridge)
    }

    /// Reads ROM (`0x0000-0x7FFF`) or external RAM (`0xA000-0xBFFF`).
    #[must_use]
    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x7FFF => self
                .rom
                .get(self.rom_offset(addr))
                .copied()
                .unwrap_or(OPEN_BUS),
            0xA000..=0xBFFF if self.mapper.selects_rtc() => 0,
            0xA000..=0xBFFF => self
                .ram_offset(addr)
                .and_then(|offset| self.ram.get(offset))
                .copied()
                .unwrap_or(OPEN_BUS),
            _ => OPEN_BUS,
        }
    }

    /// Writes a controller register (`0x0000-0x7FFF`) or external RAM.
    pub fn write(&mut self, addr: u16, val: u8) {
        match addr {
            0x0000..=0x7FFF => self.mapper.write_register(addr, val),
            0xA000..=0xBFFF => self.write_ram(addr, val),
            _ => {}
        }
    }

    /// Returns whether battery-backed RAM changed since [`Self::mark_saved`].
    #[must_use]
    pub fn needs_save(&self) -> bool {
        self.dirty && self.is_persistent()
    }

    /// Records that the current [`Self::save_image`] reached storage.
    pub fn mark_saved(&mut self) {
        self.dirty = false;
    }

    /// Returns the save image, or `None` without battery-backed RAM.
    ///
    /// # Complexity
    /// O(ram) time and space.
    #[must_use]
    pub fn save_image(&self) -> Option<Vec<u8>> {
        self.is_persistent()
            .then(|| [self.ram.as_slice(), self.rtc_footer.as_slice()].concat())
    }

    fn is_persistent(&self) -> bool {
        self.has_battery && !self.ram.is_empty()
    }

    fn restore(&mut self, mut save: Vec<u8>) -> Result<(), CartridgeError> {
        let mismatch = CartridgeError::SaveSizeMismatch {
            expected: self.ram.len(),
            found: save.len(),
        };
        let Some(footer_len) = save.len().checked_sub(self.ram.len()) else {
            return Err(mismatch);
        };
        if footer_len != 0 && !RTC_FOOTER_LENS.contains(&footer_len) {
            return Err(mismatch);
        }
        self.rtc_footer = save.split_off(self.ram.len());
        self.ram = save;
        Ok(())
    }

    fn rom_offset(&self, addr: u16) -> usize {
        let bank = self.mapper.rom_bank(addr);
        let offset = (bank << ROM_BANK_SHIFT) | usize::from(addr & ROM_BANK_MASK);
        offset & self.rom.len().saturating_sub(1)
    }

    fn ram_offset(&self, addr: u16) -> Option<usize> {
        let bank = self.mapper.ram_bank()?;
        let offset = (bank << RAM_BANK_SHIFT) | usize::from(addr & RAM_BANK_MASK);
        // Every RAM size is a power of two, so the mask mirrors like real decoding.
        Some(offset & self.ram.len().checked_sub(1)?)
    }

    fn write_ram(&mut self, addr: u16, val: u8) {
        let Some(offset) = self.ram_offset(addr) else {
            return;
        };
        if let Some(cell) = self.ram.get_mut(offset) {
            *cell = val;
            self.dirty = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAM_LEN: usize = 0x2000;
    const MBC1_BATTERY: u8 = 0x03;
    const MBC3_RTC: u8 = 0x10;
    const MBC5: u8 = 0x19;

    fn rom(mapper: u8, ram_code: u8, banks: u8) -> Vec<u8> {
        let mut rom: Vec<u8> = (0..banks)
            .flat_map(|bank| vec![bank; 1 << ROM_BANK_SHIFT])
            .collect();
        rom[0x147] = mapper;
        rom[0x149] = ram_code;
        rom
    }

    fn cart(mapper: u8) -> Cartridge {
        Cartridge::from_bytes(rom(mapper, 2, 8), None).expect("valid ROM")
    }

    fn with_save(mapper: u8, save_len: usize) -> Result<Cartridge, CartridgeError> {
        Cartridge::from_bytes(rom(mapper, 2, 2), Some(vec![9; save_len]))
    }

    #[test]
    fn from_bytes_accepts_plain_and_rtc_saves() {
        for len in [RAM_LEN, RAM_LEN + 44, RAM_LEN + 48] {
            let mut cart = with_save(MBC3_RTC, len).expect("known save layout");
            cart.write(0, 0x0A);
            assert_eq!(cart.read(0xA000), 9);
            assert_eq!(cart.save_image(), Some(vec![9; len]));
        }
    }

    #[test]
    fn from_bytes_wrong_save_size_is_rejected() {
        for len in [0, RAM_LEN - 1, RAM_LEN + 1, RAM_LEN * 2] {
            assert_eq!(
                with_save(MBC1_BATTERY, len).err(),
                Some(CartridgeError::SaveSizeMismatch {
                    expected: RAM_LEN,
                    found: len
                })
            );
        }
    }

    #[test]
    fn from_bytes_without_battery_ignores_save() {
        let cart = with_save(MBC5, 1).expect("save is irrelevant");
        assert_eq!(cart.save_image(), None);
    }

    #[test]
    fn from_bytes_pads_irregular_images() {
        for mapper in [0, MBC1_BATTERY, MBC3_RTC, MBC5] {
            let mut short = rom(mapper, 0, 2);
            short.truncate(0x150);
            let cart = Cartridge::from_bytes(short, None).expect("header is present");
            assert_eq!((cart.read(0x150), cart.read(0x7FFF)), (OPEN_BUS, OPEN_BUS));
        }
        let mut cart = Cartridge::from_bytes(rom(MBC5, 0, 6), None).expect("six banks");
        cart.write(0x2000, 5);
        assert_eq!(cart.read(0x4000), 5);
        cart.write(0x2000, 6);
        assert_eq!(cart.read(0x4000), OPEN_BUS);
    }

    #[test]
    fn from_bytes_size_limit_is_inclusive() {
        assert!(Cartridge::from_bytes(vec![0; MAX_ROM_LEN], None).is_ok());
        assert_eq!(
            Cartridge::from_bytes(vec![0; MAX_ROM_LEN + 1], None).err(),
            Some(CartridgeError::RomTooLarge {
                len: MAX_ROM_LEN + 1,
                max: MAX_ROM_LEN
            })
        );
    }

    #[test]
    fn needs_save_tracks_ram_writes() {
        let mut cart = cart(MBC1_BATTERY);
        cart.write(0xA000, 1);
        assert!(!cart.needs_save(), "RAM is disabled");
        cart.write(0, 0x0A);
        cart.write(0xA000, 1);
        assert!(cart.needs_save());
        cart.mark_saved();
        assert!(!cart.needs_save());
    }

    #[test]
    fn needs_save_without_battery_is_false() {
        let mut cart = cart(MBC5);
        cart.write(0, 0x0A);
        cart.write(0xA000, 1);
        assert_eq!(cart.read(0xA000), 1);
        assert!(!cart.needs_save());
    }

    #[test]
    fn read_switchable_bank_follows_register() {
        for mapper in [MBC1_BATTERY, MBC3_RTC, MBC5] {
            let mut cart = cart(mapper);
            assert_eq!(cart.read(0x4000), 1);
            cart.write(0x2000, 5);
            assert_eq!(cart.read(0x7FFF), 5);
            cart.write(0x2000, 13);
            assert_eq!(cart.read(0x4000), 5, "bank wraps at the ROM size");
        }
    }

    #[test]
    fn read_unmapped_is_open_bus() {
        assert_eq!(cart(MBC5).read(0xC000), OPEN_BUS);
        assert_eq!(cart(MBC5).read(0xA000), OPEN_BUS);
    }

    #[test]
    fn read_rtc_register_is_zero() {
        let mut cart = cart(MBC3_RTC);
        cart.write(0, 0x0A);
        cart.write(0x4000, 0x08);
        assert_eq!(cart.read(0xA000), 0);
        cart.write(0xA000, 7);
        assert!(!cart.needs_save());
    }
}
