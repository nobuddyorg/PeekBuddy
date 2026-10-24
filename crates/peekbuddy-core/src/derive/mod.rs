//! Derived views (#41): display groups over the raw event stream.
//!
//! One logical save is two to four raw events and a rename is two. Raw rows
//! always stay (F3); this module only decides how a viewer *shows* them, and
//! never hides a row: every [`Group`] lists all its members and their count
//! (`raw_count`, the "raw events: n" of the UI). Nothing here is persisted and
//! the engine never calls it.
//!
//! [`Deriver`] is a streaming state machine over [`StoredEvent`]s in id
//! order. It uses the event `ts` only: no wall clock, no timer (F6).
//!
//! - **Bursts.** Events about one path (case rules of #39) that follow each
//!   other within a window share an open burst. A rename pair links the old
//!   and the new path to one burst. Bursts are keyed per root.
//! - **Rename stitching.** A `RenamedFrom` waits for its `RenamedTo`: the very
//!   next event when it names the same (or an unknown) file id, or a later one
//!   with the same file id (USN pairs by FRN). A `RenamedTo` without a waiting
//!   `RenamedFrom`, and a `RenamedFrom` whose burst closes first, become
//!   [`DisplayItem::Unpaired`] rows.
//! - **Atomic saves.** When a burst closes, three patterns make it a
//!   [`GroupKind::Saved`] group: temp file written then renamed, target
//!   renamed to a backup then rewritten, delete then create of one name.
//! - **Coalescing.** In [`ViewMode::Compact`] a burst of two or more events
//!   without renames is a [`GroupKind::Coalesced`] group.
//! - **Barriers.** `Overflow`, `WatchStateChanged` and `Reconciled*` rows are
//!   never absorbed; they close every open burst of their root. Rows without
//!   a name and system rows are passed through as they are.
//! - **Order.** Items come out sorted by their first member's id; a group
//!   takes the place of its first member. A burst holds back the rows behind
//!   it until it closes (at most one window of event time).
//! - **Bounds.** At most `max_open_groups` bursts are open; when a new one
//!   would exceed that, the oldest closes as it is and `forced_flushes` counts
//!   it. Pending items are bounded the same way (four times that).

mod classify;

use std::collections::{BTreeMap, BTreeSet};

use classify::{Outcome, Step, classify, rename_pairs};

use crate::event::{Kind, RootIdx, StoredEvent};
use crate::name::{CaseMode, WideStr, fold_upper};

/// The smallest window, in milliseconds.
pub const MIN_WINDOW_MS: u32 = 50;
/// The largest window, in milliseconds.
pub const MAX_WINDOW_MS: u32 = 500;

/// FILETIME ticks (100 ns) per millisecond.
const TICKS_PER_MS: u64 = 10_000;

/// How much the viewer folds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    /// Rename pairs and atomic saves only.
    Detail,
    /// Also one group per busy path.
    Compact,
}

/// What a group is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupKind {
    /// A matched `RenamedFrom` and `RenamedTo`.
    Renamed,
    /// An atomic save.
    Saved,
    /// Several events on one path (compact mode).
    Coalesced,
}

/// A display group. Every member is a raw row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    /// What the group is.
    pub kind: GroupKind,
    /// The watch root.
    pub root: RootIdx,
    /// Id of the first member (the group sits at its place).
    pub first_id: u64,
    /// Id of the last member.
    pub last_id: u64,
    /// Every member id, ascending.
    pub member_ids: Vec<u64>,
    /// The number of raw rows, shown as "raw events: n". Equals
    /// `member_ids.len()`.
    pub raw_count: usize,
    /// The path as delivered: the new path of a rename, the target of a save.
    pub path: Vec<u16>,
    /// For a rename, the old path.
    pub from_path: Option<Vec<u16>>,
}

/// One row of the derived view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisplayItem {
    /// A raw row, shown as it is.
    Raw(u64),
    /// A rename half whose other half never came.
    Unpaired(u64),
    /// A group of raw rows.
    Group(Group),
}

