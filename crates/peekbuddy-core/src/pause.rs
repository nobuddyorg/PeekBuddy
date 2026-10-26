//! Pause and resume (#63): handles stay armed, events are counted and
//! discarded after the parse (option A of the lifecycle decision, see
//! `docs/adr/0016-pause-keeps-watches-armed.md`).
//!
//! [`Pause`] is the shared switchboard: the capture thread asks
//! [`Pause::is_paused`] and counts with [`Pause::note_missed`] (one atomic
//! increment, no allocation); the control path flips the switches and gets
//! back the [`Transition`]s that must become rows.
//!
//! # Rows
//!
//! Every effective change of a watch is one row (F19): pausing stores
//! `WatchStateChanged(SuspendedByUser)`, resuming stores
//! `WatchStateChanged(Live)` carrying the missed count in [`Event::size`].
//! The missed events themselves are never stored (F3 holds for what was
//! recorded). An overflow while paused turns the count into a lower bound:
//! the resume row has [`Flags::GAP_FOLLOWS`] set and the UI shows ">= N".
//!
//! A watch is effectively paused when the global switch or its own switch is
//! on, so resuming everything leaves individually paused watches paused and
//! gives them no row.
//!
//! # Races
//!
//! The capture thread may read "paused" just before a resume and count after
//! the control path took the number. That event is counted once and shows up
//! in the next row of the watch; it is never lost and never counted twice.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::capture::EventSink;
use crate::event::{Event, Flags, Kind, NameKind, NameRef, RootIdx, Source, VolIdx, WatchState};

/// Watches with their own switch. A root index at or past it cannot be
/// paused individually (the settings allow far fewer watches).
pub const MAX_WATCHES: usize = 256;

/// One effective change of a watch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transition {
    /// The watch.
    pub root: RootIdx,
    /// `SuspendedByUser` or `Live`.
    pub state: WatchState,
    /// Events discarded while the watch was paused; 0 for a pause.
    pub missed: u64,
    /// An overflow happened while paused: `missed` is a lower bound.
    pub lower_bound: bool,
}

impl Transition {
    /// The row for this change.
    #[must_use]
    pub const fn to_event(&self, vol: VolIdx, ts: u64, qpc: u64) -> Event {
        Event {
            ts,
            qpc,
            src_ts: 0,
            source: Source::System,
            kind: Kind::WatchStateChanged(self.state),
            raw_mask: 0,
            source_info: 0,
            is_dir: false,
            size: self.missed,
            attrs: 0,
            reparse_tag: 0,
            file_id: 0,
            parent_file_id: 0,
            usn: 0,
            vol,
            root: self.root,
            name: NameRef::EMPTY,
            name_kind: NameKind::Unknown,
            flags: if self.lower_bound {
                Flags::GAP_FOLLOWS
            } else {
                Flags::NONE
            },
        }
    }
}

/// The pause switches and the missed counters.
#[derive(Debug)]
pub struct Pause {
    global: AtomicBool,
    watch: Box<[AtomicBool]>,
    missed: Box<[AtomicU64]>,
    lower_bound: Box<[AtomicBool]>,
}

impl Default for Pause {
    fn default() -> Self {
        Self::new()
    }
}

impl Pause {
    /// Nothing paused.
    #[must_use]
    pub fn new() -> Self {
        Self {
            global: AtomicBool::new(false),
            watch: (0..MAX_WATCHES).map(|_| AtomicBool::new(false)).collect(),
            missed: (0..MAX_WATCHES).map(|_| AtomicU64::new(0)).collect(),
            lower_bound: (0..MAX_WATCHES).map(|_| AtomicBool::new(false)).collect(),
        }
    }

    /// Whether events of `root` are discarded right now.
    #[must_use]
    pub fn is_paused(&self, root: RootIdx) -> bool {
        self.global.load(Ordering::Acquire)
            || self
                .watch
                .get(usize::from(root.0))
                .is_some_and(|flag| flag.load(Ordering::Acquire))
    }

    /// Whether the global switch is on.
    #[must_use]
    pub fn global(&self) -> bool {
        self.global.load(Ordering::Acquire)
    }

