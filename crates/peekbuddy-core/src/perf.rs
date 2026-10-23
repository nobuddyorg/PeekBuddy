//! Allocation-free performance milestones and counters (#21).
//!
//! The engine and the viewer record milestones here; `peekbuddy-win` mirrors
//! each one to ETW and `--perf-probe` (#24) reads the table, so CI and WPR
//! see the same numbers. Everything is a fixed array of atomics: recording
//! never allocates, never locks and never blocks.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// Which process role recorded a milestone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The tray-resident engine (default role).
    Engine,
    /// The on-demand viewer (`--viewer`).
    Viewer,
}

impl Role {
    /// Stable name used in ETW events and probe output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Engine => "engine",
            Self::Viewer => "viewer",
        }
    }
}

/// A named point in a process's life (F7). See PERFORMANCE.md.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Milestone {
    /// `main` entered.
    ProcessStart,
    /// Settings read and validated.
    SettingsLoaded,
    /// Store opened and writable.
    StoreOpened,
    /// One watch root armed (recorded once per root).
    WatcherArmed,
    /// Tray icon added.
    TrayShown,
    /// Tray added and all watchers armed (F7 engine ready).
    EngineReady,
    /// Engine started the viewer process.
    ViewerSpawn,
    /// First presented frame with the first page of rows (F7 viewer ready).
    ViewerFirstFrame,
    /// A change-notification buffer overflowed.
    Overflow,
    /// A gap row was written.
    Gap,
    /// A reconciliation scan started.
    ReconcileStart,
    /// A reconciliation scan ended.
    ReconcileEnd,
    /// A new day segment was opened.
    SegmentRotate,
}

impl Milestone {
    /// Every milestone, in declaration order.
    pub const ALL: [Self; 13] = [
        Self::ProcessStart,
        Self::SettingsLoaded,
        Self::StoreOpened,
        Self::WatcherArmed,
        Self::TrayShown,
        Self::EngineReady,
        Self::ViewerSpawn,
        Self::ViewerFirstFrame,
        Self::Overflow,
        Self::Gap,
        Self::ReconcileStart,
        Self::ReconcileEnd,
        Self::SegmentRotate,
    ];

    /// Stable event name used in ETW and probe output.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::ProcessStart => "ProcessStart",
            Self::SettingsLoaded => "SettingsLoaded",
            Self::StoreOpened => "StoreOpened",
            Self::WatcherArmed => "WatcherArmed",
            Self::TrayShown => "TrayShown",
            Self::EngineReady => "EngineReady",
            Self::ViewerSpawn => "ViewerSpawn",
            Self::ViewerFirstFrame => "ViewerFirstFrame",
            Self::Overflow => "Overflow",
            Self::Gap => "Gap",
            Self::ReconcileStart => "ReconcileStart",
            Self::ReconcileEnd => "ReconcileEnd",
            Self::SegmentRotate => "SegmentRotate",
        }
    }

    const fn index(self) -> usize {
        self as usize
    }
}

/// Sentinel for "not recorded yet".
const UNSET: u64 = u64::MAX;

/// First and latest time and the count of each milestone, in microseconds
/// since process creation.
#[derive(Debug)]
pub struct MilestoneTable {
    first_us: [AtomicU64; Milestone::ALL.len()],
    last_us: [AtomicU64; Milestone::ALL.len()],
    count: [AtomicU32; Milestone::ALL.len()],
}

impl Default for MilestoneTable {
    fn default() -> Self {
        Self::new()
    }
}

impl MilestoneTable {
    /// An empty table; usable as a `static`.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            first_us: [const { AtomicU64::new(UNSET) }; Milestone::ALL.len()],
            last_us: [const { AtomicU64::new(UNSET) }; Milestone::ALL.len()],
            count: [const { AtomicU32::new(0) }; Milestone::ALL.len()],
        }
    }

    /// Records `milestone` at `since_process_create_us`.
    pub fn record(&self, milestone: Milestone, since_process_create_us: u64) {
        let i = milestone.index();
        if let (Some(first), Some(last), Some(count)) =
            (self.first_us.get(i), self.last_us.get(i), self.count.get(i))
        {
            // Keep the earliest value even if two threads race.
            first.fetch_min(since_process_create_us, Ordering::Relaxed);
            last.store(since_process_create_us, Ordering::Relaxed);
            count.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// The first time `milestone` was recorded.
    #[must_use]
    pub fn first_us(&self, milestone: Milestone) -> Option<u64> {
        self.first_us
            .get(milestone.index())
            .map(|v| v.load(Ordering::Relaxed))
            .filter(|&v| v != UNSET)
    }

    /// The latest time `milestone` was recorded.
    #[must_use]
    pub fn last_us(&self, milestone: Milestone) -> Option<u64> {
        self.last_us
            .get(milestone.index())
            .map(|v| v.load(Ordering::Relaxed))
            .filter(|&v| v != UNSET)
    }

    /// How often `milestone` was recorded.
    #[must_use]
    pub fn count(&self, milestone: Milestone) -> u32 {
        self.count
            .get(milestone.index())
            .map_or(0, |v| v.load(Ordering::Relaxed))
    }
}

/// Monotonic counters, emitted to ETW piggyback on batch commit (never from
/// a timer, F6).
#[derive(Debug, Default)]
pub struct Counters {
    /// Raw change records received from all backends.
    pub events_received: AtomicU64,
    /// Rows durably committed.
    pub rows_persisted: AtomicU64,
    /// Buffer overflows.
    pub overflows: AtomicU64,
    /// Gap rows written.
    pub gaps: AtomicU64,
    /// Highest ring depth seen.
    pub ring_depth_high_water: AtomicU64,
    /// Size of the last committed batch.
    pub last_batch_size: AtomicU64,
}

/// A plain copy of [`Counters`] at one instant.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CounterSnapshot {
    /// See [`Counters::events_received`].
    pub events_received: u64,
    /// See [`Counters::rows_persisted`].
    pub rows_persisted: u64,
    /// See [`Counters::overflows`].
    pub overflows: u64,
    /// See [`Counters::gaps`].
    pub gaps: u64,
    /// See [`Counters::ring_depth_high_water`].
    pub ring_depth_high_water: u64,
    /// See [`Counters::last_batch_size`].
    pub last_batch_size: u64,
}