impl DisplayItem {
    /// The id the item sorts by.
    #[must_use]
    pub fn first_id(&self) -> u64 {
        match self {
            Self::Raw(id) | Self::Unpaired(id) => *id,
            Self::Group(group) => group.first_id,
        }
    }

    /// Every raw id the item stands for, ascending.
    #[must_use]
    pub fn ids(&self) -> Vec<u64> {
        match self {
            Self::Raw(id) | Self::Unpaired(id) => vec![*id],
            Self::Group(group) => group.member_ids.clone(),
        }
    }
}

/// Deriver settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Detail or compact.
    pub mode: ViewMode,
    /// The window for save patterns and rename waits, in milliseconds
    /// (clamped to 50..=500).
    pub save_window_ms: u32,
    /// The window for coalescing in compact mode, in milliseconds (clamped to
    /// 50..=500). A burst lives for the larger of the two in compact mode.
    pub coalesce_window_ms: u32,
    /// The most bursts open at once (at least 1).
    pub max_open_groups: usize,
    /// The case rule for roots without an override.
    pub default_case: CaseMode,
    /// Per-root case rules (#39).
    pub case_overrides: Vec<(RootIdx, CaseMode)>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            mode: ViewMode::Detail,
            save_window_ms: 250,
            coalesce_window_ms: 200,
            max_open_groups: 4_096,
            default_case: CaseMode::Insensitive,
            case_overrides: Vec::new(),
        }
    }
}

impl Config {
    /// The window of a burst, in ticks.
    fn window_ticks(&self) -> u64 {
        let ms = match self.mode {
            ViewMode::Detail => self.save_window_ms,
            ViewMode::Compact => self.save_window_ms.max(self.coalesce_window_ms),
        };
        u64::from(ms.clamp(MIN_WINDOW_MS, MAX_WINDOW_MS)) * TICKS_PER_MS
    }

    fn mode_of(&self, root: RootIdx) -> CaseMode {
        self.case_overrides
            .iter()
            .rev()
            .find(|(r, _)| *r == root)
            .map_or(self.default_case, |&(_, mode)| mode)
    }
}

/// Counters of a [`Deriver`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stats {
    /// Bursts closed early because too many were open or too many items
    /// were pending.
    pub forced_flushes: u64,
    /// Rename halves shown as [`DisplayItem::Unpaired`].
    pub unpaired: u64,
    /// Bursts open now.
    pub open_groups: usize,
    /// Items not yet taken out because a burst holds them back.
    pub pending_items: usize,
}

type PathKey = (u16, Vec<u16>);

#[derive(Debug)]
struct PathInfo {
    key: PathKey,
    display: Vec<u16>,
}

#[derive(Debug)]
struct Open {
    root: RootIdx,
    first_id: u64,
    last_ts: u64,
    steps: Vec<Step>,
    paths: Vec<PathInfo>,
}

#[derive(Debug)]
enum Entry {
    Open(usize),
    Closed(DisplayItem),
}

/// A `RenamedFrom` waiting for its other half.
#[derive(Debug, Clone, Copy)]
struct Awaiting {
    open: usize,
    frn: Option<(u16, u128)>,
}

#[derive(Debug, Clone, Copy)]
struct Prev {
    id: u64,
    kind: Kind,
    root: RootIdx,
    file_id: u128,
}

enum Role {
    Barrier,
    Pass,
    Path,
}

fn role(kind: Kind) -> Role {
    match kind {
        Kind::Overflow(_)
        | Kind::WatchStateChanged(_)
        | Kind::ReconciledCreated
        | Kind::ReconciledModified
        | Kind::ReconciledDeleted => Role::Barrier,
        Kind::EngineStarted
        | Kind::EngineStopped
        | Kind::SegmentRotated
        | Kind::RetentionPurged
        | Kind::SettingsChanged => Role::Pass,
        _ => Role::Path,
    }
}