    /// Whether `root` has its own switch on.
    #[must_use]
    pub fn watch_paused(&self, root: RootIdx) -> bool {
        self.watch
            .get(usize::from(root.0))
            .is_some_and(|flag| flag.load(Ordering::Acquire))
    }

    /// Counts one discarded event of `root`.
    pub fn note_missed(&self, root: RootIdx) {
        if let Some(count) = self.missed.get(usize::from(root.0)) {
            count.fetch_add(1, Ordering::AcqRel);
        }
    }

    /// Notes a gap while `root` is paused: the missed count is a lower
    /// bound from now on.
    pub fn note_gap(&self, root: RootIdx) {
        if let Some(flag) = self.lower_bound.get(usize::from(root.0)) {
            flag.store(true, Ordering::Release);
        }
    }

    /// Events discarded for `root` since it was last resumed.
    #[must_use]
    pub fn missed(&self, root: RootIdx) -> u64 {
        self.missed
            .get(usize::from(root.0))
            .map_or(0, |count| count.load(Ordering::Acquire))
    }

    /// Switches everything off or on. `roots` are the watches that exist;
    /// the result has one transition per watch whose effective state
    /// changed.
    pub fn set_global(&self, paused: bool, roots: &[RootIdx]) -> Vec<Transition> {
        let before = self.global.swap(paused, Ordering::AcqRel);
        if before == paused {
            return Vec::new();
        }
        roots
            .iter()
            .filter(|&&root| !self.watch_paused(root))
            .map(|&root| self.transition(root, paused))
            .collect()
    }

    /// Switches one watch off or on; `None` when its effective state did not
    /// change (already so, or the global switch decides).
    pub fn set_watch(&self, root: RootIdx, paused: bool) -> Option<Transition> {
        let flag = self.watch.get(usize::from(root.0))?;
        let before = flag.swap(paused, Ordering::AcqRel);
        if before == paused || self.global.load(Ordering::Acquire) {
            return None;
        }
        Some(self.transition(root, paused))
    }

    /// Forgets the switch and the counter of a watch that was removed.
    pub fn forget(&self, root: RootIdx) {
        let at = usize::from(root.0);
        if let Some(flag) = self.watch.get(at) {
            flag.store(false, Ordering::Release);
        }
        if let Some(count) = self.missed.get(at) {
            count.store(0, Ordering::Release);
        }
        if let Some(flag) = self.lower_bound.get(at) {
            flag.store(false, Ordering::Release);
        }
    }

    fn transition(&self, root: RootIdx, paused: bool) -> Transition {
        if paused {
            return Transition {
                root,
                state: WatchState::SuspendedByUser,
                missed: 0,
                lower_bound: false,
            };
        }
        let at = usize::from(root.0);
        Transition {
            root,
            state: WatchState::Live,
            missed: self
                .missed
                .get(at)
                .map_or(0, |count| count.swap(0, Ordering::AcqRel)),
            lower_bound: self
                .lower_bound
                .get(at)
                .is_some_and(|flag| flag.swap(false, Ordering::AcqRel)),
        }
    }
}

/// An [`EventSink`] that discards the events of paused watches.
///
/// A discarded event is counted ([`Pause::note_missed`]); a discarded gap row
/// makes the count a lower bound ([`Pause::note_gap`]). The sink says it took
/// the event, so the capture side owes nothing.
#[derive(Debug)]
pub struct PausableSink<S> {
    inner: S,
    pause: Arc<Pause>,
}

impl<S> PausableSink<S> {
    /// Wraps `inner`.
    #[must_use]
    pub const fn new(inner: S, pause: Arc<Pause>) -> Self {
        Self { inner, pause }
    }

    /// The wrapped sink.
    #[must_use]
    pub fn into_inner(self) -> S {
        self.inner
    }
}

impl<S: EventSink> EventSink for PausableSink<S> {
    fn push(&mut self, event: &Event, name: &[u16]) -> bool {
        if self.pause.is_paused(event.root) {
            match event.kind {
                Kind::Overflow(_) => self.pause.note_gap(event.root),
                _ => self.pause.note_missed(event.root),
            }
            return true;
        }
        self.inner.push(event, name)
    }

    fn batch_end(&mut self) {
        self.inner.batch_end();
    }
}

#[cfg(test)]
mod tests;
