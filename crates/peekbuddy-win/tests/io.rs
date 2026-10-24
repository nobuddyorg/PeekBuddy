//! The IOCP, directory handle and volume primitives against the OS (#43).
#![cfg(windows)]
#![expect(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns may unwrap"
)]

use std::os::windows::ffi::OsStrExt;
use std::time::Duration;

use peekbuddy_win::io::dir::{CancelOutcome, DirHandle};
use peekbuddy_win::io::iocp::{Batch, Iocp};
use peekbuddy_win::io::open::OpenOptions;
use peekbuddy_win::io::path::to_long_path;
use peekbuddy_win::io::slot::IoSlot;
use peekbuddy_win::io::volume::{final_path, volume_identity};
use windows_sys::Win32::Storage::FileSystem::ReadDirectoryChangesW;

const NOTIFY_ALL: u32 = 0x1FF;

fn wide(path: &std::path::Path) -> Vec<u16> {
    path.as_os_str().encode_wide().collect()
}

fn open_dir(path: &std::path::Path) -> (Vec<u16>, DirHandle) {
    let long = to_long_path(&wide(path)).unwrap();
    let dir = DirHandle::open(&long, OpenOptions::default()).unwrap();
    (wide(path), dir)
}

fn start_watch(dir: &DirHandle, slot: &mut IoSlot) {
    use std::os::windows::io::AsRawHandle;
    let p = slot.begin().unwrap();
    // SAFETY: the pointers come from `begin` and stay valid until the slot is
    // finished; the handle is open for overlapped directory listing.
    let ok = unsafe {
        ReadDirectoryChangesW(
            dir.as_raw_handle(),
            p.buffer.cast(),
            p.len,
            1,
            NOTIFY_ALL,
            std::ptr::null_mut(),
            p.overlapped,
            None,
        )
    };
    assert_ne!(ok, 0, "{}", std::io::Error::last_os_error());
}

#[test]
fn posted_message_round_trips() {
    let iocp = Iocp::new(1).unwrap();
    iocp.post(7, 42, 0x1000).unwrap();
    let mut batch = Batch::new(4);
    assert_eq!(
        iocp.dequeue(&mut batch, Some(Duration::from_secs(5)))
            .unwrap(),
        1
    );
    let c = batch.iter().next().unwrap();
    assert_eq!((c.key, c.bytes, c.overlapped), (7, 42, 0x1000));
    assert!(c.is_success());
    assert_eq!(batch.len(), 1);
    assert!(!batch.is_empty());
}

#[test]
fn batch_counts_every_entry() {
    let iocp = Iocp::new(1).unwrap();
    iocp.post(1, 0, 0).unwrap();
    iocp.post(2, 0, 0).unwrap();
    let mut batch = Batch::new(8);
    assert_eq!(
        iocp.dequeue(&mut batch, Some(Duration::from_secs(5)))
            .unwrap(),
        2
    );
    assert_eq!(batch.len(), 2);
    assert_eq!(batch.iter().count(), 2);
}

#[test]
fn a_huge_timeout_returns_when_a_message_is_ready() {
    let iocp = Iocp::new(1).unwrap();
    iocp.post(3, 0, 0).unwrap();
    let mut batch = Batch::new(1);
    assert_eq!(iocp.dequeue(&mut batch, Some(Duration::MAX)).unwrap(), 1);
    iocp.post(4, 0, 0).unwrap();
    assert_eq!(iocp.dequeue(&mut batch, None).unwrap(), 1);
}

#[test]
fn slot_state_is_observable() {
    let mut slot = IoSlot::new(4096).unwrap();
    assert_eq!(slot.len(), 4096);
    assert!(!slot.is_empty());
    assert!(!slot.is_pending());
    assert_eq!(slot.data(4096).map(<[u8]>::len), Some(4096));
    assert!(slot.data(4097).is_none());
    slot.begin().unwrap();
    assert!(slot.is_pending());
    assert!(slot.data(0).is_none());
    slot.finish().unwrap();
    assert!(!slot.is_pending());
}

#[test]
fn cancel_without_io_is_already_completed() {
    let tmp = tempdir();
    let (_, dir) = open_dir(&tmp);
    let slot = IoSlot::new(4096).unwrap();
    assert_eq!(dir.cancel(&slot).unwrap(), CancelOutcome::AlreadyCompleted);
}

#[test]
fn dequeue_times_out_with_zero() {
    let iocp = Iocp::new(1).unwrap();
    let mut batch = Batch::new(4);
    assert_eq!(
        iocp.dequeue(&mut batch, Some(Duration::from_millis(20)))
            .unwrap(),
        0
    );
    assert!(batch.is_empty());
}

#[test]
fn directory_change_arrives_on_the_port() {
    let tmp = tempdir();
    let (_, dir) = open_dir(&tmp);
    let iocp = Iocp::new(1).unwrap();
    iocp.associate(&dir, 99).unwrap();
    let mut slot = IoSlot::new(64 * 1024).unwrap();
    start_watch(&dir, &mut slot);
    std::fs::write(tmp.join("hello.txt"), b"x").unwrap();
    let mut batch = Batch::new(8);
    assert!(
        iocp.dequeue(&mut batch, Some(Duration::from_secs(10)))
            .unwrap()
            >= 1
    );
    let c = batch.iter().next().unwrap();
    assert_eq!(c.key, 99);
    assert_eq!(c.overlapped, slot.overlapped_addr());
    assert!(c.is_success());
    assert!(c.bytes > 0);
    slot.finish().unwrap();
    assert_eq!(slot.data(c.bytes as usize).unwrap().len(), c.bytes as usize);
}

#[test]
fn cancel_completes_with_operation_aborted() {
    let tmp = tempdir();
    let (_, dir) = open_dir(&tmp);
    let iocp = Iocp::new(1).unwrap();
    iocp.associate(&dir, 1).unwrap();
    let mut slot = IoSlot::new(4096).unwrap();
    start_watch(&dir, &mut slot);
    assert_eq!(dir.cancel(&slot).unwrap(), CancelOutcome::Requested);
    let mut batch = Batch::new(4);
    assert_eq!(
        iocp.dequeue(&mut batch, Some(Duration::from_secs(10)))
            .unwrap(),
        1
    );
    let c = batch.iter().next().unwrap();
    assert_eq!(
        c.status,
        peekbuddy_win::io::error::WinError::OperationAborted
    );
    assert!(!c.is_success());
    slot.finish().unwrap();
}

#[test]
fn volume_identity_and_final_path() {
    let tmp = tempdir();
    let (path, dir) = open_dir(&tmp);
    let id = volume_identity(&dir, &path).unwrap();
    assert!(
        id.guid_path
            .starts_with(&wide(std::path::Path::new(r"\\?\Volume{")))
    );
    assert_ne!(id.file_system.len(), 0);
    let fin = final_path(&dir).unwrap();
    assert_ne!(fin.len(), 0);
    // The normalised DOS name of a local directory is `\\?\X:\...`.
    let prefix: Vec<u16> = r"\\?\".encode_utf16().collect();
    assert!(fin.starts_with(&prefix));
}

fn tempdir() -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(
        format!(
            "peekbuddy-io-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        )
        .replace(['(', ')'], ""),
    );
    std::fs::create_dir_all(&p).unwrap();
    p
}