/// The streaming deriver. Feed it events in id order with [`Deriver::push`],
/// collect items with [`Deriver::take`], and call [`Deriver::finish`] at the
/// end of the stream (or [`Deriver::advance`] when time passes without
/// events, for example on a viewer frame).
#[derive(Debug)]
pub struct Deriver {
    cfg: Config,
    window: u64,
    now: u64,
    entries: BTreeMap<u64, Entry>,
    slab: Vec<Option<Open>>,
    free: Vec<usize>,
    open_count: usize,
    by_key: BTreeMap<PathKey, usize>,
    deadlines: BTreeSet<(u64, usize)>,
    awaiting: BTreeMap<u64, Awaiting>,
    pending_from: BTreeMap<(u16, u128), u64>,
    prev: Option<Prev>,
    ready: Vec<DisplayItem>,
    stats: Stats,
}

impl Deriver {
    /// A deriver with `config`.
    #[must_use]
    pub fn new(mut config: Config) -> Self {
        config.max_open_groups = config.max_open_groups.max(1);
        Self {
            window: config.window_ticks(),
            cfg: config,
            now: 0,
            entries: BTreeMap::new(),
            slab: Vec::new(),
            free: Vec::new(),
            open_count: 0,
            by_key: BTreeMap::new(),
            deadlines: BTreeSet::new(),
            awaiting: BTreeMap::new(),
            pending_from: BTreeMap::new(),
            prev: None,
            ready: Vec::new(),
            stats: Stats::default(),
        }
    }

    /// The counters.
    #[must_use]
    pub fn stats(&self) -> Stats {
        Stats {
            open_groups: self.open_count,
            pending_items: self.entries.len(),
            ..self.stats
        }
    }

    /// Moves the items that are final into `out`.
    pub fn take(&mut self, out: &mut Vec<DisplayItem>) {
        out.append(&mut self.ready);
    }

    /// Adds the next event, with its name as delivered.
    pub fn push(&mut self, event: StoredEvent, name: WideStr<'_>) {
        let ev = event.ev;
        self.now = self.now.max(ev.ts);
        self.expire();
        match role(ev.kind) {
            Role::Barrier => {
                self.close_root(ev.root);
                self.put(event.id, DisplayItem::Raw(event.id));
            }
            Role::Pass => self.put(event.id, DisplayItem::Raw(event.id)),
            Role::Path if name.is_empty() => self.put(event.id, DisplayItem::Raw(event.id)),
            Role::Path => self.push_path(event, name),
        }
        self.prev = Some(Prev {
            id: event.id,
            kind: ev.kind,
            root: ev.root,
            file_id: ev.file_id,
        });
        self.pop_ready();
        self.bound();
    }

    /// Lets event time pass to `ts` (for example a viewer frame's time in
    /// the same clock as the events): bursts whose window ended close.
    pub fn advance(&mut self, ts: u64) {
        self.now = self.now.max(ts);
        self.expire();
        self.pop_ready();
    }

    /// Closes every open burst; every item is final afterwards.
    pub fn finish(&mut self) {
        for idx in 0..self.slab.len() {
            self.close(idx);
        }
        self.pop_ready();
    }

    fn key_of(&self, root: RootIdx, name: WideStr<'_>) -> PathKey {
        let fold = self.cfg.mode_of(root) == CaseMode::Insensitive;
        let units = name
            .units()
            .iter()
            .map(|&u| {
                let u = if u == u16::from(b'/') {
                    u16::from(b'\\')
                } else {
                    u
                };
                if fold { fold_upper(u) } else { u }
            })
            .collect();
        (root.0, units)
    }

    fn put(&mut self, id: u64, item: DisplayItem) {
        self.entries.insert(id, Entry::Closed(item));
    }

