//! Engine instrumentation (#162): the readiness gate, the viewer-spawn
//! milestone and the size-capped log file.
//!
//! Milestones are recorded with [`mark`]: once in the probe's table and once
//! to ETW, with the same timestamp. [`ReadyGate`] makes `EngineReady` mean
//! what F7 says: the tray attempt is done and every watch is armed (or
//! failed, which is a logged state and not a reason to wait forever).

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use peekbuddy_core::perf::{Milestone, MilestoneTable, Role};
use peekbuddy_win::perf::{record, since_process_create_us};

/// Records `milestone` for the engine role.
pub fn mark(table: &MilestoneTable, milestone: Milestone) {
    record(table, milestone, Role::Engine, since_process_create_us());
}

/// Fires once when `count` things have been done.
pub struct ReadyGate {
    remaining: AtomicUsize,
    fire: Box<dyn Fn() + Send + Sync>,
}

impl std::fmt::Debug for ReadyGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReadyGate")
            .field("remaining", &self.remaining.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl ReadyGate {
    /// A gate that waits for `count` calls of [`ReadyGate::done`]; with a
    /// count of 0 it fires at once.
    #[must_use]
    pub fn new(count: usize, fire: impl Fn() + Send + Sync + 'static) -> Arc<Self> {
        let gate = Arc::new(Self {
            remaining: AtomicUsize::new(count),
            fire: Box::new(fire),
        });
        if count == 0 {
            (gate.fire)();
        }
        gate
    }

    /// One thing is done. The call that brings the count to zero fires; any
    /// call after that does nothing.
    pub fn done(&self) {
        let before = self
            .remaining
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_sub(1));
        if before == Ok(1) {
            (self.fire)();
        }
    }
}

/// What to do right before the viewer is started: the `ViewerSpawn`
/// milestone. Set once by the engine.
static VIEWER_HOOK: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

/// Sets the hook that runs before every viewer spawn (first call wins).
pub fn on_viewer_spawn(hook: impl Fn() + Send + Sync + 'static) {
    let _ = VIEWER_HOOK.set(Box::new(hook));
}

/// The arguments that start the viewer of this engine: `--viewer`, and for
/// an engine with an explicit data directory (tests, the perf harness) that
/// directory too, so the viewer belongs to this engine and never to the
/// user's own.
#[must_use]
pub fn viewer_args(data_dir: Option<&Path>) -> Vec<std::ffi::OsString> {
    let mut args = vec![std::ffi::OsString::from("--viewer")];
    if let Some(dir) = data_dir {
        args.push(std::ffi::OsString::from("--data-dir"));
        args.push(dir.as_os_str().to_owned());
    }
    args
}

/// Starts the viewer process (`exe` with `args`, see [`viewer_args`]) and
/// records the milestone. The viewer's own mutex keeps a second window from
/// opening.
pub fn spawn_viewer(exe: &Path, args: &[std::ffi::OsString]) {
    if let Some(hook) = VIEWER_HOOK.get() {
        hook();
    }
    // A window needs no standard handles; handing the engine's down would
    // also keep a pipe of whoever started the engine open for as long as the
    // viewer lives.
    let _ = Command::new(exe)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// The longest the log file grows before it is rotated.
pub const LOG_CAP_BYTES: u64 = 512 * 1024;

/// A log file that never grows past [`LOG_CAP_BYTES`]: when full, it becomes
/// `<name>.1` (the old `.1` is dropped) and a new file starts. Lines only, no
/// timers; written when something is worth remembering.
#[derive(Debug)]
pub struct LogFile {
    path: PathBuf,
    cap: u64,
    inner: Mutex<Option<std::fs::File>>,
}

impl LogFile {
    /// A log at `path`, capped at `cap` bytes.
    #[must_use]
    pub const fn new(path: PathBuf, cap: u64) -> Self {
        Self {
            path,
            cap,
            inner: Mutex::new(None),
        }
    }

    fn rotate_path(&self) -> PathBuf {
        let mut name = self.path.clone().into_os_string();
        name.push(".1");
        PathBuf::from(name)
    }

    /// Appends one line, stamped with the UTC time. Failures are ignored: a
    /// log that cannot be written must never stop the engine.
    pub fn line(&self, text: &str) {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let size = guard
            .as_ref()
            .and_then(|file| file.metadata().ok())
            .map_or(0, |meta| meta.len());
        if guard.is_some() && size >= self.cap {
            *guard = None;
            let _ = std::fs::remove_file(self.rotate_path());
            let _ = std::fs::rename(&self.path, self.rotate_path());
        }
        if guard.is_none() {
            // A file left by the last run may already be over the cap.
            if std::fs::metadata(&self.path).is_ok_and(|meta| meta.len() >= self.cap) {
                let _ = std::fs::remove_file(self.rotate_path());
                let _ = std::fs::rename(&self.path, self.rotate_path());
            }
            *guard = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)
                .ok();
        }
        if let Some(file) = guard.as_mut() {
            let stamp = crate::crash::utc_stamp(std::time::SystemTime::now());
            let _ = writeln!(file, "{stamp} {text}");
        }
    }
}

