//! Cartridge: ROM image, external RAM and the bank controller mapping them.

mod error;
mod header;
mod mapper;
mod rtc;

pub use error::CartridgeError;

use crate::hardware::OPEN_BUS;
use header::Header;
use mapper::{Mapper, RamTarget};
use rtc::Rtc;

const ROM_BANK_SHIFT: u32 = 14;
const ROM_BANK_MASK: u16 = (1 << ROM_BANK_SHIFT) - 1;
/// Every cartridge maps at least two ROM banks.
const MIN_ROM_LEN: usize = 2 << ROM_BANK_SHIFT;
/// 512 banks, the MBC5 limit and the largest of any supported controller.
const MAX_ROM_LEN: usize = 512 << ROM_BANK_SHIFT;
const RAM_BANK_SHIFT: u32 = 13;
const RAM_BANK_MASK: u16 = (1 << RAM_BANK_SHIFT) - 1;
/// Save length other emulators write for 2 KiB cartridges.
const PADDED_SAVE_LEN: usize = 8 * 1024;

/// Cartridge inserted in the emulated console.
pub struct Cartridge {
    rom: Vec<u8>,
    ram: Vec<u8>,
    rtc: Option<Rtc>,
    mapper: Mapper,
    has_battery: bool,
    dirty: bool,
}

