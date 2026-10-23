//! Shared test helpers for PeekBuddy (dev-only).
//!
//! - [`TestDir`]: a temporary root under a short path that runs its cleanup
//!   hooks (stop and join watchers) before it deletes itself, and retries the
//!   deletion with backoff, because an open handle blocks deletion on Windows.
//! - [`snapshot_filters`]: insta filters that make snapshots authored on
//!   Linux pass on Windows.
//! - `run_unelevated` and `assert_interactive_session` (Windows only).
#![forbid(unsafe_code)]

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

static NEXT: AtomicU32 = AtomicU32::new(0);

/// A cleanup hook that runs before the directory is removed.
type Hook = Box<dyn FnOnce() + Send>;

/// A temporary directory for one test.
pub struct TestDir {
    path: PathBuf,
    hooks: Vec<Hook>,
    removed: bool,
}

impl std::fmt::Debug for TestDir {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestDir")
            .field("path", &self.path)
            .field("hooks", &self.hooks.len())
            .finish_non_exhaustive()
    }
}

/// The short base directory all test roots live under.
#[must_use]
pub fn base_dir() -> PathBuf {
    std::env::var_os("PEEKBUDDY_TEST_ROOT")
        .map_or_else(|| std::env::temp_dir().join("pbt"), PathBuf::from)
}

impl TestDir {
    /// Creates a fresh, unique directory `<base>/<name>-<pid>-<n>`.
    ///
    /// # Errors
    ///
    /// Returns the I/O error when the directory cannot be created.
    pub fn new(name: &str) -> io::Result<Self> {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = base_dir().join(format!("{name}-{}-{n}", std::process::id()));
        Self::create(path)
    }

    /// Creates the fixed directory `<base>/<name>`, removing stale state a
    /// killed earlier run left behind. Use it only from tests in the
    /// `fs-serial` group: two tests sharing a fixed root must never overlap.
    ///
    /// # Errors
    ///
    /// Returns the I/O error when stale state cannot be removed or the
    /// directory cannot be created.
    pub fn fixed(name: &str) -> io::Result<Self> {
        let path = base_dir().join(name);
        if path.exists() {
            remove_dir_all_with_retry(&path)?;
        }
        Self::create(path)
    }

    fn create(path: PathBuf) -> io::Result<Self> {
        std::fs::create_dir_all(&path)?;
        Ok(Self {
            path,
            hooks: Vec::new(),
            removed: false,
        })
    }

    /// The directory's path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Registers a hook that runs before removal, in reverse order of
    /// registration (stop and join a watcher here).
    pub fn on_cleanup(&mut self, hook: impl FnOnce() + Send + 'static) {
        self.hooks.push(Box::new(hook));
    }

    /// Runs the hooks and removes the directory, reporting failures.
    ///
    /// # Errors
    ///
    /// Returns the last I/O error when the directory is still present after
    /// all retries.
    pub fn close(mut self) -> io::Result<()> {
        self.cleanup()
    }

    fn cleanup(&mut self) -> io::Result<()> {
        while let Some(hook) = self.hooks.pop() {
            hook();
        }
        if self.removed {
            return Ok(());
        }
        self.removed = true;
        remove_dir_all_with_retry(&self.path)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        // Drop cannot report; tests that care call `close`.
        let _ = self.cleanup();
    }
}

/// Removes `path` recursively, retrying with exponential backoff (10 ms up
/// to about 2.5 s in total) while something still holds a handle in it.
///
/// # Errors
///
/// Returns the last error when the directory is still present after all
/// retries.
pub fn remove_dir_all_with_retry(path: &Path) -> io::Result<()> {
    let mut delay = Duration::from_millis(10);
    let mut last = None;
    for _ in 0..9 {
        match std::fs::remove_dir_all(path) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => last = Some(error),
        }
        std::thread::sleep(delay);
        delay = delay.saturating_mul(2);
    }
    Err(last.unwrap_or_else(|| io::Error::other("removal failed")))
}

