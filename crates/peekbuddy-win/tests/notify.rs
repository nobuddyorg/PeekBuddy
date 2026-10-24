//! The notify parser against the OS: layout constants and real buffers (#44).
#![cfg(windows)]
#![expect(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns may unwrap"
)]

use std::mem::{offset_of, size_of};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::AsRawHandle;
use std::time::Duration;

use peekbuddy_core::notify::{Class, layout, parse};
use peekbuddy_win::io::dir::DirHandle;
use peekbuddy_win::io::iocp::{Batch, Iocp};
use peekbuddy_win::io::open::OpenOptions;
use peekbuddy_win::io::path::to_long_path;
use peekbuddy_win::io::slot::IoSlot;
use windows_sys::Win32::Storage::FileSystem::{
    FILE_NOTIFY_EXTENDED_INFORMATION, FILE_NOTIFY_INFORMATION, ReadDirectoryChangesExW,
};
use windows_sys::Win32::System::SystemServices::FILE_NOTIFY_FULL_INFORMATION;

const NOTIFY_ALL: u32 = 0x1FF;

#[test]
fn plain_layout_matches_windows_sys() {
    assert_eq!(
        offset_of!(FILE_NOTIFY_INFORMATION, NextEntryOffset),
        layout::NEXT_ENTRY_OFFSET
    );
    assert_eq!(offset_of!(FILE_NOTIFY_INFORMATION, Action), layout::ACTION);
    assert_eq!(
        offset_of!(FILE_NOTIFY_INFORMATION, FileNameLength),
        layout::PLAIN_NAME_LENGTH
    );
    assert_eq!(
        offset_of!(FILE_NOTIFY_INFORMATION, FileName),
        layout::PLAIN_NAME
    );
    assert_eq!(layout::PLAIN_NAME, Class::Plain.header_len());
}

#[test]
fn extended_layout_matches_windows_sys() {
    type S = FILE_NOTIFY_EXTENDED_INFORMATION;
    assert_eq!(offset_of!(S, NextEntryOffset), layout::NEXT_ENTRY_OFFSET);
    assert_eq!(offset_of!(S, Action), layout::ACTION);
    assert_eq!(offset_of!(S, CreationTime), layout::CREATION_TIME);
    assert_eq!(offset_of!(S, LastModificationTime), layout::LAST_WRITE_TIME);
    assert_eq!(offset_of!(S, LastChangeTime), layout::CHANGE_TIME);
    assert_eq!(offset_of!(S, LastAccessTime), layout::ACCESS_TIME);
    assert_eq!(offset_of!(S, AllocatedLength), layout::ALLOCATED_LENGTH);
    assert_eq!(offset_of!(S, FileSize), layout::FILE_SIZE);
    assert_eq!(offset_of!(S, FileAttributes), layout::FILE_ATTRIBUTES);
    assert_eq!(offset_of!(S, Anonymous), layout::REPARSE_TAG);
    assert_eq!(offset_of!(S, FileId), layout::FILE_ID);
    assert_eq!(offset_of!(S, ParentFileId), layout::PARENT_FILE_ID);
    assert_eq!(offset_of!(S, FileNameLength), layout::EXTENDED_NAME_LENGTH);
    assert_eq!(offset_of!(S, FileName), layout::EXTENDED_NAME);
    assert_eq!(layout::EXTENDED_NAME, Class::Extended.header_len());
}

