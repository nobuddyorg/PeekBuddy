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
//!
//! It also owns the list of watches the viewer edits (#68). The capture host
//! arms a fixed set of roots at start, and a root's position in the settings
//! is its index. So an added watch is saved and starts capturing with the
//! next engine start (`armed = false` until then), and a removed watch that
//! is armed keeps its position, is paused at once and disappears from the
//! list and from the file; the position is reused by nothing until a restart.

use std::collections::BTreeSet;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use peekbuddy_core::event::{Event, RootIdx, VolIdx, WatchState};
use peekbuddy_core::ipc::WatchInfo;
use peekbuddy_core::pause::{Pause, Transition};
use peekbuddy_core::settings::edit::{self, Added, Refusal, Removed};
use peekbuddy_core::settings::{Context, Settings, to_toml};
use peekbuddy_store::pipeline::Producer;

use crate::capture_host::Signaller;

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
    /// How many watches the capture host armed at start: positions below
    /// this are root indexes, later ones wait for a restart.
    started: usize,
    /// Armed watches the user removed; hidden, paused and not written.
    removed: BTreeSet<usize>,
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
    /// The capture host's signal handle, once it exists; the watch list
    /// reads the lifecycle states through it.
    signals: OnceLock<Arc<OnceLock<Signaller>>>,
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
            signals: OnceLock::new(),
            inner: Mutex::new(Inner {
                started: settings.watches.len(),
                removed: BTreeSet::new(),
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
                state: WatchState::SuspendedByUser,
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
            .position_of(id)
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
                .filter(|_| !inner.removed.contains(&index))
                .map(|watch| watch.id.clone())
                .ok_or_else(|| format!("no watch at position {index}"))?
        };
        self.set_watch(&id, paused)
    }
}

/// Why an edit of the watch list did not happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditError {
    /// The request was refused; the viewer shows the reason.
    Refused(Refusal),
    /// The request was fine but `settings.toml` could not be written.
    Failed(String),
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(refusal) => write!(f, "{refusal}"),
            Self::Failed(message) => write!(f, "{message}"),
        }
    }
}

