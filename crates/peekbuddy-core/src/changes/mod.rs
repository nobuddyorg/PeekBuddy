//! "What changed since" (#82): the net change of a range of raw rows.
//!
//! [`Fold`] is a streaming fold over [`Op`]s in id order. It follows each file
//! through creates, renames and deletes and reports, per file, what the range
//! did to it as a whole: created, deleted, modified, renamed (from -> to) or
//! replaced. A file created and deleted inside the range is *transient*: only
//! counted, never listed. Nothing here is persisted; the raw rows stay (F3)
//! and every row says how many raw events it folded.
//!
//! Files are keyed by an opaque, `Copy` path key, in practice the interned
//! node id of the store, so the case rules of #39 are already applied. A
//! rename gives the new name a new key; the fold keeps one file identity
//! across the chain, so a file renamed three times is one row.
//!
//! # Rename halves
//!
//! RDCW reports a move out of the watched root as a delete and a move in as a
//! create. A `RenamedFrom` is therefore held back until its `RenamedTo`
//! arrives (the same non-zero file id, or an unknown one on the very next
//! op). Any other op flushes the held halves as deletes, and an unmatched
//! `RenamedTo` is a create: the fold never claims a rename it did not see.
//!
//! # Gaps
//!
//! [`Op::Gap`] records a hole in the stream of a root; [`Op::Reconciled`]
//! says a reconcile of that root finished and covered the holes before it.
//! [`Summary::incomplete`] is true while any hole is uncovered: a diff is
//! never presented as complete across an unreconciled gap.
//!
//! # Memory
//!
//! One slot per distinct *surviving* path, not per raw event: transient files
//! release their slot at once.

use std::collections::HashMap;
use std::hash::Hash;

use crate::event::Kind;

/// One raw row, reduced to what the fold needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op<K> {
    /// A file or directory appeared.
    Created(K),
    /// Data or metadata changed.
    Modified(K),
    /// A file or directory went away.
    Deleted(K),
    /// The old name of a rename.
    RenamedFrom {
        /// The old name.
        key: K,
        /// The file id, 0 when unknown.
        file_id: u128,
    },
    /// The new name of a rename.
    RenamedTo {
        /// The new name.
        key: K,
        /// The file id, 0 when unknown.
        file_id: u128,
    },
    /// A hole in the stream of a root (overflow, watch lost, journal gap).
    Gap {
        /// The root with the hole.
        root: u32,
    },
    /// A reconcile of a root finished.
    Reconciled {
        /// The reconciled root.
        root: u32,
    },
}

/// The fold's view of a stored row, or `None` for rows that say nothing about
/// a path: system rows, markers, watch states and the reconcile bookkeeping.
///
/// Metadata changes of a directory (every child change touches its parent)
/// are left out, as is a directory's modified row; created, deleted and
/// renamed directories stay. `root` is only used by gaps and reconciles.
#[must_use]
pub fn op_for<K>(kind: Kind, is_dir: bool, key: K, file_id: u128, root: u32) -> Option<Op<K>> {
    Some(match kind {
        Kind::Created | Kind::ReconciledCreated | Kind::ReconciledRenamed => Op::Created(key),
        Kind::Deleted | Kind::ReconciledDeleted => Op::Deleted(key),
        Kind::Modified
        | Kind::AttributesChanged
        | Kind::SecurityChanged
        | Kind::StreamChanged
        | Kind::LinkChanged
        | Kind::ReconciledModified => {
            if is_dir {
                return None;
            }
            Op::Modified(key)
        }
        Kind::RenamedFrom => Op::RenamedFrom { key, file_id },
        Kind::RenamedTo => Op::RenamedTo { key, file_id },
        Kind::Overflow(_) => Op::Gap { root },
        Kind::ReconcileFinished => Op::Reconciled { root },
        _ => return None,
    })
}

/// What a range did to one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum NetKind {
    /// The file did not exist before the range and does now.
    Created,
    /// The file existed before the range and is gone.
    Deleted,
    /// The file existed throughout and changed, or was deleted and created
    /// again under the same name (a replace).
    Modified,
    /// The file exists under another name.
    Renamed,
}

/// One row of the net change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Net<K> {
    /// What happened.
    pub kind: NetKind,
    /// The name now; the name before for [`NetKind::Deleted`].
    pub key: K,
    /// The name before, for [`NetKind::Renamed`].
    pub from: Option<K>,
    /// Whether the content or metadata also changed (renamed rows).
    pub modified: bool,
    /// How many raw rows this row folds ("raw events: n").
    pub raw_events: u32,
}

