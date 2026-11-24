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
fn store_failure_leaves_no_temp() {
    let dir = Sandbox::new("failure");
    fs::create_dir(dir.0.join("game.sav")).expect("blocks the rename");
    assert!(matches!(dir.save().store(&[8]), Err(SaveError::Io { .. })));
    assert_eq!(fs::read_dir(&dir.0).expect("readable").count(), 1);
}

#[cfg(unix)]
#[test]
fn store_unwritable_dir_keeps_previous_save() {
    use std::os::unix::fs::PermissionsExt;
    let dir = Sandbox::new("unwritable");
    dir.save().store(&[7]).expect("writable dir");
    fs::set_permissions(&dir.0, fs::Permissions::from_mode(0o500)).expect("owned");
    let stored = dir.save().store(&[8]);
    fs::set_permissions(&dir.0, fs::Permissions::from_mode(0o700)).expect("owned");
    // Root ignores the mode, and then the store simply succeeds.
    let expected = if stored.is_ok() { 8 } else { 7 };
    assert_eq!(dir.save().load().expect("readable"), Some(vec![expected]));
}

#[test]
fn store_concurrent_saves_do_not_mix() {
    let dir = Sandbox::new("concurrent");
    std::thread::scope(|scope| {
        for id in 0..4u8 {
            let save = SaveFile::for_rom(&dir.0.join(format!("{id}.gb"))).expect("not a save");
            scope.spawn(move || {
                for _ in 0..50 {
                    save.store(&[id; 4096]).expect("writable dir");
                    assert_eq!(save.load().expect("readable"), Some(vec![id; 4096]));
                }
            });
        }
    });
}

#[test]
fn load_unreadable_is_an_error() {
    let dir = Sandbox::new("unreadable");
    fs::create_dir(dir.0.join("game.sav")).expect("a directory cannot be read");
    assert!(matches!(dir.save().load(), Err(SaveError::Io { .. })));
}

#[test]
fn store_read_only_save_is_refused() {
    let dir = Sandbox::new("readonly");
    dir.save().store(&[7]).expect("writable dir");
    let path = dir.0.join("game.sav");
    let mut permissions = fs::metadata(&path).expect("exists").permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&path, permissions).expect("owned file");
    assert!(dir.save().store(&[8]).is_err());
    assert_eq!(dir.save().load().expect("readable"), Some(vec![7]));
}

#[cfg(unix)]
#[test]
fn store_through_link_updates_target() {
    let dir = Sandbox::new("link");
    let target = dir.0.join("cloud.bin");
    fs::write(&target, [1]).expect("writable dir");
    std::os::unix::fs::symlink(&target, dir.0.join("game.sav")).expect("unix");
    dir.save().store(&[2]).expect("writable dir");
    assert_eq!(fs::read(&target).expect("readable"), [2]);
}

#[cfg(unix)]
#[test]
fn for_rom_dangling_link_is_an_error() {
    let dir = Sandbox::new("dangling");
    std::os::unix::fs::symlink(dir.0.join("gone"), dir.0.join("game.sav")).expect("unix");
    assert!(matches!(
        SaveFile::for_rom(&dir.0.join("game.gb")),
        Err(SaveError::Io { .. })
    ));
}

#[cfg(unix)]
#[test]
fn store_keeps_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let dir = Sandbox::new("mode");
    dir.save().store(&[1]).expect("writable dir");
    let path = dir.0.join("game.sav");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("owned");
    dir.save().store(&[2]).expect("writable dir");
    let mode = fs::metadata(&path).expect("exists").permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[test]
fn store_longest_name_succeeds() {
    let dir = Sandbox::new("long");
    let rom = dir.0.join(format!("{}.gb", "a".repeat(246)));
    let save = SaveFile::for_rom(&rom).expect("not a save");
    if cfg!(unix) {
        save.store(&[1]).expect("255-byte names are valid");
        assert_eq!(save.load().expect("readable"), Some(vec![1]));
    }
}
