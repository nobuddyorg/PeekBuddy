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

/// Starts the viewer process (`exe --viewer`) and records the milestone.
/// The viewer's own mutex keeps a second window from opening.
pub fn spawn_viewer(exe: &Path) {
    if let Some(hook) = VIEWER_HOOK.get() {
        hook();
    }
    let _ = Command::new(exe).arg("--viewer").spawn();
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
        assert!(std::fs::metadata(&old).unwrap().len() < 400 + 120);
        // The newest line is in the current file.
        let current = std::fs::read_to_string(&path).unwrap();
        assert!(current.contains("line number 59"), "{current}");
        // Nothing but the two files exists.
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_log_that_cannot_be_written_does_not_panic() {
        let log = LogFile::new(PathBuf::from(r"Z:\no\such\dir\engine.log"), 100);
        log.line("nobody hears this");
    }
}
