//! The compact view (#66): groups folded inside the rows the grid holds.
//!
//! Pure data, no egui. The grid keeps one row per stored event (raw rows are
//! the truth, F3); this module only decides which of the held rows are
//! *folded away* behind another. It runs the derived-view state machine of
//! `peekbuddy-core` (#41) over the island, oldest first, and takes the
//! newest member of each group as the group's **head**: that row stays, with
//! a badge, and the older members are folded away until the head is
//! expanded.
//!
//! The scope is the island only. The log is far too big to derive as a
//! whole, so a group that reaches beyond the island is folded as far as the
//! island sees it, and the scrollbar still counts raw rows (an estimate in
//! compact mode). Nothing is hidden for good: with every group expanded the
//! grid shows exactly the raw rows.

use std::collections::{BTreeSet, HashMap};

use peekbuddy_core::derive::{Config, DisplayItem, GroupKind, ViewMode, derive_all};
use peekbuddy_core::event::{Event, Flags, NameKind, NameRef, RootIdx, StoredEvent, VolIdx};
use peekbuddy_core::name::WideStr;

use crate::rows::GridRow;

/// What a held row is in the compact view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fold {
    /// An ordinary row.
    Plain,
    /// The newest row of a group: it stays and shows a badge.
    Head {
        /// What the group is.
        kind: GroupKind,
        /// The raw rows in the group, the head included.
        members: u32,
        /// The other members are shown.
        expanded: bool,
    },
    /// An older member of a collapsed group: folded away, zero height.
    Folded,
    /// An older member of an expanded group: shown like any row.
    Inside,
}

impl Fold {
    /// The badge text of a head: `3 events, saved`.
    #[must_use]
    pub fn badge(members: u32, kind: GroupKind) -> String {
        let what = match kind {
            GroupKind::Renamed => "renamed",
            GroupKind::Saved => "saved",
            GroupKind::Coalesced => "changed",
        };
        format!("{members} events, {what}")
    }
}

/// The folds of the held rows.
#[derive(Debug, Default)]
pub struct Folds {
    /// Virtual index of the first held row when the folds were made.
    top: u64,
    /// Every row that is not plain, by event id.
    marks: HashMap<i64, Fold>,
    /// `hidden[k]` is how many of the first `k` held rows are folded away.
    hidden: Vec<u32>,
}

/// The `RootIdx` for the roots of the held rows, in order of appearance.
fn root_index(roots: &mut Vec<Vec<u16>>, root: &[u16]) -> RootIdx {
    let at = roots
        .iter()
        .position(|known| known.as_slice() == root)
        .unwrap_or_else(|| {
            roots.push(root.to_vec());
            roots.len() - 1
        });
    RootIdx(u16::try_from(at).unwrap_or(u16::MAX - 1))
}

fn stored(row: &GridRow, roots: &mut Vec<Vec<u16>>) -> StoredEvent {
    let raw = &row.raw;
    StoredEvent {
        id: u64::try_from(raw.id).unwrap_or(0),
        ev: Event {
            ts: raw.ts,
            qpc: 0,
            src_ts: 0,
            source: raw.source,
            kind: raw.kind,
            raw_mask: raw.raw_mask,
            source_info: 0,
            is_dir: raw.is_dir,
            size: raw.size.unwrap_or(0),
            attrs: raw.attrs.unwrap_or(0),
            reparse_tag: raw.reparse_tag,
            file_id: raw.file_id,
            parent_file_id: 0,
            usn: 0,
            vol: VolIdx(0),
            root: if raw.root.is_empty() {
                RootIdx::NONE
            } else {
                root_index(roots, &raw.root)
            },
            name: NameRef::EMPTY,
            name_kind: NameKind::Unknown,
            flags: Flags::from_bits(raw.flags).unwrap_or(Flags::NONE),
        },
    }
}

impl Folds {
    /// No folds: every row is plain (the raw view).
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// Derives the folds of `rows`, given newest first, the first of them at
    /// virtual index `top`. A group whose head is in `expanded` shows its
    /// members.
    #[must_use]
    pub fn build<'a>(
        rows: impl DoubleEndedIterator<Item = &'a GridRow> + Clone,
        top: u64,
        expanded: &BTreeSet<i64>,
    ) -> Self {
        let mut roots = Vec::new();
        let events: Vec<(StoredEvent, &[u16])> = rows
            .clone()
            .rev()
            .map(|row| (stored(row, &mut roots), row.raw.path.as_slice()))
            .collect();
        let config = Config {
            mode: ViewMode::Compact,
            ..Config::default()
        };
        let (items, _) = derive_all(
            config,
            events
                .iter()
                .map(|(event, path)| (*event, WideStr::new(path))),
        );
        let mut marks = HashMap::new();
        for item in items {
            let DisplayItem::Group(group) = item else {
                continue;
            };
            let ids: Vec<i64> = group
                .member_ids
                .iter()
                .filter_map(|id| i64::try_from(*id).ok())
                .collect();
            let Some((&head, older)) = ids.split_last() else {
                continue;
            };
            let open = expanded.contains(&head);
            marks.insert(
                head,
                Fold::Head {
                    kind: group.kind,
                    members: u32::try_from(ids.len()).unwrap_or(u32::MAX),
                    expanded: open,
                },
            );
            for id in older {
                marks.insert(*id, if open { Fold::Inside } else { Fold::Folded });
            }
        }
        let mut hidden = Vec::with_capacity(events.len() + 1);
        let mut count = 0_u32;
        hidden.push(0);
        for row in rows {
            if marks.get(&row.id) == Some(&Fold::Folded) {
                count += 1;
            }
            hidden.push(count);
        }
        Self { top, marks, hidden }
    }

    /// What the row with `id` is.
    #[must_use]
    pub fn of(&self, id: i64) -> Fold {
        self.marks.get(&id).copied().unwrap_or(Fold::Plain)
    }

    /// Whether `id` heads a group.
    #[must_use]
    pub fn is_head(&self, id: i64) -> bool {
        matches!(self.of(id), Fold::Head { .. })
    }

    /// The ids of every group head.
    pub fn heads(&self) -> impl Iterator<Item = i64> + '_ {
        self.marks
            .iter()
            .filter(|(_, fold)| matches!(fold, Fold::Head { .. }))
            .map(|(id, _)| *id)
    }

    /// How many rows before virtual index `row` are folded away. Rows outside
    /// the held island count as the island's edge: nothing before it, all of
    /// them after it.
    #[must_use]
    pub fn hidden_before(&self, row: u64) -> u64 {
        let Some(at) = row.checked_sub(self.top) else {
            return 0;
        };
        let at = usize::try_from(at).unwrap_or(usize::MAX);
        let count = self.hidden.get(at).or_else(|| self.hidden.last());
        u64::from(count.copied().unwrap_or(0))
    }

    /// How many rows before virtual index `row` take up room: the rows that
    /// are not folded away.
    #[must_use]
    pub fn shown_before(&self, row: u64) -> u64 {
        row.saturating_sub(self.hidden_before(row))
    }

    /// Whether the held row at virtual index `row` is folded away.
    #[must_use]
    pub fn is_folded(&self, row: u64) -> bool {
        self.hidden_before(row.saturating_add(1)) > self.hidden_before(row)
    }

    /// Rows folded away in all.
    #[must_use]
    pub fn folded(&self) -> u64 {
        self.hidden.last().copied().map_or(0, u64::from)
    }
}

#[cfg(test)]
mod tests;
