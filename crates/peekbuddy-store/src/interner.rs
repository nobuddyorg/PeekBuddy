//! The writer-side interner for the `node` table (#53).
//!
//! It maps `(parent_id, name)` to a node id in memory, so ingest never does a
//! SELECT-or-INSERT: a hit allocates nothing and touches no SQL. A miss inserts
//! the node and its search row inside the caller's transaction. After a restart
//! the map is empty, so a miss first tries the `INSERT ... ON CONFLICT DO
//! NOTHING` and looks the id up only when the node already existed.
//!
//! Its size grows with distinct nodes, not with event count; [`Interner::stats`]
//! reports entries and bytes for the perf probe (#24).

use std::collections::HashMap;

use peekbuddy_core::event::NameKind;
use peekbuddy_core::name::WideStr;
use rusqlite::{Connection, OptionalExtension, params};

use crate::names::to_columns;
use crate::schema::{INSERT_NODE, INSERT_NODE_FTS, SELECT_NODE};

/// The parent id of top-level nodes: never NULL (a NULL would defeat the
/// unique index).
pub const TOP_LEVEL: i64 = 0;

/// What the interner holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InternerStats {
    /// Distinct `(parent, name)` entries.
    pub entries: usize,
    /// Bytes of name units held.
    pub name_bytes: usize,
}

/// A node to intern.
#[derive(Debug, Clone, Copy)]
pub struct NodeKey<'a> {
    /// The volume row.
    pub vol_id: i64,
    /// The parent node, or [`TOP_LEVEL`].
    pub parent_id: i64,
    /// The exact name.
    pub name: WideStr<'a>,
    /// Which kind of name it is.
    pub name_kind: NameKind,
    /// The file id's low half, when known.
    pub file_id: Option<i64>,
}

/// The in-memory node dictionary.
#[derive(Debug, Default)]
pub struct Interner {
    by_parent: HashMap<i64, HashMap<Box<[u16]>, i64>>,
    stats: InternerStats,
}

impl Interner {
    /// An empty interner.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Entries and bytes held.
    #[must_use]
    pub const fn stats(&self) -> InternerStats {
        self.stats
    }

    /// Forgets every entry. Call it after the transaction that interned them
    /// rolled back, so ids that never reached disk are not handed out.
    pub fn clear(&mut self) {
        self.by_parent.clear();
        self.stats = InternerStats::default();
    }

    /// The id of a node already in memory; no SQL, no allocation.
    #[must_use]
    pub fn lookup(&self, parent_id: i64, name: WideStr<'_>) -> Option<i64> {
        self.by_parent
            .get(&parent_id)
            .and_then(|names| names.get(name.units()))
            .copied()
    }

    /// The id of `key`, inserting the node (and its search row) on a miss.
    ///
    /// Call it inside the batch transaction.
    ///
    /// # Errors
    ///
    /// SQLite failures.
    pub fn intern(&mut self, conn: &Connection, key: &NodeKey<'_>) -> rusqlite::Result<i64> {
        if let Some(id) = self.lookup(key.parent_id, key.name) {
            return Ok(id);
        }
        let columns = to_columns(key.name);
        let inserted = conn.prepare_cached(INSERT_NODE)?.execute(params![
            key.vol_id,
            key.parent_id,
            columns.text,
            columns.wide,
            i64::from(key.name_kind.tag()),
            key.file_id,
        ])?;
        let id = if inserted == 1 {
            let id = conn.last_insert_rowid();
            conn.prepare_cached(INSERT_NODE_FTS)?
                .execute(params![id, columns.text])?;
            id
        } else {
            conn.prepare_cached(SELECT_NODE)?
                .query_row(params![key.parent_id, columns.text, columns.wide], |row| {
                    row.get(0)
                })
                .optional()?
                .ok_or(rusqlite::Error::QueryReturnedNoRows)?
        };
        self.stats.entries += 1;
        self.stats.name_bytes += key.name.len() * 2;
        self.by_parent
            .entry(key.parent_id)
            .or_default()
            .insert(key.name.units().into(), id);
        Ok(id)
    }
}
