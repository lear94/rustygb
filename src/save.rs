//! Crash-safe storage of battery saves next to their ROM.

use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const SAVE_EXTENSION: &str = "sav";
const TEMP_SUFFIX: &str = ".tmp";

/// Error reading or writing a save file.
#[derive(Debug)]
#[non_exhaustive]
pub enum SaveError {
    /// The ROM itself carries the save extension, so saving would destroy it.
    RomIsSave(
        /// Offending ROM path.
        PathBuf,
    ),
    /// The file system rejected the operation.
    Io {
        /// Save path involved.
        path: PathBuf,
        /// Underlying failure.
        source: io::Error,
    },
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RomIsSave(path) => write!(f, "{} is a save file, not a ROM", path.display()),
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
        }
    }
}

impl Error for SaveError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::RomIsSave(_) => None,
            Self::Io { source, .. } => Some(source),
        }
    }
}

/// Save file belonging to one ROM.
#[derive(Debug, Clone)]
pub struct SaveFile {
    path: PathBuf,
}

impl SaveFile {
    /// Locates the save of `rom_path`.
    ///
    /// # Errors
    /// [`SaveError::RomIsSave`] - `rom_path` already ends in `.sav`.
    pub fn for_rom(rom_path: &Path) -> Result<Self, SaveError> {
        let is_save = rom_path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case(SAVE_EXTENSION));
        if is_save {
            return Err(SaveError::RomIsSave(rom_path.to_path_buf()));
        }
        let path = rom_path.with_extension(SAVE_EXTENSION);
        // Through a link, the rename would replace the link and not the save it names.
        let path = fs::canonicalize(&path).unwrap_or(path);
        Ok(Self { path })
    }

    /// Reads the save; `None` when it does not exist yet.
    ///
    /// # Errors
    /// [`SaveError::Io`] - the file exists but cannot be read.
    ///
    /// # Complexity
    /// O(file) time and space.
    pub fn load(&self) -> Result<Option<Vec<u8>>, SaveError> {
        match fs::read(&self.path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(self.io_error(source)),
        }
    }

    /// Replaces the save atomically; a failure leaves the previous one intact.
    ///
    /// # Errors
    /// [`SaveError::Io`] - the data did not reach storage.
    ///
    /// # Complexity
    /// O(image) time.
    pub fn store(&self, image: &[u8]) -> Result<(), SaveError> {
        self.replace(image).map_err(|source| self.io_error(source))
    }

    fn replace(&self, image: &[u8]) -> io::Result<()> {
        if fs::metadata(&self.path).is_ok_and(|save| save.permissions().readonly()) {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        let temp_path = self.temp_path();
        let stored = write_synced(&temp_path, image)
            .and_then(|()| fs::rename(&temp_path, &self.path))
            .and_then(|()| sync_parent(&self.path));
        if stored.is_err() {
            // Best effort: the temp may not exist, and the store error matters more.
            let _ = fs::remove_file(&temp_path);
        }
        stored
    }

    /// Distinct per process, so concurrent instances never share a file.
    fn temp_path(&self) -> PathBuf {
        let mut path = OsString::from(&self.path);
        path.push(format!(".{}{TEMP_SUFFIX}", std::process::id()));
        PathBuf::from(path)
    }

    fn io_error(&self, source: io::Error) -> SaveError {
        SaveError::Io {
            path: self.path.clone(),
            source,
        }
    }
}

fn write_synced(path: &Path, image: &[u8]) -> io::Result<()> {
    // A crashed process with this id may have left its temp behind. Removing it and
    // creating anew never follows a link planted there.
    match fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    let mut file = File::create_new(path)?;
    file.write_all(image)?;
    // Without the sync, the rename may reach disk before the data does.
    file.sync_all()
}

/// Makes a rename durable; Windows does so itself and cannot open directories.
fn sync_parent(path: &Path) -> io::Result<()> {
    match path.parent() {
        Some(parent) if cfg!(unix) => {
            let dir = if parent.as_os_str().is_empty() {
                Path::new(".")
            } else {
                parent
            };
            File::open(dir)?.sync_all()
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn for_rom_replaces_extension() {
        let save = SaveFile::for_rom(Path::new("dir/game.gb")).expect("not a save");
        assert_eq!(save.path, Path::new("dir/game.sav"));
    }

    #[test]
    fn for_rom_save_extension_is_rejected() {
        for path in ["game.sav", "game.SAV"] {
            assert!(matches!(
                SaveFile::for_rom(Path::new(path)),
                Err(SaveError::RomIsSave(_))
            ));
        }
    }

    #[test]
    fn display_names_the_path() {
        let error = SaveError::RomIsSave(PathBuf::from("x.sav"));
        assert_eq!(error.to_string(), "x.sav is a save file, not a ROM");
        assert!(error.source().is_none());
    }
}