    fn push_path(&mut self, event: StoredEvent, name: WideStr<'_>) {
        let ev = event.ev;
        let key = self.key_of(ev.root, name);
        match ev.kind {
            Kind::RenamedTo => {
                if let Some((from_id, open)) = self.find_partner(&event) {
                    self.pair(open, from_id, event, key, name);
                } else {
                    self.stats.unpaired += 1;
                    self.put(event.id, DisplayItem::Unpaired(event.id));
                }
            }
            Kind::RenamedFrom => {
                let open = self.add_step(event, key, name, true);
                let frn = (ev.file_id != 0).then_some((ev.root.0, ev.file_id));
                if let Some(frn) = frn {
                    self.pending_from.insert(frn, event.id);
                }
                self.awaiting.insert(event.id, Awaiting { open, frn });
            }
            _ => {
                self.add_step(event, key, name, false);
            }
        }
    }

    /// The waiting `RenamedFrom` for this `RenamedTo`: the previous event
    /// when it fits, else one with the same file id.
    fn find_partner(&self, event: &StoredEvent) -> Option<(u64, usize)> {
        let ev = &event.ev;
        if let Some(prev) = self.prev
            && prev.kind == Kind::RenamedFrom
            && prev.root == ev.root
            && (prev.file_id == ev.file_id || prev.file_id == 0 || ev.file_id == 0)
            && let Some(waiting) = self.awaiting.get(&prev.id)
        {
            return Some((prev.id, waiting.open));
        }
        if ev.file_id != 0
            && let Some(&from_id) = self.pending_from.get(&(ev.root.0, ev.file_id))
            && let Some(waiting) = self.awaiting.get(&from_id)
        {
            return Some((from_id, waiting.open));
        }
        None
    }

    /// Joins a `RenamedTo` to the burst of its `RenamedFrom`.
    fn pair(
        &mut self,
        open: usize,
        from_id: u64,
        event: StoredEvent,
        key: PathKey,
        name: WideStr<'_>,
    ) {
        self.stop_waiting(from_id);
        // An earlier burst on the new path joins this one.
        if let Some(&other) = self.by_key.get(&key)
            && other != open
        {
            self.merge(other, open);
        }
        let ts = event.ev.ts;
        let Some(burst) = self.slab.get_mut(open).and_then(Option::as_mut) else {
            // The burst is gone (cannot happen while it is awaited): never
            // lose the row.
            self.stats.unpaired += 1;
            self.put(event.id, DisplayItem::Unpaired(event.id));
            return;
        };
        let path = if let Some(path) = burst.paths.iter().position(|p| p.key == key) {
            path
        } else {
            burst.paths.push(PathInfo {
                key: key.clone(),
                display: name.units().to_vec(),
            });
            burst.paths.len() - 1
        };
        if let Some(from) = burst.steps.iter_mut().find(|s| s.id == from_id) {
            from.awaiting = false;
            from.partner = Some(event.id);
        }
        burst.steps.push(Step {
            id: event.id,
            kind: Kind::RenamedTo,
            path,
            partner: Some(from_id),
            awaiting: false,
        });
        self.by_key.insert(key, open);
        self.touch(open, ts);
    }

    fn stop_waiting(&mut self, from_id: u64) {
        if let Some(waiting) = self.awaiting.remove(&from_id)
            && let Some(frn) = waiting.frn
            && self.pending_from.get(&frn) == Some(&from_id)
        {
            self.pending_from.remove(&frn);
        }
    }

