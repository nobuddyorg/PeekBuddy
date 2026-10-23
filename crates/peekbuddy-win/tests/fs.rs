//! File-system wrappers and the high-resolution timer (Windows only, #25).
#![cfg(windows)]

use std::os::windows::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use peekbuddy_win::fs::{
    ATTRIBUTE_ARCHIVE, create_junction, file_system_name, short_names_generated, toggle_archive,
};
use peekbuddy_win::timer::HighResTimer;

/// `FILE_ATTRIBUTE_REPARSE_POINT`.
const REPARSE_POINT: u32 = 0x400;

/// A fresh directory for one test, removed on drop.
struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> std::io::Result<Self> {
        let path = std::env::temp_dir().join(format!("pbfs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path)?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_junction_is_a_reparse_point_that_shows_its_target() {
    let dir = Dir::new("junction").unwrap();
    let target = dir.path().join("target");
    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join("probe.txt"), b"x").unwrap();
    let junction = dir.path().join("link");
    create_junction(&junction, &target).unwrap();
    let meta = std::fs::symlink_metadata(&junction).unwrap();
    assert_ne!(meta.file_attributes() & REPARSE_POINT, 0);
    assert!(junction.join("probe.txt").exists());
    // Removing the tree removes the junction, never the target's content
    // through it (F18).
    std::fs::remove_dir_all(&junction).unwrap();
    assert!(target.join("probe.txt").exists());
}

#[test]
fn a_failed_junction_leaves_no_directory() {
    let dir = Dir::new("junction-fail").unwrap();
    let junction = dir.path().join("link");
    std::fs::create_dir(&junction).unwrap();
    // The directory exists already: creation fails and the existing one
    // stays as it was.
    assert!(create_junction(&junction, dir.path()).is_err());
    assert_eq!(
        std::fs::symlink_metadata(&junction)
            .unwrap()
            .file_attributes()
            & REPARSE_POINT,
        0
    );
}

#[test]
fn junctions_work_beyond_max_path_and_with_lone_surrogates() {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt as _;
    let dir = Dir::new("junction-long").unwrap();
    let mut deep = dir.path().to_path_buf();
    for _ in 0..3 {
        deep.push("L".repeat(100));
    }
    std::fs::create_dir_all(&deep).unwrap();
    let odd = OsString::from_wide(&[u16::from(b'j'), 0xD800]);
    let junction = deep.join(odd);
    create_junction(&junction, dir.path()).unwrap();
    assert!(junction.as_os_str().len() > 260);
    let meta = std::fs::symlink_metadata(&junction).unwrap();
    assert_ne!(meta.file_attributes() & REPARSE_POINT, 0);
}

#[test]
fn toggling_the_archive_bit_flips_it_both_ways() {
    let dir = Dir::new("archive").unwrap();
    let file = dir.path().join("f.txt");
    std::fs::write(&file, b"x").unwrap();
    let archive =
        |path: &Path| std::fs::metadata(path).unwrap().file_attributes() & ATTRIBUTE_ARCHIVE;
    let before = archive(&file);
    toggle_archive(&file).unwrap();
    assert_ne!(archive(&file), before);
    toggle_archive(&file).unwrap();
    assert_eq!(archive(&file), before);
    // The file stays writable.
    std::fs::write(&file, b"y").unwrap();
    assert!(toggle_archive(&dir.path().join("missing")).is_err());
}

#[test]
fn the_temp_volume_has_a_file_system_name() {
    let name = file_system_name(&std::env::temp_dir()).unwrap();
    assert!(["NTFS", "ReFS"].contains(&name.as_str()), "{name}");
}

#[test]
fn the_short_name_probe_cleans_up() {
    let dir = Dir::new("short").unwrap();
    short_names_generated(dir.path()).unwrap();
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn the_high_resolution_timer_sleeps_about_as_asked() {
    let timer = HighResTimer::new().unwrap();
    let start = Instant::now();
    // 2 ms, five times.
    for _ in 0..5 {
        timer.sleep(20_000).unwrap();
    }
    let elapsed = start.elapsed();
    assert!(elapsed >= Duration::from_millis(9), "{elapsed:?}");
    // Not the 15.6 ms default tick per sleep.
    assert!(elapsed < Duration::from_millis(70), "{elapsed:?}");
}