impl Counters {
    /// All zero; usable as a `static`.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            events_received: AtomicU64::new(0),
            rows_persisted: AtomicU64::new(0),
            overflows: AtomicU64::new(0),
            gaps: AtomicU64::new(0),
            ring_depth_high_water: AtomicU64::new(0),
            last_batch_size: AtomicU64::new(0),
        }
    }

    /// Records the ring depth, keeping the maximum.
    pub fn observe_ring_depth(&self, depth: u64) {
        self.ring_depth_high_water
            .fetch_max(depth, Ordering::Relaxed);
    }

    /// Reads every counter.
    #[must_use]
    pub fn snapshot(&self) -> CounterSnapshot {
        CounterSnapshot {
            events_received: self.events_received.load(Ordering::Relaxed),
            rows_persisted: self.rows_persisted.load(Ordering::Relaxed),
            overflows: self.overflows.load(Ordering::Relaxed),
            gaps: self.gaps.load(Ordering::Relaxed),
            ring_depth_high_water: self.ring_depth_high_water.load(Ordering::Relaxed),
            last_batch_size: self.last_batch_size.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Counters, Milestone, MilestoneTable, Role};
    use std::sync::atomic::Ordering;

    #[test]
    fn unrecorded_milestones_are_absent() {
        let table = MilestoneTable::new();
        for milestone in Milestone::ALL {
            assert_eq!(table.first_us(milestone), None);
            assert_eq!(table.last_us(milestone), None);
            assert_eq!(table.count(milestone), 0);
        }
    }

    #[test]
    fn first_last_and_count_are_tracked() {
        let table = MilestoneTable::new();
        table.record(Milestone::WatcherArmed, 30);
        table.record(Milestone::WatcherArmed, 50);
        table.record(Milestone::WatcherArmed, 40);
        assert_eq!(table.first_us(Milestone::WatcherArmed), Some(30));
        assert_eq!(table.last_us(Milestone::WatcherArmed), Some(40));
        assert_eq!(table.count(Milestone::WatcherArmed), 3);
        assert_eq!(table.count(Milestone::TrayShown), 0);
    }

    #[test]
    fn first_keeps_the_minimum_even_out_of_order() {
        let table = MilestoneTable::new();
        table.record(Milestone::Gap, 100);
        table.record(Milestone::Gap, 7);
        assert_eq!(table.first_us(Milestone::Gap), Some(7));
    }

    #[test]
    fn names_are_unique_and_all_is_in_index_order() {
        for (i, milestone) in Milestone::ALL.iter().enumerate() {
            assert_eq!(milestone.index(), i);
        }
        let mut names: Vec<_> = Milestone::ALL.iter().map(|m| m.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), Milestone::ALL.len());
    }

    #[test]
    fn role_names() {
        assert_eq!(Role::Engine.as_str(), "engine");
        assert_eq!(Role::Viewer.as_str(), "viewer");
    }

    #[test]
    fn counters_snapshot_and_high_water() {
        let counters = Counters::new();
        counters.events_received.fetch_add(5, Ordering::Relaxed);
        counters.rows_persisted.fetch_add(4, Ordering::Relaxed);
        counters.overflows.fetch_add(1, Ordering::Relaxed);
        counters.gaps.fetch_add(1, Ordering::Relaxed);
        counters.last_batch_size.store(4, Ordering::Relaxed);
        counters.observe_ring_depth(10);
        counters.observe_ring_depth(3);
        let snapshot = counters.snapshot();
        assert_eq!(snapshot.events_received, 5);
        assert_eq!(snapshot.rows_persisted, 4);
        assert_eq!(snapshot.overflows, 1);
        assert_eq!(snapshot.gaps, 1);
        assert_eq!(snapshot.ring_depth_high_water, 10);
        assert_eq!(snapshot.last_batch_size, 4);
    }
}