/// A hole in the range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GapInfo {
    /// The root with the hole.
    pub root: u32,
    /// Whether a reconcile of the root ran after it.
    pub reconciled: bool,
}

/// The result of a fold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary<K> {
    /// The net change, by kind and then key.
    pub rows: Vec<Net<K>>,
    /// Files created and deleted inside the range, hidden by default.
    pub transient: u32,
    /// Raw rows folded, transient ones included.
    pub raw_events: u64,
    /// The holes in the range, in order.
    pub gaps: Vec<GapInfo>,
}

impl<K> Summary<K> {
    /// Whether a hole in the range was never reconciled.
    #[must_use]
    pub fn incomplete(&self) -> bool {
        self.gaps.iter().any(|g| !g.reconciled)
    }
}

/// A file followed through the range.
#[derive(Debug, Clone, Copy)]
struct File<K> {
    /// The name before the range; `None` when the range created it.
    origin: Option<K>,
    /// The name now; `None` once deleted.
    cur: Option<K>,
    modified: bool,
    raw: u32,
}

/// A `RenamedFrom` waiting for its `RenamedTo`.
#[derive(Debug, Clone, Copy)]
struct Held<K> {
    key: K,
    file_id: u128,
}

/// The streaming fold.
#[derive(Debug)]
pub struct Fold<K> {
    files: Vec<Option<File<K>>>,
    free: Vec<usize>,
    by_name: HashMap<K, usize>,
    held: Vec<Held<K>>,
    transient: u32,
    raw_events: u64,
    gaps: Vec<GapInfo>,
}

impl<K: Copy + Eq + Hash + Ord> Default for Fold<K> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Copy + Eq + Hash + Ord> Fold<K> {
    /// An empty fold.
    #[must_use]
    pub fn new() -> Self {
        Self {
            files: Vec::new(),
            free: Vec::new(),
            by_name: HashMap::new(),
            held: Vec::new(),
            transient: 0,
            raw_events: 0,
            gaps: Vec::new(),
        }
    }

    /// Distinct surviving files held now (the memory the fold uses).
    #[must_use]
    pub fn live_files(&self) -> usize {
        self.by_name.len()
    }

    /// Fold one row; rows come in id order.
    pub fn push(&mut self, op: Op<K>) {
        self.raw_events += 1;
        match op {
            Op::RenamedFrom { key, file_id } => self.held.push(Held { key, file_id }),
            Op::RenamedTo { key, file_id } => self.renamed_to(key, file_id),
            other => {
                self.flush_held();
                self.apply(other);
            }
        }
    }

    /// Finish and report. Held rename halves count as deletes.
    #[must_use]
    pub fn finish(mut self) -> Summary<K> {
        self.flush_held();
        let mut rows = Vec::with_capacity(self.by_name.len());
        let mut deleted: Vec<File<K>> = Vec::new();
        for file in self.files.iter().flatten() {
            match (file.origin, file.cur) {
                (None, Some(key)) => rows.push(Net {
                    kind: NetKind::Created,
                    key,
                    from: None,
                    modified: false,
                    raw_events: file.raw,
                }),
                (Some(_), None) => deleted.push(*file),
                (Some(from), Some(key)) if from != key => rows.push(Net {
                    kind: NetKind::Renamed,
                    key,
                    from: Some(from),
                    modified: file.modified,
                    raw_events: file.raw,
                }),
                (Some(key), Some(_)) if file.modified => rows.push(Net {
                    kind: NetKind::Modified,
                    key,
                    from: None,
                    modified: true,
                    raw_events: file.raw,
                }),
                // Untouched, or renamed away and back: no net change.
                (Some(_), Some(_)) | (None, None) => {}
            }
        }
        // A file deleted and another created under the same name is a
        // replace: one modified row.
        for gone in deleted {
            let Some(key) = gone.origin else { continue };
            let created = rows
                .iter()
                .position(|r| r.kind == NetKind::Created && r.key == key);
            match created.map(|i| rows.swap_remove(i)) {
                Some(born) => rows.push(Net {
                    kind: NetKind::Modified,
                    key,
                    from: None,
                    modified: true,
                    raw_events: gone.raw.saturating_add(born.raw_events),
                }),
                None => rows.push(Net {
                    kind: NetKind::Deleted,
                    key,
                    from: None,
                    modified: false,
                    raw_events: gone.raw,
                }),
            }
        }
        rows.sort_by_key(|r| (r.kind, r.key));
        Summary {
            rows,
            transient: self.transient,
            raw_events: self.raw_events,
            gaps: self.gaps,
        }
    }