    /// Adds an event to the burst of its path, opening one if needed.
    fn add_step(
        &mut self,
        event: StoredEvent,
        key: PathKey,
        name: WideStr<'_>,
        awaiting: bool,
    ) -> usize {
        let ev = event.ev;
        if let Some(&open) = self.by_key.get(&key)
            && let Some(burst) = self.slab.get_mut(open).and_then(Option::as_mut)
            && let Some(path) = burst.paths.iter().position(|p| p.key == key)
        {
            burst.steps.push(Step {
                id: event.id,
                kind: ev.kind,
                path,
                partner: None,
                awaiting,
            });
            self.touch(open, ev.ts);
            return open;
        }
        if self.open_count >= self.cfg.max_open_groups {
            self.pop_ready();
            self.force_flush_front();
        }
        let burst = Open {
            root: ev.root,
            first_id: event.id,
            last_ts: ev.ts,
            steps: vec![Step {
                id: event.id,
                kind: ev.kind,
                path: 0,
                partner: None,
                awaiting,
            }],
            paths: vec![PathInfo {
                key: key.clone(),
                display: name.units().to_vec(),
            }],
        };
        let open = if let Some(slot) = self.free.pop() {
            if let Some(cell) = self.slab.get_mut(slot) {
                *cell = Some(burst);
            }
            slot
        } else {
            self.slab.push(Some(burst));
            self.slab.len() - 1
        };
        self.open_count += 1;
        self.by_key.insert(key, open);
        self.deadlines.insert((ev.ts, open));
        self.entries.insert(event.id, Entry::Open(open));
        open
    }

    fn touch(&mut self, open: usize, ts: u64) {
        if let Some(burst) = self.slab.get_mut(open).and_then(Option::as_mut) {
            let last = burst.last_ts.max(ts);
            if last != burst.last_ts {
                self.deadlines.remove(&(burst.last_ts, open));
                self.deadlines.insert((last, open));
                burst.last_ts = last;
            }
        }
    }

    /// Moves burst `from` into burst `into`.
    fn merge(&mut self, from: usize, into: usize) {
        let Some(mut source) = self.slab.get_mut(from).and_then(Option::take) else {
            return;
        };
        let Some(mut target) = self.slab.get_mut(into).and_then(Option::take) else {
            if let Some(cell) = self.slab.get_mut(from) {
                *cell = Some(source);
            }
            return;
        };
        self.free.push(from);
        self.open_count -= 1;
        self.deadlines.remove(&(source.last_ts, from));
        self.deadlines.remove(&(target.last_ts, into));
        self.entries.remove(&source.first_id);
        let offset = target.paths.len();
        for path in &source.paths {
            self.by_key.insert(path.key.clone(), into);
        }
        for step in &mut source.steps {
            step.path += offset;
            if step.awaiting
                && let Some(waiting) = self.awaiting.get_mut(&step.id)
            {
                waiting.open = into;
            }
        }
        target.paths.append(&mut source.paths);
        target.steps.append(&mut source.steps);
        target.steps.sort_by_key(|s| s.id);
        target.last_ts = target.last_ts.max(source.last_ts);
        if source.first_id < target.first_id {
            self.entries.remove(&target.first_id);
            target.first_id = source.first_id;
            self.entries.insert(target.first_id, Entry::Open(into));
        }
        self.deadlines.insert((target.last_ts, into));
        if let Some(cell) = self.slab.get_mut(into) {
            *cell = Some(target);
        }
    }

    fn expire(&mut self) {
        while let Some(&(last, open)) = self.deadlines.first() {
            if last.saturating_add(self.window) < self.now {
                self.close(open);
            } else {
                break;
            }
        }
    }

    fn close_root(&mut self, root: RootIdx) {
        let doomed: Vec<usize> = self
            .slab
            .iter()
            .enumerate()
            .filter_map(|(i, b)| b.as_ref().filter(|b| b.root == root).map(|_| i))
            .collect();
        for open in doomed {
            self.close(open);
        }
    }

    /// Closes burst `open` and puts its items in place of its entry.
    fn close(&mut self, open: usize) {
        let Some(burst) = self.slab.get_mut(open).and_then(Option::take) else {
            return;
        };
        self.free.push(open);
        self.open_count -= 1;
        self.deadlines.remove(&(burst.last_ts, open));
        for path in &burst.paths {
            if self.by_key.get(&path.key) == Some(&open) {
                self.by_key.remove(&path.key);
            }
        }
        for step in burst.steps.iter().filter(|s| s.awaiting) {
            self.stop_waiting(step.id);
        }
        self.place(&burst);
    }