/// insta filters that make a snapshot independent of the OS: backslashes
/// become slashes, drive letters disappear, CRLF becomes LF and RFC 3339
/// timestamps become `[timestamp]`.
#[must_use]
pub fn snapshot_filters() -> Vec<(&'static str, &'static str)> {
    vec![
        (r"\r\n", "\n"),
        (r"\b[A-Z]:\\", "/"),
        (r"\\\\", "/"),
        (r"\\", "/"),
        (
            r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})",
            "[timestamp]",
        ),
    ]
}

/// Runs `command_line` as an unelevated child (Administrators deny-only,
/// Medium integrity, no console) and returns its exit code (F12: hosted
/// runners run elevated).
///
/// # Errors
///
/// Returns the OS error when the child cannot be started.
#[cfg(windows)]
pub fn run_unelevated(command_line: &str) -> io::Result<u32> {
    peekbuddy_win::process::run_restricted(command_line)
}

/// Panics unless the test runs in an interactive session (not session 0).
///
/// # Panics
///
/// When the process runs in session 0 or the session cannot be determined.
#[cfg(windows)]
pub fn assert_interactive_session() {
    match peekbuddy_win::session::current_session_id() {
        Ok(0) => unreachable_session("process runs in session 0"),
        Ok(_) => {}
        Err(error) => unreachable_session(&format!("session lookup failed: {error}")),
    }
}

#[cfg(windows)]
#[expect(clippy::panic, reason = "a test-only assertion helper")]
fn unreachable_session(why: &str) {
    panic!("not an interactive session: {why}");
}

#[cfg(test)]
mod tests {
    use super::{TestDir, snapshot_filters};
    use std::time::Duration;

    #[test]
    fn new_directories_are_unique_and_removed() {
        let a = TestDir::new("unique").unwrap();
        let b = TestDir::new("unique").unwrap();
        assert_ne!(a.path(), b.path());
        let path = a.path().to_owned();
        std::fs::write(path.join("f.txt"), b"x").unwrap();
        a.close().unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn hooks_run_before_removal_in_reverse_order() {
        let mut dir = TestDir::new("hooks").unwrap();
        let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        for i in 0..3 {
            let log = log.clone();
            let path = dir.path().to_owned();
            dir.on_cleanup(move || log.lock().unwrap().push((i, path.exists())));
        }
        dir.close().unwrap();
        assert_eq!(*log.lock().unwrap(), vec![(2, true), (1, true), (0, true)]);
    }

    #[test]
    fn removal_waits_for_a_handle_held_by_another_thread() {
        let dir = TestDir::new("held").unwrap();
        let file = dir.path().join("held.txt");
        std::fs::write(&file, b"x").unwrap();
        let (opened, wait) = std::sync::mpsc::channel();
        let holder = std::thread::spawn(move || {
            let mut options = std::fs::OpenOptions::new();
            options.read(true);
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt as _;
                // No FILE_SHARE_DELETE: deletion fails while this is open.
                options.share_mode(0);
            }
            let handle = options.open(&file).unwrap();
            opened.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(300));
            drop(handle);
        });
        wait.recv().unwrap();
        let path = dir.path().to_owned();
        dir.close().unwrap();
        holder.join().unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn fixed_removes_stale_state() {
        let stale = super::base_dir().join("fixed-stale");
        std::fs::create_dir_all(&stale).unwrap();
        std::fs::write(stale.join("old.txt"), b"x").unwrap();
        let dir = TestDir::fixed("fixed-stale").unwrap();
        assert!(!dir.path().join("old.txt").exists());
    }

    #[test]
    fn snapshot_is_os_independent() {
        let windows = "C:\\Users\\me\\watch\\a.txt\r\nmodified 2026-10-23T09:52:28.123Z\r\n";
        let linux = "/Users/me/watch/a.txt\nmodified 2026-10-23T09:52:28Z\n";
        for input in [windows, linux] {
            insta::with_settings!({ filters => snapshot_filters() }, {
                insta::assert_snapshot!("os_independent_event", input);
            });
        }
    }
}
