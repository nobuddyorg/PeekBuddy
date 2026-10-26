//! Pause and resume as the engine runs them (#63).
//!
//! [`PauseCtl`] ties the pure switchboard ([`Pause`]) to the settings and the
//! store: a command flips the switches, every effective change becomes a row,
//! and a per-watch pause is written to `settings.toml` (`enabled = false`) so
//! it survives a restart. A global pause is not persisted: a restart resumes.
//!
//! The tray, the viewer's pipe commands and the capture thread share it. The
//! capture thread only reads the lock-free [`Pause`]; the lock here is for the
//! control path.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use peekbuddy_core::event::{Event, RootIdx, VolIdx};
use peekbuddy_core::pause::{Pause, Transition};
use peekbuddy_core::settings::{Settings, to_toml};
use peekbuddy_store::pipeline::Producer;

/// Where state rows go.
pub trait RowSink: Send {
    /// Stores one row (a state change has no name).
    fn row(&mut self, event: Event);
}

/// Rows into the writer queue.
#[derive(Debug)]
pub struct ProducerRows(pub Producer);

impl RowSink for ProducerRows {
    fn row(&mut self, event: Event) {
        let mut batch = self.0.new_batch();
        if batch.push(event, &[]).is_ok() {
            let _ = self.0.push(batch);
        }
    }
}

/// Reads the two clocks for a row: wall time (FILETIME ticks) and the
/// monotonic count.
pub type Clock = fn() -> (u64, u64);

/// Wall time (FILETIME ticks) and a monotonic count, now.
#[must_use]
pub fn now_ticks() -> (u64, u64) {
    use std::sync::OnceLock;
    use std::time::{Instant, SystemTime, UNIX_EPOCH};

    /// 100 ns ticks from 1601-01-01 to 1970-01-01.
    const UNIX_EPOCH_TICKS: u64 = 116_444_736_000_000_000;
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    let wall = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_nanos() / 100).unwrap_or(u64::MAX));
    let mono = u64::try_from(EPOCH.get_or_init(Instant::now).elapsed().as_nanos()).unwrap_or(0);
    (UNIX_EPOCH_TICKS.saturating_add(wall), mono)
}

struct Inner {
    settings: Settings,
    settings_path: PathBuf,
    /// False for a file from a newer build: it is never written.
    writable: bool,
    rows: Box<dyn RowSink>,
    clock: Clock,
}

/// Pause control for the watches of the settings.
pub struct PauseCtl {
    pause: Arc<Pause>,
    inner: Mutex<Inner>,
}

impl std::fmt::Debug for PauseCtl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PauseCtl")
            .field("global", &self.pause.global())
            .finish_non_exhaustive()
    }
}

fn root_of(index: usize) -> RootIdx {
    RootIdx(u16::try_from(index).unwrap_or(u16::MAX))
}

impl PauseCtl {
    /// Control over the watches of `settings`. Watches with `enabled = false`
    /// start paused (their rows were written when they were paused).
    #[must_use]
    pub fn new(
        settings: Settings,
        settings_path: PathBuf,
        writable: bool,
        rows: Box<dyn RowSink>,
        clock: Clock,
    ) -> Arc<Self> {
        let pause = Arc::new(Pause::new());
        for (index, watch) in settings.watches.iter().enumerate() {
            if !watch.enabled {
                pause.set_watch(root_of(index), true);
            }
        }
        Arc::new(Self {
            pause,
            inner: Mutex::new(Inner {
                settings,
                settings_path,
                writable,
                rows,
                clock,
            }),
        })
    }

    /// The switches capture reads.
    #[must_use]
    pub fn pause(&self) -> Arc<Pause> {
        Arc::clone(&self.pause)
    }

    /// Logs the watches that start paused, one row each (F19: a watch is
    /// never silently dead).
    pub fn announce_start(&self) {
        let Ok(mut inner) = self.inner.lock() else {
            return;
        };
        let transitions: Vec<Transition> = (0..inner.settings.watches.len())
            .map(root_of)
            .filter(|&root| self.pause.watch_paused(root))
            .map(|root| Transition {
                root,
                state: peekbuddy_core::event::WatchState::SuspendedByUser,
                missed: 0,
                lower_bound: false,
            })
            .collect();
        inner.emit(&transitions);
    }

    /// Pauses every watch.
    pub fn pause_all(&self) {
        self.set_all(true);
    }

    /// Resumes every watch that is not paused on its own.
    pub fn resume_all(&self) {
        self.set_all(false);
    }

    fn set_all(&self, paused: bool) {
        let Ok(mut inner) = self.inner.lock() else {
            return;
        };
        let roots: Vec<RootIdx> = (0..inner.settings.watches.len()).map(root_of).collect();
        let transitions = self.pause.set_global(paused, &roots);
        inner.emit(&transitions);
    }

    /// Pauses or resumes the watch `id` and persists the choice.
    ///
    /// # Errors
    ///
    /// A message for an unknown id, or when `settings.toml` cannot be written
    /// (the pause itself still took effect).
    pub fn set_watch(&self, id: &str, paused: bool) -> Result<(), String> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| String::from("pause control is unavailable"))?;
        let index = inner
            .settings
            .watches
            .iter()
            .position(|watch| watch.id == id)
            .ok_or_else(|| format!("no watch with id `{id}`"))?;
        let transition = self.pause.set_watch(root_of(index), paused);
        inner.emit(transition.as_slice());
        if let Some(watch) = inner.settings.watches.get_mut(index) {
            watch.enabled = !paused;
        }
        inner.persist()
    }

    /// Pauses or resumes the watch at `index` (the tray's numbering).
    ///
    /// # Errors
    ///
    /// As [`PauseCtl::set_watch`].
    pub fn set_watch_at(&self, index: usize, paused: bool) -> Result<(), String> {
        let id = {
            let inner = self
                .inner
                .lock()
                .map_err(|_| String::from("pause control is unavailable"))?;
            inner
                .settings
                .watches
                .get(index)
                .map(|watch| watch.id.clone())
                .ok_or_else(|| format!("no watch at position {index}"))?
        };
        self.set_watch(&id, paused)
    }
}

impl Inner {
    fn emit(&mut self, transitions: &[Transition]) {
        let (ts, qpc) = (self.clock)();
        for transition in transitions {
            self.rows.row(transition.to_event(VolIdx(0), ts, qpc));
        }
    }

    fn persist(&self) -> Result<(), String> {
        if !self.writable {
            return Err(String::from(
                "settings.toml is from a newer version and is not written; the pause lasts until the engine restarts",
            ));
        }
        let text = to_toml(&self.settings)?;
        peekbuddy_win::atomic::write_atomic(&self.settings_path, text.as_bytes())
            .map_err(|error| format!("could not write settings.toml: {error}"))
    }
}

#[cfg(test)]
mod tests;
