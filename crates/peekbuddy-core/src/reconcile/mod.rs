//! The reconciliation walk (#46): compare the real tree with the last known
//! state and log what the live stream missed.
//!
//! It is not polling. It runs after an overflow, a ring-full gap, a lost and
//! restored watch, and at engine start, and it only compares: names, 128-bit
//! file ids, sizes, write and change times, attributes and reparse tags. It
//! never opens file data and never descends into a reparse point.
//!
//! The pieces are traits so everything runs on Linux against fakes
//! ([`memory`]): [`DirSource`] lists a directory, [`Snapshot`] is the last
//! known state, [`RowOut`] takes the rows. [`walk::run`] ties them together.
//!
//! # Rows
//!
//! All rows have `source = Reconcile` and carry the path relative to the
//! root. A walk with a snapshot writes `ReconcileStarted`, one row per
//! difference (`ReconciledCreated`, `ReconciledModified`,
//! `ReconciledDeleted`, `ReconciledRenamed`, `AccessDenied`) and
//! `ReconcileFinished`. The first walk of a root only writes the snapshot and
//! one `BaselineEstablished` row.
//!
//! # What counts as a difference
//!
//! - Same name, same kind: modified when the file id, attributes or reparse
//!   tag differ, or, for files, the size, write time or change time. A
//!   directory's own times and size change with its entries and are not
//!   compared.
//! - Same name, file turned into a directory or the reverse: deleted, then
//!   created.
//! - A name only in the snapshot and a name only on disk with the same file
//!   id and kind: renamed. This pairs moves across directories too, and a
//!   moved directory pairs every entry below it.
//! - Everything else only in the snapshot is deleted (a directory takes its
//!   whole known subtree with it), everything else only on disk is created.
//!
//! Renames pair file ids that are equal in all 128 bits. A file system that
//! reuses ids can produce a false pair; FAT and exFAT ids are not stable, so
//! on those roots a rename degrades to delete and create.

pub mod dirinfo;
pub mod memory;
pub mod walk;

use std::cmp::Ordering;

use crate::event::Event;

/// What is known about one entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Meta {
    /// The 128-bit file id.
    pub file_id: u128,
    /// `FileAttributes`.
    pub attributes: u32,
    /// `ReparsePointTag`.
    pub reparse_tag: u32,
    /// `EndOfFile`.
    pub size: u64,
    /// `LastWriteTime`.
    pub write_time: i64,
    /// `ChangeTime`.
    pub change_time: i64,
}

impl Meta {
    /// Whether the entry is a directory.
    #[must_use]
    pub const fn is_dir(&self) -> bool {
        self.attributes & dirinfo::ATTR_DIRECTORY != 0
    }

    /// Whether the entry is a reparse point.
    #[must_use]
    pub const fn is_reparse(&self) -> bool {
        self.attributes & dirinfo::ATTR_REPARSE_POINT != 0
    }

    /// Whether `self` (new) differs from `old` in a way worth a row.
    #[must_use]
    pub const fn differs_from(&self, old: &Self) -> bool {
        if self.file_id != old.file_id
            || self.attributes != old.attributes
            || self.reparse_tag != old.reparse_tag
        {
            return true;
        }
        !self.is_dir()
            && (self.size != old.size
                || self.write_time != old.write_time
                || self.change_time != old.change_time)
    }
}

/// One entry of a [`Listing`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Item {
    start: usize,
    len: usize,
    /// What is known about the entry.
    pub meta: Meta,
    /// The snapshot's id of the entry (0 for an entry read from disk).
    pub id: i64,
}

/// A directory's entries with their names in one buffer, reused between
/// directories so a walk allocates a constant amount per listing.
#[derive(Debug, Default)]
pub struct Listing {
    items: Vec<Item>,
    units: Vec<u16>,
}

impl Listing {
    /// An empty listing.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Forgets the entries; the storage stays.
    pub fn clear(&mut self) {
        self.items.clear();
        self.units.clear();
    }

    /// Adds an entry.
    pub fn push(&mut self, name: &[u16], meta: Meta, id: i64) {
        self.items.push(Item {
            start: self.units.len(),
            len: name.len(),
            meta,
            id,
        });
        self.units.extend_from_slice(name);
    }

