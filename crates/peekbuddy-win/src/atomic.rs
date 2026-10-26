//! Atomic file replacement for small files such as `settings.toml` (#62).
//!
//! The new content goes to `<name>.tmp` in the same directory (a replace
//! across volumes is not atomic), is flushed with `FlushFileBuffers`, and
//! then replaces the target with `ReplaceFileW`, or `MoveFileExW` when there
//! is no target yet. A failure at any point leaves the old file untouched.
//! Transient failures (an antivirus scanner holding a handle) are retried
//! with a short backoff, then reported.

use std::io::Write as _;
use std::os::windows::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_LOCK_VIOLATION, ERROR_SHARING_VIOLATION,
    ERROR_UNABLE_TO_MOVE_REPLACEMENT, ERROR_UNABLE_TO_MOVE_REPLACEMENT_2,
    ERROR_UNABLE_TO_REMOVE_REPLACED,
};
use windows_sys::Win32::Storage::FileSystem::{
    MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW, ReplaceFileW,
};

/// How many times a transient failure is tried again.
const ATTEMPTS: u32 = 8;
/// First pause; it doubles up to [`MAX_PAUSE`].
const FIRST_PAUSE: Duration = Duration::from_millis(10);
const MAX_PAUSE: Duration = Duration::from_millis(160);

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

fn is_transient(error: &std::io::Error) -> bool {
    error.raw_os_error().is_some_and(|code| {
        [
            ERROR_SHARING_VIOLATION,
            ERROR_LOCK_VIOLATION,
            ERROR_ACCESS_DENIED,
            ERROR_UNABLE_TO_MOVE_REPLACEMENT,
            ERROR_UNABLE_TO_MOVE_REPLACEMENT_2,
            ERROR_UNABLE_TO_REMOVE_REPLACED,
        ]
        .iter()
        .any(|known| u32::try_from(code).is_ok_and(|code| code == *known))
    })
}

fn temp_path(target: &Path) -> PathBuf {
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    target.with_file_name(name)
}

fn replace_once(target: &Path, temp: &Path) -> std::io::Result<()> {
    let (target_w, temp_w) = (wide(target), wide(temp));
    if target.exists() {
        // SAFETY: both paths are NUL-terminated UTF-16 that outlive the call;
        // no backup name, flags or reserved pointers are passed.
        let ok = unsafe {
            ReplaceFileW(
                target_w.as_ptr(),
                temp_w.as_ptr(),
                std::ptr::null(),
                0,
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        if ok != 0 {
            return Ok(());
        }
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: both paths are NUL-terminated UTF-16 that outlive the call.
    let ok = unsafe {
        MoveFileExW(
            temp_w.as_ptr(),
            target_w.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Replaces `target` with `bytes` atomically.
///
/// # Errors
///
/// The I/O error that stopped the write. The old content of `target` is
/// intact whenever this returns an error, and the temporary file is removed.
pub fn write_atomic(target: &Path, bytes: &[u8]) -> std::io::Result<()> {
    write_atomic_with(target, bytes, || Ok(()))
}

/// Like [`write_atomic`], calling `before_replace` after the temporary file
/// is durable and before it replaces the target (fault injection in tests).
///
/// # Errors
///
/// See [`write_atomic`]; an error from `before_replace` aborts the write.
pub fn write_atomic_with(
    target: &Path,
    bytes: &[u8],
    before_replace: impl FnOnce() -> std::io::Result<()>,
) -> std::io::Result<()> {
    let temp = temp_path(target);
    let result = (|| {
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(bytes)?;
        // FlushFileBuffers: the bytes are on disk before the name changes.
        file.sync_all()?;
        drop(file);
        before_replace()?;
        let mut pause = FIRST_PAUSE;
        for attempt in 1..=ATTEMPTS {
            match replace_once(target, &temp) {
                Ok(()) => return Ok(()),
                Err(error) if attempt < ATTEMPTS && is_transient(&error) => {
                    std::thread::sleep(pause);
                    pause = (pause * 2).min(MAX_PAUSE);
                }
                Err(error) => return Err(error),
            }
        }
        Err(std::io::Error::other("replace retries exhausted"))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use std::os::windows::fs::OpenOptionsExt as _;
    use std::time::Instant;

    use super::{temp_path, write_atomic, write_atomic_with};

    fn dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("pb-atomic-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_new_file_is_created_and_an_old_one_replaced() {
        let dir = dir("basic");
        let target = dir.join("settings.toml");
        write_atomic(&target, b"first").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"first");
        write_atomic(&target, b"second, longer").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"second, longer");
        assert!(!temp_path(&target).exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failure_before_the_replace_leaves_the_old_file_intact() {
        let dir = dir("fault");
        let target = dir.join("settings.toml");
        write_atomic(&target, b"old").unwrap();
        let error = write_atomic_with(&target, b"new", || {
            Err(std::io::Error::other("injected failure"))
        })
        .unwrap_err();
        assert_eq!(error.to_string(), "injected failure");
        assert_eq!(std::fs::read(&target).unwrap(), b"old");
        assert!(
            !temp_path(&target).exists(),
            "the temp file was left behind"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_held_target_is_retried_then_reported_and_stays_intact() {
        let dir = dir("held");
        let target = dir.join("settings.toml");
        write_atomic(&target, b"old").unwrap();
        // Another handle without sharing keeps ReplaceFileW from succeeding.
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&target)
            .unwrap();
        let started = Instant::now();
        let error = write_atomic(&target, b"new").unwrap_err();
        assert!(super::is_transient(&error), "{error:?}");
        assert!(started.elapsed().as_millis() >= 100, "no retry happened");
        drop(held);
        assert_eq!(std::fs::read(&target).unwrap(), b"old");
        assert!(!temp_path(&target).exists());
        // Once released, the same write goes through.
        write_atomic(&target, b"new").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