    fn group(
        burst: &Open,
        kind: GroupKind,
        ids: Vec<u64>,
        path: usize,
        from_path: Option<usize>,
    ) -> DisplayItem {
        let display = |i: usize| {
            burst
                .paths
                .get(i)
                .map(|p| p.display.clone())
                .unwrap_or_default()
        };
        DisplayItem::Group(Group {
            kind,
            root: burst.root,
            first_id: ids.first().copied().unwrap_or(burst.first_id),
            last_id: ids.last().copied().unwrap_or(burst.first_id),
            raw_count: ids.len(),
            member_ids: ids,
            path: display(path),
            from_path: from_path.map(display),
        })
    }

    fn place(&mut self, burst: &Open) {
        let compact = self.cfg.mode == ViewMode::Compact;
        let all: Vec<u64> = burst.steps.iter().map(|s| s.id).collect();
        match classify(&burst.steps, compact) {
            Outcome::Saved { target } => {
                let item = Self::group(burst, GroupKind::Saved, all, target, None);
                self.put(burst.first_id, item);
            }
            Outcome::Coalesced => {
                let path = burst.steps.last().map_or(0, |s| s.path);
                let item = Self::group(burst, GroupKind::Coalesced, all, path, None);
                self.put(burst.first_id, item);
            }
            Outcome::Split => {
                let pairs = rename_pairs(&burst.steps);
                let mut paired = vec![false; burst.steps.len()];
                for &(from, to) in &pairs {
                    for index in [from, to] {
                        if let Some(flag) = paired.get_mut(index) {
                            *flag = true;
                        }
                    }
                }
                for &(from, to) in &pairs {
                    let (Some(f), Some(t)) = (burst.steps.get(from), burst.steps.get(to)) else {
                        continue;
                    };
                    let item = Self::group(
                        burst,
                        GroupKind::Renamed,
                        vec![f.id, t.id],
                        t.path,
                        Some(f.path),
                    );
                    self.put(f.id, item);
                }
                for (step, &is_paired) in burst.steps.iter().zip(&paired) {
                    if is_paired {
                        continue;
                    }
                    if step.awaiting {
                        self.stats.unpaired += 1;
                        self.put(step.id, DisplayItem::Unpaired(step.id));
                    } else {
                        self.put(step.id, DisplayItem::Raw(step.id));
                    }
                }
            }
        }
    }

    /// Moves the final items at the front of the order out.
    fn pop_ready(&mut self) {
        while let Some(entry) = self.entries.first_entry() {
            if matches!(entry.get(), Entry::Closed(_)) {
                if let Entry::Closed(item) = entry.remove() {
                    self.ready.push(item);
                }
            } else {
                break;
            }
        }
    }

    /// Closes the burst at the front of the order (the oldest) as it is.
    fn force_flush_front(&mut self) {
        let front = self.entries.first_key_value().and_then(|(_, e)| match e {
            Entry::Open(open) => Some(*open),
            Entry::Closed(_) => None,
        });
        if let Some(open) = front {
            self.stats.forced_flushes += 1;
            self.close(open);
            self.pop_ready();
        }
    }

    fn bound(&mut self) {
        let max_pending = self.cfg.max_open_groups.saturating_mul(4);
        while self.entries.len() > max_pending {
            let before = self.entries.len();
            self.force_flush_front();
            if self.entries.len() == before {
                break;
            }
        }
    }
}

/// Derives a whole slice: events with their names, in id order.
#[must_use]
pub fn derive_all<'a>(
    config: Config,
    events: impl IntoIterator<Item = (StoredEvent, WideStr<'a>)>,
) -> (Vec<DisplayItem>, Stats) {
    let mut deriver = Deriver::new(config);
    for (event, name) in events {
        deriver.push(event, name);
    }
    deriver.finish();
    let mut items = Vec::new();
    deriver.take(&mut items);
    (items, deriver.stats())
}

#[cfg(test)]
mod tests;
