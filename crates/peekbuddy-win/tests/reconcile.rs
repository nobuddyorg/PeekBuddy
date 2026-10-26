//! The directory lister against the OS (#46).
#![cfg(windows)]
#![expect(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns may unwrap"
)]

use std::collections::BTreeMap;
use std::mem::offset_of;
use std::os::windows::ffi::OsStrExt;

use peekbuddy_core::reconcile::dirinfo::{self, layout};
use peekbuddy_core::reconcile::{DirSource, ListError, Listing};
use peekbuddy_win::io::path::to_long_path;
use peekbuddy_win::reconcile::{BackgroundMode, DirLister};
use windows_sys::Win32::Storage::FileSystem::FILE_ID_EXTD_DIR_INFO;

fn wide(path: &std::path::Path) -> Vec<u16> {
    path.as_os_str().encode_wide().collect()
}

fn tempdir(tag: &str) -> std::path::PathBuf {
    let mut p = std::env::var_os("RUNNER_TEMP").map_or_else(std::env::temp_dir, Into::into);
    p.push(format!("peekbuddy-reconcile-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn the_layout_matches_windows_sys() {
    type S = FILE_ID_EXTD_DIR_INFO;
    assert_eq!(offset_of!(S, NextEntryOffset), layout::NEXT_ENTRY_OFFSET);
    assert_eq!(offset_of!(S, FileIndex), layout::FILE_INDEX);
    assert_eq!(offset_of!(S, CreationTime), layout::CREATION_TIME);
    assert_eq!(offset_of!(S, LastAccessTime), layout::ACCESS_TIME);
    assert_eq!(offset_of!(S, LastWriteTime), layout::WRITE_TIME);
    assert_eq!(offset_of!(S, ChangeTime), layout::CHANGE_TIME);
    assert_eq!(offset_of!(S, EndOfFile), layout::END_OF_FILE);
    assert_eq!(offset_of!(S, AllocationSize), layout::ALLOCATION_SIZE);
    assert_eq!(offset_of!(S, FileAttributes), layout::ATTRIBUTES);
    assert_eq!(offset_of!(S, FileNameLength), layout::NAME_LENGTH);
    assert_eq!(offset_of!(S, EaSize), layout::EA_SIZE);
    assert_eq!(offset_of!(S, ReparsePointTag), layout::REPARSE_TAG);
    assert_eq!(offset_of!(S, FileId), layout::FILE_ID);
    assert_eq!(offset_of!(S, FileName), layout::NAME);
    assert_eq!(dirinfo::HEADER_LEN, layout::NAME);
    assert_eq!(dirinfo::ATTR_DIRECTORY, 0x10);
    assert_eq!(dirinfo::ATTR_REPARSE_POINT, 0x400);
}

fn list(lister: &mut DirLister, path: &str) -> Result<BTreeMap<String, (u64, bool)>, ListError> {
    let mut out = Listing::new();
    let units: Vec<u16> = path.encode_utf16().collect();
    lister.list(&units, &mut out)?;
    Ok(out
        .items()
        .iter()
        .map(|item| {
            (
                String::from_utf16_lossy(out.name(item)),
                (item.meta.size, item.meta.is_dir()),
            )
        })
        .collect())
}

#[test]
fn a_listing_matches_the_directory_and_skips_dots() {
    let dir = tempdir("list");
    std::fs::write(dir.join("a.txt"), b"12345").unwrap();
    std::fs::write(dir.join("héllo wörld.txt"), b"x").unwrap();
    std::fs::create_dir(dir.join("sub")).unwrap();
    std::fs::write(dir.join("sub").join("deep.txt"), b"1234567890").unwrap();
    let mut lister = DirLister::new(&to_long_path(&wide(&dir)).unwrap());

    let root = list(&mut lister, "").unwrap();
    assert_eq!(
        root.into_iter().collect::<Vec<_>>(),
        [
            (String::from("a.txt"), (5, false)),
            (String::from("héllo wörld.txt"), (1, false)),
            (String::from("sub"), (0, true)),
        ]
    );
    let sub = list(&mut lister, "sub").unwrap();
    assert_eq!(
        sub.into_iter().collect::<Vec<_>>(),
        [(String::from("deep.txt"), (10, false))]
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_big_directory_needs_more_than_one_buffer() {
    let dir = tempdir("big");
    for i in 0..1500 {
        std::fs::write(
            dir.join(format!("a-rather-long-file-name-number-{i:05}.txt")),
            b"",
        )
        .unwrap();
    }
    let mut lister = DirLister::new(&to_long_path(&wide(&dir)).unwrap());
    let all = list(&mut lister, "").unwrap();
    assert_eq!(all.len(), 1500);
    // And the same lister lists again from the start.
    assert_eq!(list(&mut lister, "").unwrap().len(), 1500);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_missing_directory_is_gone() {
    let dir = tempdir("gone");
    let mut lister = DirLister::new(&to_long_path(&wide(&dir)).unwrap());
    assert_eq!(list(&mut lister, "nope"), Err(ListError::Gone));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn background_mode_can_be_entered_and_left() {
    let guard = BackgroundMode::enter();
    // A hosted runner may refuse; entering twice must not break either way.
    let _ = guard.is_active();
    drop(guard);
    let again = BackgroundMode::enter();
    drop(again);
}
