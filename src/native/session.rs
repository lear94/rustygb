//! Running game together with the file that keeps its battery RAM.

use rusty_gb::save::SaveFile;
use rusty_gb::GameBoy;
use std::error::Error;
use std::path::Path;

/// Frames between autosave checks (~10 s): bounds the progress a crash can lose.
const AUTOSAVE_PERIOD_FRAMES: u32 = 600;

pub(super) struct Session {
    pub(super) gb: GameBoy,
    save: SaveFile,
    frames_since_autosave: u32,
}

impl Session {
    pub(super) fn open(path: &Path, sample_rate_hz: u32) -> Result<Self, Box<dyn Error>> {
        let save = SaveFile::for_rom(path)?;
        let rom = std::fs::read(path)?;
        // An unreadable or foreign save must abort the load: continuing with blank RAM
        // would overwrite it.
        let gb = GameBoy::from_rom_with_save(rom, save.load()?, sample_rate_hz)?;
        Ok(Self {
            gb,
            save,
            frames_since_autosave: 0,
        })
    }

    pub(super) fn run_frame(&mut self) {
        self.gb.run_frame();
        self.frames_since_autosave += 1;
        if self.frames_since_autosave >= AUTOSAVE_PERIOD_FRAMES {
            self.frames_since_autosave = 0;
            self.persist();
        }
    }

    /// Returns whether the battery RAM is safe on disk.
    pub(super) fn persist(&mut self) -> bool {
        let cartridge = self.gb.cartridge();
        if !cartridge.needs_save() {
            return true;
        }
        let Some(image) = cartridge.save_image() else {
            return true;
        };
        if let Err(e) = self.save.store(&image) {
            eprintln!("Failed to save game: {e}");
            return false;
        }
        cartridge.mark_saved();
        true
    }
}
