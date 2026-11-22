//! Hash on demand (#85): the SHA-256 of the file at a row's path, computed
//! when the user asks, on a thread of its own.
//!
//! The file is hashed as it is now, not as it was when the row was written.
//! The reading is a [`HashFn`] the app passes in (`peekbuddy_win::hash` on
//! Windows): it uses a fixed buffer, can be cancelled and refuses a cloud
//! placeholder, so asking for a hash never downloads a file (F18).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use peekbuddy_core::filehash::{Outcome, hex};

/// Reads and hashes a file: `(path, cancelled, progress)`.
pub type HashFn =
    dyn Fn(&Path, &dyn Fn() -> bool, &dyn Fn(u64)) -> std::io::Result<Outcome> + Send + Sync;

/// How a hash stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// Reading.
    Running,
    /// The SHA-256 in lowercase hex.
    Done(String),
    /// Stopped by the user.
    Cancelled,
    /// The file's data is not on this machine; it was not read.
    Placeholder,
    /// The file could not be read (gone, moved, no access).
    Failed(String),
}

/// A running or finished hash of the file of one row.
#[derive(Debug)]
pub struct Hashing {
    row: i64,
    state: Arc<Mutex<State>>,
    read: Arc<AtomicU64>,
    cancel: Arc<AtomicBool>,
}

impl Hashing {
    /// Starts hashing `path` for the row `row`; `wake` asks the window to
    /// draw again when it ends.
    ///
    /// # Errors
    ///
    /// The error from starting the thread.
    pub fn start(
        row: i64,
        path: PathBuf,
        hash: Arc<HashFn>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> std::io::Result<Self> {
        let state = Arc::new(Mutex::new(State::Running));
        let read = Arc::new(AtomicU64::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (state_t, read_t, cancel_t) =
            (Arc::clone(&state), Arc::clone(&read), Arc::clone(&cancel));
        std::thread::Builder::new()
            .name(String::from("hash"))
            .spawn(move || {
                let outcome = hash(&path, &|| cancel_t.load(Ordering::Relaxed), &|done| {
                    read_t.store(done, Ordering::Relaxed);
                });
                let next = match outcome {
                    Ok(Outcome::Done(digest)) => State::Done(hex(&digest)),
                    Ok(Outcome::Cancelled) => State::Cancelled,
                    Ok(Outcome::Placeholder) => State::Placeholder,
                    Err(error) => State::Failed(error.to_string()),
                };
                *state_t.lock().unwrap_or_else(PoisonError::into_inner) = next;
                wake();
            })?;
        Ok(Self {
            row,
            state,
            read,
            cancel,
        })
    }

    /// The row whose file is hashed.
    #[must_use]
    pub const fn row(&self) -> i64 {
        self.row
    }

    /// How it stands.
    #[must_use]
    pub fn state(&self) -> State {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Bytes read so far.
    #[must_use]
    pub fn read(&self) -> u64 {
        self.read.load(Ordering::Relaxed)
    }

    /// Stops at the next buffer.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::{Duration, Instant};

    use peekbuddy_core::filehash::Outcome;

    use super::{HashFn, Hashing, State};

    fn wait(hashing: &Hashing) -> State {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let state = hashing.state();
            if state != State::Running {
                return state;
            }
            assert!(Instant::now() < deadline, "the hash did not finish");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn start(
        hash: impl Fn(&Path, &dyn Fn() -> bool, &dyn Fn(u64)) -> std::io::Result<Outcome>
        + Send
        + Sync
        + 'static,
    ) -> (Hashing, Arc<AtomicU32>) {
        let wakes = Arc::new(AtomicU32::new(0));
        let counter = Arc::clone(&wakes);
        let hash: Arc<HashFn> = Arc::new(hash);
        let hashing = Hashing::start(
            7,
            PathBuf::from(r"C:\w\a.txt"),
            hash,
            Arc::new(move || {
                counter.fetch_add(1, Ordering::SeqCst);
            }),
        )
        .unwrap();
        (hashing, wakes)
    }

    #[test]
    fn a_finished_hash_is_shown_in_hex_and_wakes_the_window_once() {
        let (hashing, wakes) = start(|path, _, progress| {
            assert_eq!(path, Path::new(r"C:\w\a.txt"));
            progress(3);
            Ok(Outcome::Done([0xAB; 32]))
        });
        assert_eq!(hashing.row(), 7);
        assert_eq!(wait(&hashing), State::Done("ab".repeat(32)));
        assert_eq!(hashing.read(), 3);
        assert_eq!(wakes.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_cancel_reaches_the_reader() {
        let (hashing, _) = start(|_, cancelled, _| {
            let deadline = Instant::now() + Duration::from_secs(20);
            while !cancelled() {
                assert!(Instant::now() < deadline, "the cancel never came");
                std::thread::sleep(Duration::from_millis(1));
            }
            Ok(Outcome::Cancelled)
        });
        hashing.cancel();
        assert_eq!(wait(&hashing), State::Cancelled);
    }

    #[test]
    fn a_placeholder_and_an_error_are_said_as_such() {
        let (placeholder, _) = start(|_, _, _| Ok(Outcome::Placeholder));
        assert_eq!(wait(&placeholder), State::Placeholder);
        let (missing, _) = start(|_, _, _| {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "not there",
            ))
        });
        assert_eq!(wait(&missing), State::Failed(String::from("not there")));
    }
}