static LOG: OnceLock<LogFile> = OnceLock::new();

/// Starts the engine log in `logs_dir` (first call wins).
pub fn init_log(logs_dir: &Path) {
    let _ = LOG.set(LogFile::new(logs_dir.join("engine.log"), LOG_CAP_BYTES));
}

/// Writes one line to the engine log, if it was started.
pub fn log(text: &str) {
    if let Some(file) = LOG.get() {
        file.line(text);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicU32;

    use super::*;

    #[test]
    fn the_gate_fires_once_after_the_last_call() {
        let fired = Arc::new(AtomicU32::new(0));
        let counter = Arc::clone(&fired);
        let gate = ReadyGate::new(3, move || {
            counter.fetch_add(1, Ordering::Relaxed);
        });
        gate.done();
        gate.done();
        assert_eq!(fired.load(Ordering::Relaxed), 0);
        gate.done();
        assert_eq!(fired.load(Ordering::Relaxed), 1);
        gate.done();
        gate.done();
        assert_eq!(fired.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn a_gate_of_nothing_fires_at_once() {
        let fired = Arc::new(AtomicU32::new(0));
        let counter = Arc::clone(&fired);
        let gate = ReadyGate::new(0, move || {
            counter.fetch_add(1, Ordering::Relaxed);
        });
        gate.done();
        assert_eq!(fired.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn the_log_rotates_at_its_cap_and_keeps_one_old_file() {
        let dir = std::env::temp_dir().join(format!("pb-log-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("engine.log");
        let log = LogFile::new(path.clone(), 400);
        for i in 0..60 {
            log.line(&format!("line number {i} with some text"));
        }
        let old = dir.join("engine.log.1");
        assert!(old.exists());
        assert!(std::fs::metadata(&path).unwrap().len() < 400 + 120);
        // Rotated at the cap, not at every line.
        let old_len = std::fs::metadata(&old).unwrap().len();
        assert!((400..400 + 120).contains(&old_len), "{old_len}");
        // The newest line is in the current file.
        let current = std::fs::read_to_string(&path).unwrap();
        assert!(current.contains("line number 59"), "{current}");
        // Nothing but the two files exists.
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_file_left_by_the_last_run_is_kept_under_the_cap_and_rotated_over_it() {
        let dir = std::env::temp_dir().join(format!("pb-log-left-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("engine.log");
        let old = dir.join("engine.log.1");

        std::fs::write(&path, "from the last run\n").unwrap();
        LogFile::new(path.clone(), 400).line("this run");
        let current = std::fs::read_to_string(&path).unwrap();
        assert!(current.starts_with("from the last run\n"), "{current}");
        assert!(current.contains("this run"), "{current}");
        assert!(!old.exists());

        std::fs::write(&path, "x".repeat(400)).unwrap();
        LogFile::new(path.clone(), 400).line("after a full one");
        assert_eq!(std::fs::read_to_string(&old).unwrap(), "x".repeat(400));
        let current = std::fs::read_to_string(&path).unwrap();
        assert!(
            current.contains("after a full one") && !current.contains('x'),
            "{current}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_engine_log_is_written_once_started() {
        let dir = std::env::temp_dir().join(format!("pb-log-init-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        init_log(&dir);
        log("a line for the engine log");
        let text = std::fs::read_to_string(dir.join("engine.log")).unwrap();
        assert!(text.contains("a line for the engine log"), "{text}");
    }

    #[test]
    fn the_viewer_of_a_test_engine_gets_its_data_dir() {
        assert_eq!(viewer_args(None), ["--viewer"]);
        assert_eq!(
            viewer_args(Some(Path::new(r"C:\t\data"))),
            ["--viewer", "--data-dir", r"C:\t\data"]
        );
    }

    #[test]
    fn a_viewer_spawn_runs_the_hook_and_starts_the_process() {
        let calls = Arc::new(AtomicU32::new(0));
        let counter = Arc::clone(&calls);
        on_viewer_spawn(move || {
            counter.fetch_add(1, Ordering::Relaxed);
        });
        // Any process will do: `cmd /c mkdir` leaves a mark to wait for.
        let mark = std::env::temp_dir().join(format!("pb-spawn-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&mark);
        let cmd =
            std::env::var_os("ComSpec").map_or_else(|| PathBuf::from("cmd.exe"), PathBuf::from);
        spawn_viewer(
            &cmd,
            &["/c".into(), "mkdir".into(), mark.clone().into_os_string()],
        );
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !mark.exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "the process did not run"
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        std::fs::remove_dir_all(&mark).unwrap();
    }

    #[test]
    fn a_log_that_cannot_be_written_does_not_panic() {
        let log = LogFile::new(PathBuf::from(r"Z:\no\such\dir\engine.log"), 100);
        log.line("nobody hears this");
    }
}
