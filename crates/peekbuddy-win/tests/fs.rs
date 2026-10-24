//! File-system wrappers and the high-resolution timer (Windows only, #25).
#![cfg(windows)]

use std::os::windows::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use std::io::Write as _;
use std::os::windows::ffi::{OsStrExt as _, OsStringExt as _};

use peekbuddy_core::name::CaseMode;
use peekbuddy_win::fs::{
    ATTRIBUTE_ARCHIVE, NO_FOLLOW_NO_RECALL, SHARE_ALL, case_mode, create_junction, file_id,
    file_system_name, open_directory, resolve_long_name, short_names_generated, toggle_archive,
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

/// Resolves `path` through its file id on the volume of `dir`.
fn resolve(dir: &Path, path: &Path) -> std::io::Result<Vec<u16>> {
    let hint = open_directory(dir)?;
    let mut out = vec![0_u16; 1024];
    let len = resolve_long_name(&hint, file_id(path)?, &mut out)?;
    out.truncate(len);
    Ok(out)
}

#[test]
fn a_file_id_resolves_to_the_long_name() {
    let dir = Dir::new("resolve").unwrap();
    let path = dir.path().join("PeekBuddy resolution probe.txt");
    std::fs::write(&path, b"x").unwrap();
    let resolved = String::from_utf16(&resolve(dir.path(), &path).unwrap()).unwrap();
    assert!(resolved.starts_with('\\'), "volume-relative: {resolved}");
    assert!(
        resolved.ends_with("\\PeekBuddy resolution probe.txt"),
        "{resolved}"
    );
}

#[test]
fn a_lone_surrogate_name_survives_resolution_exactly() {
    let dir = Dir::new("surrogate").unwrap();
    let units: Vec<u16> = "lone-"
        .encode_utf16()
        .chain([0xD800])
        .chain("-end.txt".encode_utf16())
        .collect();
    let path = dir.path().join(std::ffi::OsString::from_wide(&units));
    std::fs::write(&path, b"x").unwrap();
    let resolved = resolve(dir.path(), &path).unwrap();
    assert!(resolved.ends_with(&units), "{resolved:04X?}");
    // The directory listing carries the same exact units.
    let listed: Vec<Vec<u16>> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().encode_wide().collect())
        .collect();
    assert_eq!(listed, vec![units]);
}

#[test]
fn a_deleted_file_does_not_resolve() {
    let dir = Dir::new("gone").unwrap();
    let path = dir.path().join("gone.txt");
    std::fs::write(&path, b"x").unwrap();
    let id = file_id(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    let hint = open_directory(dir.path()).unwrap();
    let mut out = vec![0_u16; 1024];
    assert!(resolve_long_name(&hint, id, &mut out).is_err());
}

#[test]
fn a_small_buffer_reports_insufficient_buffer() {
    let dir = Dir::new("small").unwrap();
    let path = dir.path().join("PeekBuddy small buffer probe.txt");
    std::fs::write(&path, b"x").unwrap();
    let hint = open_directory(dir.path()).unwrap();
    let mut out = [0_u16; 4];
    let error = resolve_long_name(&hint, file_id(&path).unwrap(), &mut out).unwrap_err();
    assert_eq!(error.raw_os_error(), Some(122));
}

#[test]
fn directories_are_case_insensitive_unless_flagged() {
    let dir = Dir::new("case").unwrap();
    assert_eq!(case_mode(dir.path()).unwrap(), CaseMode::Insensitive);
    let status = std::process::Command::new("fsutil.exe")
        .args(["file", "setCaseSensitiveInfo"])
        .arg(dir.path())
        .arg("enable")
        .output();
    match status {
        Ok(output) if output.status.success() => {
            assert_eq!(case_mode(dir.path()).unwrap(), CaseMode::Sensitive);
        }
        other => {
            let _ = writeln!(
                std::io::stderr(),
                "skipped: fsutil could not enable case sensitivity: {other:?}"
            );
        }
    }
}

#[test]
fn the_flag_literals_match_windows_sys() {
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_NO_RECALL, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };
    assert_eq!(
        SHARE_ALL,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE
    );
    assert_eq!(
        NO_FOLLOW_NO_RECALL,
        FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_OPEN_NO_RECALL
    );
}