impl PauseCtl {
    /// Adds a recursive watch on `path`: validated, written to
    /// `settings.toml` and listed. It starts capturing with the next engine
    /// start.
    ///
    /// # Errors
    ///
    /// [`EditError::Refused`] with the reason (data or export directory,
    /// duplicate, a whole drive that is not `confirmed`, a bad path), or
    /// [`EditError::Failed`] when the file cannot be written; then nothing
    /// changes.
    pub fn add_watch(
        &self,
        path: &str,
        confirmed: bool,
        context: &Context,
    ) -> Result<Added, EditError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| EditError::Failed(String::from("watch control is unavailable")))?;
        inner.writable()?;
        let mut view = inner.live_settings();
        let added =
            edit::add_watch(&mut view, path, confirmed, context).map_err(EditError::Refused)?;
        inner.write(&view).map_err(EditError::Failed)?;
        inner.settings.watches.push(added.watch.clone());
        Ok(added)
    }

    /// Removes the watch `id`. Rows already recorded stay in the log; an
    /// armed watch stops recording at once (a state row says so) and is gone
    /// from the file.
    ///
    /// # Errors
    ///
    /// As [`PauseCtl::add_watch`].
    pub fn remove_watch(&self, id: &str) -> Result<Removed, EditError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| EditError::Failed(String::from("watch control is unavailable")))?;
        inner.writable()?;
        let index = inner
            .position_of(id)
            .ok_or_else(|| EditError::Refused(Refusal::Unknown(id.to_owned())))?;
        let mut view = inner.live_settings();
        let removed = edit::remove_watch(&mut view, id).map_err(EditError::Refused)?;
        inner.write(&view).map_err(EditError::Failed)?;
        if index < inner.started {
            let transition = self.pause.set_watch(root_of(index), true);
            inner.emit(transition.as_slice());
            inner.removed.insert(index);
        } else {
            inner.settings.watches.remove(index);
        }
        Ok(removed)
    }

    /// Takes new settings from the viewer's settings screen or its window
    /// position: everything except the watches, which change only through
    /// [`PauseCtl::add_watch`], [`PauseCtl::remove_watch`] and the pause
    /// commands (they have positions the running capture depends on). The
    /// file is written from this control's own state, so a later pause never
    /// writes the old values back.
    ///
    /// # Errors
    ///
    /// The reason when the file is from a newer version or cannot be written;
    /// nothing changes then.
    pub fn apply_settings(&self, new: &Settings) -> Result<(), String> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| String::from("settings control is unavailable"))?;
        if !inner.writable {
            return Err(String::from(
                "settings.toml is from a newer version and is not changed by this build",
            ));
        }
        let mut merged = inner.live_settings();
        merged.version = new.version;
        merged.general = new.general.clone();
        merged.store = new.store.clone();
        merged.view = new.view.clone();
        merged.viewer = new.viewer.clone();
        merged.filters = new.filters.clone();
        inner.write(&merged)?;
        inner.settings.version = merged.version;
        inner.settings.general = merged.general;
        inner.settings.store = merged.store;
        inner.settings.view = merged.view;
        inner.settings.viewer = merged.viewer;
        inner.settings.filters = merged.filters;
        Ok(())
    }

    /// Tells the control where the lifecycle states of the armed roots can
    /// be read once the capture host runs.
    pub fn bind_signals(&self, signals: &Arc<OnceLock<Signaller>>) {
        let _ = self.signals.set(Arc::clone(signals));
    }

    /// The watches for the viewer's list, with the states of the running
    /// capture host.
    #[must_use]
    pub fn list(&self) -> Vec<WatchInfo> {
        let states: HashMap<u16, WatchState> = self
            .signals
            .get()
            .and_then(|bind| bind.get())
            .into_iter()
            .flat_map(Signaller::states)
            .map(|(root, state)| (root.0, state.watch_state()))
            .collect();
        self.list_with(|root| states.get(&root.0).copied())
    }

    /// [`PauseCtl::list`] with the lifecycle state of an armed root given by
    /// `state_of`. A pause overrides it, because pausing keeps the watch
    /// armed (ADR 0016).
    #[must_use]
    pub fn list_with(&self, state_of: impl Fn(RootIdx) -> Option<WatchState>) -> Vec<WatchInfo> {
        let Ok(inner) = self.inner.lock() else {
            return Vec::new();
        };
        inner
            .settings
            .watches
            .iter()
            .enumerate()
            .filter(|(index, _)| !inner.removed.contains(index))
            .map(|(index, watch)| {
                let root = root_of(index);
                let armed = index < inner.started;
                let state = if !armed {
                    WatchState::Arming
                } else if self.pause.is_paused(root) {
                    WatchState::SuspendedByUser
                } else {
                    state_of(root).unwrap_or(WatchState::Arming)
                };
                WatchInfo {
                    id: watch.id.clone(),
                    path: watch.path.clone(),
                    recursive: watch.recursive,
                    state,
                    armed,
                    missed_while_paused: if armed { self.pause.missed(root) } else { 0 },
                }
            })
            .collect()
    }
}

impl Inner {
    /// The position of the live watch `id`.
    fn position_of(&self, id: &str) -> Option<usize> {
        self.settings
            .watches
            .iter()
            .enumerate()
            .position(|(index, watch)| watch.id == id && !self.removed.contains(&index))
    }

    /// The settings without the watches the user removed.
    fn live_settings(&self) -> Settings {
        let mut live = self.settings.clone();
        live.watches = self
            .settings
            .watches
            .iter()
            .enumerate()
            .filter(|(index, _)| !self.removed.contains(index))
            .map(|(_, watch)| watch.clone())
            .collect();
        live
    }

    fn writable(&self) -> Result<(), EditError> {
        if self.writable {
            Ok(())
        } else {
            Err(EditError::Failed(String::from(
                "settings.toml is from a newer version and is not changed by this build",
            )))
        }
    }

    fn write(&self, settings: &Settings) -> Result<(), String> {
        let text = to_toml(settings)?;
        peekbuddy_win::atomic::write_atomic(&self.settings_path, text.as_bytes())
            .map_err(|error| format!("could not write settings.toml: {error}"))
    }

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
        self.write(&self.live_settings())
    }
}

#[cfg(test)]
mod tests;
