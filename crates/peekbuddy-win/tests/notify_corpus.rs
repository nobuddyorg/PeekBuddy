//! Records raw `ReadDirectoryChangesExW` buffers of all three information
//! classes from real NTFS (#44, #270) as seeds for the `notify_parse` fuzz
//! target: one file per operation, `fuzz/corpus/notify_parse/<class>_<op>`,
//! in the format of the target (a head byte that picks the class, then the
//! buffer).
//!
//! The test does nothing unless `PEEKBUDDY_CORPUS_OUT` names the directory to
//! write to; `cargo test -p peekbuddy-win --test notify_corpus` with that
//! variable set regenerates the corpus. The files are committed; the unit test
//! in `peekbuddy-core` parses all of them.
#![cfg(windows)]
#![expect(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns may unwrap"
)]

use std::os::windows::fs::OpenOptionsExt as _;
use std::os::windows::io::AsRawHandle as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_BACKUP_SEMANTICS, ReadDirectoryChangesExW, ReadDirectoryChangesW,
    ReadDirectoryNotifyExtendedInformation, ReadDirectoryNotifyFullInformation,
};

/// `FILE_LIST_DIRECTORY`.
const LIST_DIRECTORY: u32 = 1;
/// `FILE_SHARE_READ | WRITE | DELETE`.
const SHARE_ALL: u32 = 7;
/// The same filter as the engine: name, dir name, attributes, size, last
/// write, creation, security.
const FILTER: u32 = 0x1 | 0x2 | 0x4 | 0x8 | 0x10 | 0x40 | 0x100;

#[derive(Clone, Copy)]
enum Class {
    Plain,
    Extended,
    Full,
}

impl Class {
    const ALL: [(Self, &'static str, u8); 3] = [
        (Self::Plain, "plain", 0),
        (Self::Extended, "extended", 1),
        (Self::Full, "full", 2),
    ];
}

/// One blocking call: the buffer of the first batch of changes the actor makes.
fn read_once(dir: &Path, class: Class) -> Vec<u8> {
    let handle = std::fs::OpenOptions::new()
        .read(true)
        .access_mode(LIST_DIRECTORY)
        .share_mode(SHARE_ALL)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(dir)
        .unwrap();
    let mut buffer = vec![0u8; 64 * 1024];
    let mut returned = 0u32;
    let length = u32::try_from(buffer.len()).unwrap();
    let raw = handle.as_raw_handle();
    let out = buffer.as_mut_ptr().cast();
    let ok = match class {
        Class::Plain => {
            // SAFETY: `raw` is an open directory handle with
            // FILE_LIST_DIRECTORY; `out` points to `length` writable bytes and
            // `returned` is a valid out pointer; no overlapped structure and
            // no completion routine, so the call is synchronous and returns
            // before they go out of scope.
            unsafe {
                ReadDirectoryChangesW(
                    raw,
                    out,
                    length,
                    1,
                    FILTER,
                    &raw mut returned,
                    std::ptr::null_mut(),
                    None,
                )
            }
        }
        Class::Extended => {
            // SAFETY: as above.
            unsafe {
                ReadDirectoryChangesExW(
                    raw,
                    out,
                    length,
                    1,
                    FILTER,
                    &raw mut returned,
                    std::ptr::null_mut(),
                    None,
                    ReadDirectoryNotifyExtendedInformation,
                )
            }
        }
        Class::Full => {
            // SAFETY: as above.
            unsafe {
                ReadDirectoryChangesExW(
                    raw,
                    out,
                    length,
                    1,
                    FILTER,
                    &raw mut returned,
                    std::ptr::null_mut(),
                    None,
                    ReadDirectoryNotifyFullInformation,
                )
            }
        }
    };
    assert_ne!(ok, 0, "{}", std::io::Error::last_os_error());
    buffer.truncate(usize::try_from(returned).unwrap());
    buffer
}

/// An operation the corpus records: a name and what to do in the directory.
type Op = (&'static str, fn(&Path));

fn ops() -> Vec<Op> {
    vec![
        ("create", |dir| {
            std::fs::write(dir.join("new.txt"), b"").unwrap();
        }),
        ("write", |dir| {
            std::fs::write(dir.join("data.bin"), vec![7u8; 100_000]).unwrap();
        }),
        ("rename", |dir| {
            std::fs::rename(dir.join("old.txt"), dir.join("renamed.txt")).unwrap();
        }),
        ("delete", |dir| {
            std::fs::remove_file(dir.join("old.txt")).unwrap();
        }),
        ("mkdir", |dir| std::fs::create_dir(dir.join("sub")).unwrap()),
        ("attrib", |dir| {
            let path = dir.join("old.txt");
            let mut permissions = std::fs::metadata(&path).unwrap().permissions();
            permissions.set_readonly(true);
            std::fs::set_permissions(&path, permissions).unwrap();
        }),
        ("unicode", |dir| {
            std::fs::write(dir.join("\u{C4}rger \u{1F600} \u{4E2D}\u{6587}.txt"), b"x").unwrap();
        }),
        ("long_name", |dir| {
            std::fs::write(dir.join(format!("{}.txt", "n".repeat(200))), b"x").unwrap();
        }),
        ("nested", |dir| {
            std::fs::create_dir_all(dir.join("a").join("b")).unwrap();
            std::fs::write(dir.join("a").join("b").join("deep.txt"), b"x").unwrap();
        }),
        ("hard_link", |dir| {
            std::fs::hard_link(dir.join("old.txt"), dir.join("linked.txt")).unwrap();
        }),
        ("burst", |dir| {
            for index in 0..40 {
                std::fs::write(dir.join(format!("f{index}.txt")), b"x").unwrap();
            }
        }),
    ]
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pb-corpus-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("old.txt"), b"old").unwrap();
    dir
}

#[test]
fn record_the_notify_parse_corpus() {
    let Some(out) = std::env::var_os("PEEKBUDDY_CORPUS_OUT") else {
        return;
    };
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out).unwrap();
    let mut written = 0;
    for (class, name, tag) in Class::ALL {
        for (op, act) in ops() {
            let dir = scratch(&format!("{name}-{op}"));
            let actor_dir = dir.clone();
            let actor = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(300));
                act(&actor_dir);
            });
            let buffer = read_once(&dir, class);
            actor.join().unwrap();
            let _ = std::fs::remove_dir_all(&dir);
            assert!(!buffer.is_empty(), "{name} {op}: an empty buffer");
            // The head byte of the fuzz target: the class in the low two bits,
            // 63/64 of the buffer as the reported size in the rest.
            let mut file = vec![(63 << 2) | tag];
            file.extend_from_slice(&buffer);
            std::fs::write(out.join(format!("{name}_{op}")), file).unwrap();
            written += 1;
        }
    }
    assert_eq!(written, 33);
}

#[test]
fn the_head_byte_picks_the_class_and_nearly_the_whole_buffer() {
    // `fuzz/fuzz_targets/notify_parse.rs` reads `head % 3` as the class and
    // `head >> 2` sixty-fourths of the buffer as the reported size.
    for (_, _, tag) in Class::ALL {
        let head = (63u8 << 2) | tag;
        assert_eq!(head % 3, tag);
        assert_eq!(head >> 2, 63);
    }
}
