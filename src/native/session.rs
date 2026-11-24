//! Running game together with the file that keeps its battery RAM.

use rusty_gb::save::SaveFile;
use rusty_gb::GameBoy;
use std::error::Error;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

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
        let mut gb = GameBoy::from_rom_with_save(rom, save.load()?, sample_rate_hz)?;
        gb.cartridge().sync_clock(unix_now_s());
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
        let Some(image) = cartridge.save_image(unix_now_s()) else {
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

/// Wall-clock time; a clock set before 1970 counts as the epoch.
fn unix_now_s() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

#[cfg(test)]
mod tests {
    //! The session is glue between the core and the file system, so these tests use both.

    use super::*;
    use std::path::PathBuf;

    /// Enables RAM, stores 0x5A and loops.
    const PROGRAM: [u8; 12] = [
        0x3E, 0x0A, 0xEA, 0x00, 0x00, 0x3E, 0x5A, 0xEA, 0x00, 0xA0, 0x18, 0xFE,
    ];

    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("rgb-{}-{name}", std::process::id()));
            std::fs::create_dir_all(&dir).expect("temp dir is writable");
            let mut rom = vec![0; 0x8000];
            rom[0x100..0x103].copy_from_slice(&[0xC3, 0x50, 0x01]);
            rom[0x147] = 3;
            rom[0x149] = 2;
            rom[0x150..0x150 + PROGRAM.len()].copy_from_slice(&PROGRAM);
            std::fs::write(dir.join("game.gb"), rom).expect("writable");
            Self(dir)
        }

        fn open(&self) -> Result<Session, Box<dyn Error>> {
            Session::open(&self.0.join("game.gb"), 48_000)
        }

        fn saved(&self) -> Option<u8> {
            let save = std::fs::read(self.0.join("game.sav")).ok()?;
            save.first().copied()
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn persist_writes_changes_only() {
        let dir = Sandbox::new("persist");
        let mut session = dir.open().expect("valid ROM");
        assert!(session.persist());
        assert_eq!(dir.saved(), None);
        session.run_frame();
        assert!(session.persist());
        assert_eq!(dir.saved(), Some(0x5A));
    }

    #[test]
    fn run_frame_autosaves_each_period() {
        let dir = Sandbox::new("autosave");
        let mut session = dir.open().expect("valid ROM");
        (1..AUTOSAVE_PERIOD_FRAMES).for_each(|_| session.run_frame());
        assert_eq!(dir.saved(), None);
        session.run_frame();
        assert_eq!(dir.saved(), Some(0x5A));
    }

    #[test]
    fn persist_failure_stays_dirty() {
        let dir = Sandbox::new("failure");
        let mut session = dir.open().expect("valid ROM");
        session.run_frame();
        std::fs::create_dir(dir.0.join("game.sav")).expect("blocks the save");
        assert!(!session.persist());
        std::fs::remove_dir(dir.0.join("game.sav")).expect("empty");
        assert!(session.persist());
        assert_eq!(dir.saved(), Some(0x5A));
    }

    #[test]
    fn open_foreign_save_is_refused_and_kept() {
        let dir = Sandbox::new("foreign");
        std::fs::write(dir.0.join("game.sav"), [9; 100]).expect("writable");
        assert!(dir.open().is_err());
        assert_eq!(dir.saved(), Some(9));
    }
}
