//! Exporting the view (#215): the rows of the current filter, streamed to CSV
//! or JSON Lines by the store's exporter on a thread of its own.
//!
//! The file goes to `<data dir>\exports\`. That folder is inside the data
//! directory, which is always excluded from watching (F16), so writing the
//! export never makes an event of its own. The store writes
//! `<name>.partial` and renames it when it is complete; cancelling, closing
//! the viewer or a failed write (a full disk) deletes the partial file.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use peekbuddy_core::export::{Format, Options};
use peekbuddy_core::query::CancelFlag;
use peekbuddy_store::export::{ExportError, Job, Outcome, Progress, export_file};
use peekbuddy_store::read::Reader;

use crate::rows::OffsetFn;

/// How an export stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// Writing.
    Running,
    /// The file is complete under its final name.
    Done {
        /// Rows written.
        rows: u64,
        /// Where the file is.
        path: PathBuf,
    },
    /// Cancelled; nothing is left behind.
    Cancelled,
    /// Failed; the partial file is gone.
    Failed(String),
}

/// The extension of a format.
#[must_use]
pub const fn extension(format: Format) -> &'static str {
    match format {
        Format::Csv => "csv",
        Format::JsonLines => "jsonl",
    }
}

/// The file name of an export started at `stamp` (text safe for a file name).
#[must_use]
pub fn file_name(stamp: &str, format: Format) -> String {
    let safe: String = stamp
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' {
                ch
            } else {
                '-'
            }
        })
        .collect();
    format!("peekbuddy-{safe}.{}", extension(format))
}

/// A target that does not exist yet: `name.ext`, else `name (2).ext`, ...
#[must_use]
pub fn free_target(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name, ""));
    // A thousand tries are plenty; then the name is reused (the store never
    // overwrites a finished file: the rename fails instead).
    (2..1_000)
        .map(|n| dir.join(format!("{stem} ({n}).{ext}")))
        .find(|candidate| !candidate.exists())
        .unwrap_or(first)
}

/// A running or finished export.
#[derive(Debug)]
pub struct Export {
    state: Arc<Mutex<State>>,
    progress: Arc<Progress>,
    cancel: CancelFlag,
    format: Format,
    done: Arc<AtomicU8>,
}

impl Export {
    /// Starts exporting `job` to `target`. The folder is created.
    ///
    /// # Errors
    ///
    /// The error from creating the folder or the thread.
    pub fn start(
        segments: &Path,
        target: PathBuf,
        job: Job,
        offset: Arc<OffsetFn>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> std::io::Result<Self> {
        if let Some(dir) = target.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let state = Arc::new(Mutex::new(State::Running));
        let progress = Arc::new(Progress::default());
        let cancel = CancelFlag::new();
        let done = Arc::new(AtomicU8::new(0));
        let format = job.format;
        let (state_t, progress_t, cancel_t, done_t) = (
            Arc::clone(&state),
            Arc::clone(&progress),
            cancel.clone(),
            Arc::clone(&done),
        );
        let reader = Reader::new(segments);
        std::thread::Builder::new()
            .name(String::from("export"))
            .spawn(move || {
                let result = export_file(
                    &reader,
                    &job,
                    &target,
                    &|ts| offset(ts),
                    &cancel_t,
                    &progress_t,
                );
                let next = match result {
                    Ok(Outcome::Done { rows }) => State::Done { rows, path: target },
                    Ok(Outcome::Cancelled) => State::Cancelled,
                    Err(error @ (ExportError::Io(_) | ExportError::Read(_))) => {
                        State::Failed(error.to_string())
                    }
                };
                *state_t.lock().unwrap_or_else(PoisonError::into_inner) = next;
                done_t.store(1, Ordering::Release);
                wake();
            })?;
        Ok(Self {
            state,
            progress,
            cancel,
            format,
            done,
        })
    }

    /// How it stands.
    #[must_use]
    pub fn state(&self) -> State {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Whether the thread has finished.
    #[must_use]
    pub fn finished(&self) -> bool {
        self.done.load(Ordering::Acquire) == 1
    }

    /// Rows written so far.
    #[must_use]
    pub fn written(&self) -> u64 {
        self.progress.written()
    }

    /// An upper bound on the rows to write.
    #[must_use]
    pub fn estimated(&self) -> u64 {
        self.progress.estimated()
    }

    /// The format being written.
    #[must_use]
    pub const fn format(&self) -> Format {
        self.format
    }

    /// Stops the export at its next step; the partial file is removed.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }
}

/// The default options of an export: no byte order mark, no Excel guard
/// (both alter the data and are off by default).
#[must_use]
pub fn default_options() -> Options {
    Options::default()
}

#[cfg(test)]
#[expect(clippy::panic, reason = "a test panics on an unexpected state")]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use peekbuddy_core::export::Format;
    use peekbuddy_core::query::Filter;
    use peekbuddy_store::export::Job;

    use super::{Export, State, default_options, extension, file_name, free_target};

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pb-export-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn job(format: Format) -> Job {
        Job {
            filter: Filter::default(),
            format,
            options: default_options(),
        }
    }

    fn wait(export: &Export) -> State {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !export.finished() {
            assert!(Instant::now() < deadline, "the export did not finish");
            std::thread::sleep(Duration::from_millis(5));
        }
        export.state()
    }

    #[test]
    fn names_are_safe_and_never_overwrite() {
        assert_eq!(
            file_name("2026-10-26 14:00:00.123", Format::Csv),
            "peekbuddy-2026-10-26-14-00-00-123.csv"
        );
        assert_eq!(extension(Format::JsonLines), "jsonl");
        let dir = dir("names");
        let first = free_target(&dir, "a.csv");
        assert_eq!(first, dir.join("a.csv"));
        std::fs::write(&first, "x").unwrap();
        assert_eq!(free_target(&dir, "a.csv"), dir.join("a (2).csv"));
        std::fs::write(dir.join("a (2).csv"), "x").unwrap();
        assert_eq!(free_target(&dir, "a.csv"), dir.join("a (3).csv"));
    }

    #[test]
    fn an_export_of_an_empty_log_writes_the_header_and_ends_done() {
        let dir = dir("empty");
        // The engine creates the segments folder before anything else.
        std::fs::create_dir_all(dir.join("segments")).unwrap();
        let target = dir.join("exports").join("log.csv");
        let wakes = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let counter = Arc::clone(&wakes);
        let export = Export::start(
            &dir.join("segments"),
            target.clone(),
            job(Format::Csv),
            Arc::new(|_| 0),
            Arc::new(move || {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }),
        )
        .unwrap();
        match wait(&export) {
            State::Done { rows, path } => {
                assert_eq!(rows, 0);
                assert_eq!(path, target);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(wakes.load(std::sync::atomic::Ordering::SeqCst), 1);
        let text = std::fs::read_to_string(&target).unwrap();
        assert!(text.starts_with("id,ts_utc"), "{text}");
        assert!(!target.with_extension("csv.partial").exists());
    }

    #[test]
    fn a_target_that_cannot_be_written_fails_and_leaves_nothing() {
        let dir = dir("fail");
        // The "folder" is a file: creating the export folder fails.
        let blocker = dir.join("exports");
        std::fs::write(&blocker, "not a folder").unwrap();
        let result = Export::start(
            &dir.join("segments"),
            blocker.join("log.csv"),
            job(Format::JsonLines),
            Arc::new(|_| 0),
            Arc::new(|| {}),
        );
        assert!(result.is_err());
    }
}
