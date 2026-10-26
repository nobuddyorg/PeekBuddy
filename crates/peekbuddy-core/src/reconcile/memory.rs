//! In-memory stand-ins for the reconcile traits: a directory tree, a
//! snapshot and a row collector. They make the walk testable on any host, and
//! the property tests use them as the model.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::convert::Infallible;

use super::{DirSource, ListError, Listing, Meta, RowOut, Snapshot};
use crate::event::{Event, Kind};

const SEPARATOR: u16 = b'\\' as u16;

fn split(path: &[u16]) -> (&[u16], &[u16]) {
    match path.iter().rposition(|&unit| unit == SEPARATOR) {
        Some(at) => (
            path.get(..at).unwrap_or_default(),
            path.get(at + 1..).unwrap_or_default(),
        ),
        None => (&[], path),
    }
}

/// A directory tree in memory: full relative path to metadata.
#[derive(Debug, Clone, Default)]
pub struct MemoryTree {
    entries: BTreeMap<Vec<u16>, Meta>,
    index: HashMap<Vec<u16>, Vec<Vec<u16>>>,
    denied: HashSet<Vec<u16>>,
    /// How many directories were listed.
    pub listed: usize,
}

impl MemoryTree {
    /// An empty tree (a root with nothing in it).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the entry at `path`; its parents must be set too.
    pub fn set(&mut self, path: &[u16], meta: Meta) {
        if self.entries.insert(path.to_vec(), meta).is_none() {
            let (dir, _) = split(path);
            self.index
                .entry(dir.to_vec())
                .or_default()
                .push(path.to_vec());
        }
    }

    /// Removes the entry at `path` (not its children).
    pub fn unset(&mut self, path: &[u16]) {
        if self.entries.remove(path).is_some() {
            let (dir, _) = split(path);
            if let Some(list) = self.index.get_mut(dir) {
                list.retain(|p| p.as_slice() != path);
            }
        }
    }

    /// Makes the directory at `path` refuse to be listed.
    pub fn deny(&mut self, path: &[u16]) {
        self.denied.insert(path.to_vec());
    }

    /// All entries, by path.
    #[must_use]
    pub const fn entries(&self) -> &BTreeMap<Vec<u16>, Meta> {
        &self.entries
    }
}

impl DirSource for MemoryTree {
    fn list(&mut self, path: &[u16], out: &mut Listing) -> Result<(), ListError> {
        self.listed += 1;
        if self.denied.contains(path) {
            return Err(ListError::AccessDenied);
        }
        if !path.is_empty() && !self.entries.contains_key(path) {
            return Err(ListError::Gone);
        }
        for child in self.index.get(path).into_iter().flatten() {
            if let Some(meta) = self.entries.get(child) {
                out.push(split(child).1, *meta, 0);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Stored {
    root: u16,
    parent: i64,
    name: Vec<u16>,
    meta: Meta,
}

/// A snapshot in memory with real transactions (a copy taken at `begin`).
#[derive(Debug, Clone, Default)]
pub struct MemorySnapshot {
    rows: BTreeMap<i64, Stored>,
    next: i64,
    baselined: HashSet<u16>,
    saved: Option<(BTreeMap<i64, Stored>, i64, HashSet<u16>)>,
}

impl MemorySnapshot {
    /// An empty snapshot.
    #[must_use]
    pub fn new() -> Self {
        Self {
            next: 1,
            ..Self::default()
        }
    }

    /// Every known entry of `root` as a path-to-metadata map, the shape of
    /// [`MemoryTree::entries`], for comparing.
    #[must_use]
    pub fn paths(&self, root: u16) -> BTreeMap<Vec<u16>, Meta> {
        let mut out = BTreeMap::new();
        for (id, row) in &self.rows {
            if row.root != root {
                continue;
            }
            let mut path = row.name.clone();
            let mut parent = row.parent;
            let mut hops = 0;
            while parent != 0 && hops < 10_000 {
                let Some(up) = self.rows.get(&parent) else {
                    break;
                };
                let mut joined = up.name.clone();
                joined.push(SEPARATOR);
                joined.extend_from_slice(&path);
                path = joined;
                parent = up.parent;
                hops += 1;
            }
            let _ = id;
            out.insert(path, row.meta);
        }
        out
    }

    /// The number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether the snapshot is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

impl Snapshot for MemorySnapshot {
    type Error = Infallible;

    fn is_baselined(&mut self, root: u16) -> Result<bool, Infallible> {
        Ok(self.baselined.contains(&root))
    }

    fn set_baselined(&mut self, root: u16) -> Result<(), Infallible> {
        self.baselined.insert(root);
        Ok(())
    }

    fn begin(&mut self) -> Result<(), Infallible> {
        self.saved = Some((self.rows.clone(), self.next, self.baselined.clone()));
        Ok(())
    }

    fn commit(&mut self) -> Result<(), Infallible> {
        self.saved = None;
        Ok(())
    }

    fn rollback(&mut self) {
        if let Some((rows, next, baselined)) = self.saved.take() {
            self.rows = rows;
            self.next = next;
            self.baselined = baselined;
        }
    }

    fn children(&mut self, root: u16, parent: i64, out: &mut Listing) -> Result<(), Infallible> {
        for (id, row) in &self.rows {
            if row.root == root && row.parent == parent {
                out.push(&row.name, row.meta, *id);
            }
        }
        Ok(())
    }

    fn upsert(
        &mut self,
        root: u16,
        parent: i64,
        name: &[u16],
        meta: &Meta,
        existing: Option<i64>,
    ) -> Result<i64, Infallible> {
        let id = existing.unwrap_or_else(|| {
            let id = self.next;
            self.next += 1;
            id
        });
        self.rows.insert(
            id,
            Stored {
                root,
                parent,
                name: name.to_vec(),
                meta: *meta,
            },
        );
        Ok(id)
    }

    fn remove(&mut self, _root: u16, id: i64) -> Result<(), Infallible> {
        self.rows.remove(&id);
        Ok(())
    }
}

/// Collects rows as `(kind, path)` pairs.
#[derive(Debug, Default)]
pub struct Rows {
    /// The rows, in order.
    pub rows: Vec<(Kind, Vec<u16>, Event)>,
    /// What [`RowOut::flush`] answers.
    pub durable: bool,
}

impl Rows {
    /// A collector whose flush succeeds.
    #[must_use]
    pub fn new() -> Self {
        Self {
            rows: Vec::new(),
            durable: true,
        }
    }

    /// The `(kind, path as text)` of the rows, for assertions.
    #[must_use]
    pub fn summary(&self) -> Vec<(Kind, String)> {
        self.rows
            .iter()
            .map(|(kind, path, _)| (*kind, String::from_utf16_lossy(path)))
            .collect()
    }

    /// The rows that describe a difference, not a state.
    #[must_use]
    pub fn differences(&self) -> Vec<(Kind, String)> {
        self.summary()
            .into_iter()
            .filter(|(kind, _)| {
                matches!(
                    kind,
                    Kind::ReconciledCreated
                        | Kind::ReconciledModified
                        | Kind::ReconciledDeleted
                        | Kind::ReconciledRenamed
                        | Kind::AccessDenied
                )
            })
            .collect()
    }
}

impl RowOut for Rows {
    fn row(&mut self, event: &Event, name: &[u16]) {
        self.rows.push((event.kind, name.to_vec(), *event));
    }

    fn flush(&mut self) -> bool {
        self.durable
    }
}