    fn renamed_to(&mut self, to: K, file_id: u128) {
        let hit = self
            .held
            .iter()
            .rposition(|h| file_id != 0 && h.file_id != 0 && h.file_id == file_id)
            .or_else(|| {
                // An unknown id only pairs with the very last held half.
                let last = self.held.len().checked_sub(1)?;
                let h = self.held.get(last)?;
                (file_id == 0 || h.file_id == 0).then_some(last)
            });
        if let Some(i) = hit {
            let half = self.held.remove(i);
            // Everything held before the match lost its partner.
            let older: Vec<Held<K>> = self.held.drain(..i).collect();
            for h in older {
                self.delete(h.key);
            }
            self.rename(half.key, to);
        } else {
            self.flush_held();
            self.create(to);
        }
    }

    fn flush_held(&mut self) {
        let held = std::mem::take(&mut self.held);
        for h in held {
            self.delete(h.key);
        }
    }

    fn apply(&mut self, op: Op<K>) {
        match op {
            Op::Created(key) => self.create(key),
            Op::Modified(key) => {
                let i = self.lookup(key);
                self.touch(i);
            }
            Op::Deleted(key) => self.delete(key),
            Op::Gap { root } => self.gaps.push(GapInfo {
                root,
                reconciled: false,
            }),
            Op::Reconciled { root } => {
                for g in self.gaps.iter_mut().filter(|g| g.root == root) {
                    g.reconciled = true;
                }
            }
            Op::RenamedFrom { .. } | Op::RenamedTo { .. } => {}
        }
    }

    fn touch(&mut self, i: usize) {
        if let Some(f) = self.file_mut(i) {
            f.modified = true;
            f.raw = f.raw.saturating_add(1);
        }
    }

    /// The slot of the file now named `key`, adopting a file that existed
    /// before the range when the name is new to the fold.
    fn lookup(&mut self, key: K) -> usize {
        if let Some(&i) = self.by_name.get(&key) {
            return i;
        }
        let i = self.alloc(File {
            origin: Some(key),
            cur: Some(key),
            modified: false,
            raw: 0,
        });
        self.by_name.insert(key, i);
        i
    }

    fn alloc(&mut self, file: File<K>) -> usize {
        if let Some(i) = self.free.pop() {
            if let Some(slot) = self.files.get_mut(i) {
                *slot = Some(file);
            }
            i
        } else {
            self.files.push(Some(file));
            self.files.len() - 1
        }
    }

    fn file_mut(&mut self, i: usize) -> Option<&mut File<K>> {
        self.files.get_mut(i).and_then(Option::as_mut)
    }

    fn create(&mut self, key: K) {
        if let Some(&i) = self.by_name.get(&key) {
            // The name is taken: a create we cannot explain (a missed
            // delete). Treat it as a change of the file we know.
            self.touch(i);
            return;
        }
        let i = self.alloc(File {
            origin: None,
            cur: Some(key),
            modified: false,
            raw: 1,
        });
        self.by_name.insert(key, i);
    }

    fn delete(&mut self, key: K) {
        let i = self.lookup(key);
        self.by_name.remove(&key);
        let Some(f) = self.file_mut(i) else { return };
        f.raw = f.raw.saturating_add(1);
        if f.origin.is_some() {
            f.cur = None;
        } else {
            // Born and gone inside the range: transient. Free the slot.
            if let Some(slot) = self.files.get_mut(i) {
                *slot = None;
            }
            self.free.push(i);
            self.transient = self.transient.saturating_add(1);
        }
    }

    fn rename(&mut self, from: K, to: K) {
        let i = self.lookup(from);
        self.by_name.remove(&from);
        // A file already at the target is overwritten.
        if self.by_name.contains_key(&to) {
            self.delete(to);
        }
        if let Some(f) = self.file_mut(i) {
            f.cur = Some(to);
            f.raw = f.raw.saturating_add(2);
        }
        self.by_name.insert(to, i);
    }
}

#[cfg(test)]
mod tests;