    /// Adds an entry whose name is given as an iterator of units.
    pub fn push_units(
        &mut self,
        len: usize,
        units: impl Iterator<Item = u16>,
        meta: Meta,
        id: i64,
    ) {
        let start = self.units.len();
        self.units.extend(units.take(len));
        let len = self.units.len() - start;
        self.items.push(Item {
            start,
            len,
            meta,
            id,
        });
    }

    /// The entries, in the order they were added or sorted.
    #[must_use]
    pub fn items(&self) -> &[Item] {
        &self.items
    }

    /// The number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether there are no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The name of `item`.
    #[must_use]
    pub fn name(&self, item: &Item) -> &[u16] {
        self.units
            .get(item.start..item.start + item.len)
            .unwrap_or_default()
    }

    /// Sorts the entries by name (code unit order).
    pub fn sort(&mut self) {
        let units = &self.units;
        self.items.sort_unstable_by(|a, b| {
            let name = |i: &Item| units.get(i.start..i.start + i.len).unwrap_or_default();
            name(a).cmp(name(b))
        });
    }

    /// Compares the names of two items of this listing and another.
    #[must_use]
    pub fn cmp_names(&self, a: &Item, other: &Self, b: &Item) -> Ordering {
        self.name(a).cmp(other.name(b))
    }
}

/// Why a directory could not be listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListError {
    /// Access was denied: the directory stays as it was in the snapshot.
    AccessDenied,
    /// The directory is gone (deleted since its parent was listed): it counts
    /// as empty.
    Gone,
    /// Any other failure: treated like a denied directory.
    Other,
}

/// Lists directories of a root.
pub trait DirSource {
    /// Fills `out` with the entries of the directory at `path` (relative to
    /// the root, `\` separated, empty for the root itself), without `.` and
    /// `..`.
    ///
    /// # Errors
    ///
    /// Why the directory could not be listed.
    fn list(&mut self, path: &[u16], out: &mut Listing) -> Result<(), ListError>;
}

/// The last known state of the trees, kept on disk.
///
/// A walk works inside one transaction: [`Snapshot::begin`] first, then
/// reads and writes, then [`Snapshot::commit`] (or [`Snapshot::rollback`]
/// when the walk was cancelled or its rows could not be made durable), so the
/// snapshot never moves ahead of the rows that explain it.
pub trait Snapshot {
    /// The storage error.
    type Error: std::fmt::Display;

    /// Whether the root has a baseline.
    ///
    /// # Errors
    ///
    /// The storage error.
    fn is_baselined(&mut self, root: u16) -> Result<bool, Self::Error>;
    /// Marks the root as baselined.
    ///
    /// # Errors
    ///
    /// The storage error.
    fn set_baselined(&mut self, root: u16) -> Result<(), Self::Error>;
    /// Starts the transaction.
    ///
    /// # Errors
    ///
    /// The storage error.
    fn begin(&mut self) -> Result<(), Self::Error>;
    /// Makes the transaction durable.
    ///
    /// # Errors
    ///
    /// The storage error.
    fn commit(&mut self) -> Result<(), Self::Error>;
    /// Drops the transaction.
    fn rollback(&mut self);
    /// Fills `out` with the known children of `parent` (0 is the root
    /// directory itself).
    ///
    /// # Errors
    ///
    /// The storage error.
    fn children(&mut self, root: u16, parent: i64, out: &mut Listing) -> Result<(), Self::Error>;
    /// Inserts or updates the entry `name` below `parent`; `existing` is its
    /// id when it is known. Returns the id.
    ///
    /// # Errors
    ///
    /// The storage error.
    fn upsert(
        &mut self,
        root: u16,
        parent: i64,
        name: &[u16],
        meta: &Meta,
        existing: Option<i64>,
    ) -> Result<i64, Self::Error>;
    /// Removes one entry (its children are removed by their own calls).
    ///
    /// # Errors
    ///
    /// The storage error.
    fn remove(&mut self, root: u16, id: i64) -> Result<(), Self::Error>;
}

/// Where the rows of a walk go.
pub trait RowOut {
    /// Takes one row; `name` is the path relative to the root.
    fn row(&mut self, event: &Event, name: &[u16]);
    /// Waits until the rows so far are durable; `false` when they are not.
    fn flush(&mut self) -> bool;
}