impl Cartridge {
    /// Builds a cartridge from a ROM image and an optional save image.
    ///
    /// # Arguments
    /// * `saved_ram` - raw RAM dump, optionally followed by an RTC footer; ignored when
    ///   empty or when the cartridge has no battery. Call [`Self::sync_clock`] afterwards.
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
            rtc: header.has_rtc.then(Rtc::default),
            mapper: header.mapper,
            has_battery: header.has_battery,
            dirty: false,
        };
        if let Some(save) = saved_ram.filter(|save| !save.is_empty() && cartridge.is_persistent()) {
            cartridge.restore(&save)?;
        }
        Ok(cartridge)
    }

    /// Advances the real-time clock, if any, by one M-cycle.
    pub fn tick(&mut self) {
        if let Some(rtc) = &mut self.rtc {
            rtc.tick();
        }
    }

    /// Adds the time elapsed since the restored save to the real-time clock.
    pub fn sync_clock(&mut self, unix_now_s: u64) {
        if let Some(rtc) = &mut self.rtc {
            rtc.sync(unix_now_s);
        }
    }

    /// Reads ROM (`0x0000-0x7FFF`) or external RAM (`0xA000-0xBFFF`).
    #[must_use]
    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x3FFF => self.read_rom(self.mapper.fixed_rom_bank(), addr),
            0x4000..=0x7FFF => self.read_rom(self.mapper.switchable_rom_bank(), addr),
            0xA000..=0xBFFF => self.read_ram(addr),
            _ => OPEN_BUS,
        }
    }

    /// Writes a controller register (`0x0000-0x7FFF`) or external RAM.
    pub fn write(&mut self, addr: u16, val: u8) {
        match addr {
            0x0000..=0x7FFF => {
                self.mapper.write_register(addr, val);
                if let (0x6000.., Some(rtc)) = (addr, &mut self.rtc) {
                    rtc.write_latch(val);
                }
            }
            0xA000..=0xBFFF => self.write_ram(addr, val),
            _ => {}
        }
    }

    /// Returns whether battery-backed state changed since [`Self::mark_saved`].
    #[must_use]
    pub fn needs_save(&self) -> bool {
        self.dirty && self.is_persistent()
    }

    /// Records that the current [`Self::save_image`] reached storage.
    pub fn mark_saved(&mut self) {
        self.dirty = false;
    }

    /// Returns the save image, or `None` without battery-backed state.
    ///
    /// # Complexity
    /// O(ram) time and space.
    #[must_use]
    pub fn save_image(&self, unix_now_s: u64) -> Option<Vec<u8>> {
        let footer = self.rtc.as_ref().map(|rtc| rtc.footer(unix_now_s));
        self.is_persistent()
            .then(|| [self.ram.as_slice(), &footer.unwrap_or_default()].concat())
    }

    fn is_persistent(&self) -> bool {
        self.has_battery && (!self.ram.is_empty() || self.rtc.is_some())
    }

    fn restore(&mut self, save: &[u8]) -> Result<(), CartridgeError> {
        let mismatch = CartridgeError::SaveSizeMismatch {
            expected: self.ram.len(),
            found: save.len(),
        };
        let Some((ram, rest)) = save.split_at_checked(self.ram.len()) else {
            return Err(mismatch);
        };
        // Padding is recognized first: no RTC cartridge has such a short RAM.
        let is_padded = !ram.is_empty() && self.rtc.is_none() && save.len() == PADDED_SAVE_LEN;
        let is_footer = matches!(rest.len(), 0 | rtc::SHORT_FOOTER_LEN | rtc::FOOTER_LEN);
        if !is_padded && !is_footer {
            return Err(mismatch);
        }
        self.ram.copy_from_slice(ram);
        if let Some(rtc) = &mut self.rtc {
            rtc.restore(rest);
        }
        Ok(())
    }

    fn read_rom(&self, bank: usize, addr: u16) -> u8 {
        let offset = (bank << ROM_BANK_SHIFT) | usize::from(addr & ROM_BANK_MASK);
        self.rom
            .get(offset & self.rom.len().saturating_sub(1))
            .copied()
            .unwrap_or(OPEN_BUS)
    }

    fn read_ram(&self, addr: u16) -> u8 {
        match (self.mapper.ram_target(), &self.rtc) {
            (RamTarget::Bank(bank), _) => self
                .ram
                .get(self.ram_offset(bank, addr))
                .map_or(OPEN_BUS, |byte| byte | self.mapper.ram_unused_bits()),
            (RamTarget::Rtc(select), Some(rtc)) => rtc.read(select),
            _ => OPEN_BUS,
        }
    }

    fn write_ram(&mut self, addr: u16, val: u8) {
        match (self.mapper.ram_target(), &mut self.rtc) {
            (RamTarget::Bank(bank), _) => {
                let offset = self.ram_offset(bank, addr);
                if let Some(cell) = self.ram.get_mut(offset) {
                    // Rewriting the same value must not schedule a save.
                    self.dirty |= *cell != val;
                    *cell = val;
                }
            }
            (RamTarget::Rtc(select), Some(rtc)) => {
                rtc.write(select, val);
                self.dirty = true;
            }
            _ => {}
        }
    }

    /// Offset into RAM; every size is a power of two, so the mask mirrors like real
    /// decoding. Without RAM it is out of range.
    fn ram_offset(&self, bank: usize, addr: u16) -> usize {
        let offset = (bank << RAM_BANK_SHIFT) | usize::from(addr & RAM_BANK_MASK);
        offset & self.ram.len().wrapping_sub(1)
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
        for mapper in [MBC1_BATTERY, MBC3_RTC] {
            for len in [RAM_LEN, RAM_LEN + 44, RAM_LEN + 48] {
                let mut cart = with_save(mapper, len).expect("known save layout");
                cart.write(0, 0x0A);
                assert_eq!(cart.read(0xA000), 9);
            }
        }
    }

    #[test]
    fn from_bytes_padded_small_save_is_accepted() {
        let small = |len| Cartridge::from_bytes(rom(MBC1_BATTERY, 1, 2), Some(vec![9; len]));
        let mut cart = small(PADDED_SAVE_LEN).expect("padded by another emulator");
        cart.write(0, 0x0A);
        assert_eq!((cart.read(0xA7FF), cart.read(0xA800)), (9, 9));
        assert_eq!(cart.save_image(0).map(|image| image.len()), Some(0x800));
        assert!(small(PADDED_SAVE_LEN + 1).is_err());
    }

    #[test]
    fn from_bytes_empty_save_is_absent() {
        assert!(with_save(MBC1_BATTERY, 0).is_ok());
    }

    #[test]
    fn save_image_round_trips_clock() {
        let mut cart = cart(MBC3_RTC);
        cart.write(0, 0x0A);
        cart.write(0x4000, 0x08);
        cart.write(0xA000, 30);
        assert!(cart.needs_save());
        let image = cart.save_image(100);
        let mut reloaded =
            Cartridge::from_bytes(rom(MBC3_RTC, 2, 8), image).expect("own save layout");
        reloaded.sync_clock(105);
        reloaded.write(0, 0x0A);
        reloaded.write(0x4000, 0x08);
        assert_eq!(reloaded.read(0xA000), 30, "latched value is unchanged");
        reloaded.write(0x6000, 0);
        reloaded.write(0x6000, 1);
        assert_eq!(reloaded.read(0xA000), 35);
    }

    #[test]
    fn tick_advances_clock() {
        let mut cart = cart(MBC3_RTC);
        (0..1_048_576).for_each(|_| cart.tick());
        cart.write(0, 0x0A);
        cart.write(0x4000, 0x08);
        cart.write(0x6000, 0);
        cart.write(0x6000, 1);
        assert_eq!(cart.read(0xA000), 1);
    }

    #[test]
    fn read_mbc2_ram_is_four_bits_and_mirrored() {
        let mut cart = cart(0x06);
        cart.write(0, 0x0A);
        cart.write(0xA000, 0x35);
        assert_eq!((cart.read(0xA000), cart.read(0xA200)), (0xF5, 0xF5));
        assert_eq!(cart.save_image(0).map(|image| image.len()), Some(512));
    }

    #[test]
    fn write_mbc30_banks_are_distinct() {
        let mut cart = Cartridge::from_bytes(rom(0x13, 5, 2), None).expect("MBC30");
        cart.write(0, 0x0A);
        for bank in 0..8 {
            cart.write(0x4000, bank);
            cart.write(0xA000, bank);
        }
        for bank in 0..8 {
            cart.write(0x4000, bank);
            assert_eq!(cart.read(0xA000), bank);
        }
    }

    #[test]
    fn needs_save_same_value_is_clean() {
        let mut cart = cart(MBC1_BATTERY);
        cart.write(0, 0x0A);
        cart.write(0xA000, 0);
        assert!(!cart.needs_save());
    }

    #[test]
    fn from_bytes_wrong_save_size_is_rejected() {
        for len in [RAM_LEN - 1, RAM_LEN + 1, RAM_LEN * 2] {
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
        assert_eq!(cart.save_image(0), None);
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
    fn read_rtc_without_clock_is_open_bus() {
        let mut cart = cart(0x13);
        cart.write(0, 0x0A);
        cart.write(0x4000, 0x08);
        assert_eq!(cart.read(0xA000), OPEN_BUS);
        cart.write(0xA000, 7);
        assert!(!cart.needs_save());
    }
}