#[test]
fn full_layout_matches_windows_sys() {
    type S = FILE_NOTIFY_FULL_INFORMATION;
    assert_eq!(offset_of!(S, NextEntryOffset), layout::NEXT_ENTRY_OFFSET);
    assert_eq!(offset_of!(S, Action), layout::ACTION);
    assert_eq!(offset_of!(S, CreationTime), layout::CREATION_TIME);
    assert_eq!(offset_of!(S, FileAttributes), layout::FILE_ATTRIBUTES);
    assert_eq!(offset_of!(S, Anonymous), layout::REPARSE_TAG);
    assert_eq!(offset_of!(S, FileId), layout::FILE_ID);
    assert_eq!(offset_of!(S, ParentFileId), layout::PARENT_FILE_ID);
    assert_eq!(offset_of!(S, FileNameLength), layout::EXTENDED_NAME_LENGTH);
    assert_eq!(offset_of!(S, FileNameFlags), layout::FULL_NAME_FLAGS);
    assert_eq!(offset_of!(S, FileName), layout::EXTENDED_NAME);
    assert_eq!(layout::EXTENDED_NAME, Class::Full.header_len());
    assert_eq!(
        size_of::<FILE_NOTIFY_EXTENDED_INFORMATION>(),
        size_of::<S>()
    );
}

fn tempdir(tag: &str) -> std::path::PathBuf {
    let mut p = std::env::var_os("RUNNER_TEMP").map_or_else(std::env::temp_dir, Into::into);
    p.push(format!("peekbuddy-notify-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&p).unwrap();
    p
}

/// Watches `dir` with `info_class`, runs `act`, and returns the bytes of the
/// first completion; `None` when the OS rejects the class.
fn capture(dir: &std::path::Path, info_class: i32, act: impl FnOnce()) -> Option<Vec<u8>> {
    let wide: Vec<u16> = dir.as_os_str().encode_wide().collect();
    let handle = DirHandle::open(&to_long_path(&wide).unwrap(), OpenOptions::default()).unwrap();
    let iocp = Iocp::new(1).unwrap();
    iocp.associate(&handle, 1).unwrap();
    let mut slot = IoSlot::new(64 * 1024).unwrap();
    let p = slot.begin().unwrap();
    // SAFETY: the pointers come from `begin` and stay valid until the slot is
    // finished; the handle is open for overlapped directory listing.
    let ok = unsafe {
        ReadDirectoryChangesExW(
            handle.as_raw_handle(),
            p.buffer.cast(),
            p.len,
            1,
            NOTIFY_ALL,
            std::ptr::null_mut(),
            p.overlapped,
            None,
            info_class,
        )
    };
    if ok == 0 {
        slot.finish().unwrap();
        return None;
    }
    act();
    let mut batch = Batch::new(4);
    assert!(
        iocp.dequeue(&mut batch, Some(Duration::from_secs(10)))
            .unwrap()
            >= 1
    );
    let c = batch.iter().next().unwrap();
    assert!(c.is_success(), "{:?}", c.status);
    slot.finish().unwrap();
    Some(slot.data(c.bytes as usize).unwrap().to_vec())
}

fn exercise(class: Class, info_class: i32, tag: &str) {
    let dir = tempdir(tag);
    let target = dir.clone();
    let Some(bytes) = capture(&dir, info_class, move || {
        std::fs::write(target.join("héllo wörld.txt"), b"data").unwrap();
    }) else {
        // Only the full class (Windows 11 22H2+) may be missing.
        assert_eq!(class, Class::Full, "class {info_class} rejected");
        return;
    };
    let mut names = Vec::new();
    for item in parse(&bytes, bytes.len(), class) {
        let record = item.unwrap();
        assert_eq!(record.extended.is_some(), class != Class::Plain);
        names.push(String::from_utf16_lossy(
            &record.name.units().collect::<Vec<_>>(),
        ));
    }
    assert!(
        names.iter().any(|n| n.starts_with("h\u{e9}llo")),
        "{names:?}"
    );
    // Cutting the buffer inside a record is reported, never a panic.
    let cut = bytes.len() - 1;
    let last = parse(&bytes, cut, class).last();
    assert!(matches!(last, Some(Ok(_) | Err(_))));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn real_plain_buffers_parse() {
    exercise(Class::Plain, 1, "plain");
}

#[test]
fn real_extended_buffers_parse() {
    exercise(Class::Extended, 2, "extended");
}

#[test]
fn real_full_buffers_parse() {
    exercise(Class::Full, 3, "full");
}
