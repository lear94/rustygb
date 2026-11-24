//! Save files on a real file system: atomic replacement has no in-memory equivalent.
#![cfg(not(target_arch = "wasm32"))]
#![allow(
    clippy::expect_used,
    reason = "test fixtures: clippy only exempts #[test] functions"
)]

use rusty_gb::save::{SaveError, SaveFile};
use std::fs;
use std::path::PathBuf;

/// Directory private to one test, removed when the test ends.
struct Sandbox(PathBuf);

impl Sandbox {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("rusty_gb-{}-{name}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp dir is writable");
        Self(dir)
    }

    fn save(&self) -> SaveFile {
        SaveFile::for_rom(&self.0.join("game.gb")).expect("not a save")
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn load_missing_is_none() {
    let dir = Sandbox::new("missing");
    assert_eq!(dir.save().load().expect("missing is not an error"), None);
}

#[test]
fn store_replaces_and_leaves_no_temp() {
    let dir = Sandbox::new("replace");
    for image in [&[1, 2, 3][..], &[4]] {
        dir.save().store(image).expect("writable dir");
        assert_eq!(dir.save().load().expect("readable"), Some(image.to_vec()));
    }
    assert_eq!(fs::read_dir(&dir.0).expect("readable").count(), 1);
}

#[test]
fn store_failure_keeps_previous_save() {
    let dir = Sandbox::new("failure");
    dir.save().store(&[7]).expect("writable dir");
    fs::create_dir(dir.0.join("game.sav.tmp")).expect("blocks the temp file");
    assert!(matches!(dir.save().store(&[8]), Err(SaveError::Io { .. })));
    assert_eq!(dir.save().load().expect("readable"), Some(vec![7]));
}

#[test]
fn load_unreadable_is_an_error() {
    let dir = Sandbox::new("unreadable");
    fs::create_dir(dir.0.join("game.sav")).expect("a directory cannot be read");
    assert!(matches!(dir.save().load(), Err(SaveError::Io { .. })));
}
