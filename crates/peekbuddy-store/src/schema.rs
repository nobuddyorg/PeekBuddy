//! Schema v1, migrations and the writer's statements (#53).
//!
//! Every table is `STRICT`, has no foreign keys and no `AUTOINCREMENT`. The
//! DDL is split into a segment part ([`SEGMENT_DDL`]: events, stats, cursors,
//! meta) and a dictionary part ([`DICTIONARY_DDL`]: the interned directory
//! tree and its search index, the roots and the volumes) so that #54 can put
//! the dictionary in a catalog or replicate it per segment.
//!
//! Enum discriminants stored in the tables (`kind`, `source`, `name_kind`,
//! `flags` bits) are on-disk format: append-only, never renumbered. They are
//! snapshot-tested.
//!
//! # Migrations
//!
//! [`migrate`] applies ordered, forward-only steps, each in its own
//! transaction that also bumps `meta.schema_version`. A database with a newer
//! schema than this build supports fails with [`StoreError::TooNew`] (an old
//! viewer facing a new engine). A database that is not a PeekBuddy segment
//! fails with [`StoreError::NotASegment`].

use std::fmt;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, Transaction};

/// The schema version this build writes.
pub const SCHEMA_VERSION: u32 = 1;

/// `PRAGMA application_id` of a PeekBuddy segment: ASCII `PBK1`.
pub const APPLICATION_ID: i32 = 0x5042_4B31;

/// FILETIME ticks (100 ns) per hour: the bucket size of `stats.hour_ts`.
pub const TICKS_PER_HOUR: i64 = 36_000_000_000;

/// The hour bucket of a FILETIME timestamp, for `stats.hour_ts`.
#[must_use]
pub const fn hour_bucket(ts: i64) -> i64 {
    ts.div_euclid(TICKS_PER_HOUR) * TICKS_PER_HOUR
}

/// The dictionary part: nodes, their search index, roots, volumes and the
/// reserved process table.
pub const DICTIONARY_DDL: &str = "\
CREATE TABLE node (
  id INTEGER PRIMARY KEY,
  vol_id INTEGER NOT NULL,
  parent_id INTEGER NOT NULL,
  name TEXT NOT NULL,
  name_w BLOB,
  name_kind INTEGER NOT NULL,
  file_id INTEGER
) STRICT;
CREATE UNIQUE INDEX node_key ON node(parent_id, name, coalesce(name_w, x''));
CREATE VIRTUAL TABLE node_fts USING fts5(
  name,
  content='node',
  content_rowid='id',
  tokenize='trigram case_sensitive 0 remove_diacritics 1',
  detail='none'
);
CREATE TABLE root (
  id INTEGER PRIMARY KEY,
  path TEXT NOT NULL,
  path_w BLOB,
  recursive INTEGER NOT NULL,
  vol_id INTEGER NOT NULL
) STRICT;
CREATE TABLE vol (
  id INTEGER PRIMARY KEY,
  guid TEXT NOT NULL,
  serial INTEGER NOT NULL,
  letter TEXT,
  journal_id INTEGER
) STRICT;
CREATE TABLE proc (
  id INTEGER PRIMARY KEY,
  pid INTEGER NOT NULL,
  image TEXT NOT NULL,
  started INTEGER
) STRICT;
";

/// The segment part: events, hourly stats, capture cursors and meta.
pub const SEGMENT_DDL: &str = "\
CREATE TABLE event (
  id INTEGER PRIMARY KEY,
  ts INTEGER NOT NULL,
  kind INTEGER NOT NULL,
  source INTEGER NOT NULL,
  raw_mask INTEGER NOT NULL,
  node_id INTEGER NOT NULL,
  old_node_id INTEGER,
  is_dir INTEGER NOT NULL,
  size INTEGER,
  attrs INTEGER,
  reparse_tag INTEGER,
  file_id INTEGER,
  file_id_hi INTEGER,
  usn INTEGER,
  vol_id INTEGER NOT NULL,
  root_id INTEGER NOT NULL,
  flags INTEGER NOT NULL,
  pid INTEGER,
  proc_id INTEGER
) STRICT;
CREATE INDEX event_node ON event(node_id, id);
CREATE INDEX event_root ON event(root_id, id);
CREATE TABLE stats (
  hour_ts INTEGER NOT NULL,
  root_id INTEGER NOT NULL,
  kind INTEGER NOT NULL,
  n INTEGER NOT NULL,
  min_id INTEGER NOT NULL,
  max_id INTEGER NOT NULL,
  PRIMARY KEY (hour_ts, root_id, kind)
) STRICT, WITHOUT ROWID;
CREATE TABLE cursor (
  root_id INTEGER NOT NULL,
  vol_id INTEGER NOT NULL,
  usn INTEGER,
  last_event_id INTEGER NOT NULL,
  ts INTEGER NOT NULL,
  PRIMARY KEY (root_id, vol_id)
) STRICT, WITHOUT ROWID;
CREATE TABLE meta (
  key TEXT PRIMARY KEY,
  value ANY
) STRICT, WITHOUT ROWID;
";

/// Inserts one event; the writer assigns `id` inside the commit transaction.
pub const INSERT_EVENT: &str = "\
INSERT INTO event (id, ts, kind, source, raw_mask, node_id, old_node_id, is_dir, size, attrs, \
reparse_tag, file_id, file_id_hi, usn, vol_id, root_id, flags, pid, proc_id) \
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)";

/// Interns a node. A conflict means the node exists (a cold interner after a
/// restart); the caller then looks its id up.
pub const INSERT_NODE: &str = "\
INSERT INTO node (vol_id, parent_id, name, name_w, name_kind, file_id) \
VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
ON CONFLICT (parent_id, name, coalesce(name_w, x'')) DO NOTHING";

/// Looks up an existing node by its key.
pub const SELECT_NODE: &str = "\
SELECT id FROM node WHERE parent_id = ?1 AND name = ?2 AND coalesce(name_w, x'') = coalesce(?3, x'')";

/// Indexes a new node's name for search; written by the writer, no triggers.
pub const INSERT_NODE_FTS: &str = "INSERT INTO node_fts (rowid, name) VALUES (?1, ?2)";

/// One UPSERT per `(hour, root, kind)` key per batch.
pub const UPSERT_STATS: &str = "\
INSERT INTO stats (hour_ts, root_id, kind, n, min_id, max_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
ON CONFLICT (hour_ts, root_id, kind) DO UPDATE SET \
n = n + excluded.n, min_id = min(min_id, excluded.min_id), max_id = max(max_id, excluded.max_id)";

/// A store error.
#[derive(Debug)]
pub enum StoreError {
    /// SQLite failed.
    Sqlite(rusqlite::Error),
    /// The segment has a newer schema than this build supports.
    TooNew {
        /// The version in the file.
        found: u32,
        /// The newest version this build knows.
        supported: u32,
    },
    /// The file is not a PeekBuddy segment.
    NotASegment {
        /// Its `application_id`.
        application_id: i32,
    },
    /// `meta.schema_version` is not a valid version.
    BadVersion,
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(e) => write!(f, "SQLite error: {e}"),
            Self::TooNew { found, supported } => write!(
                f,
                "segment schema {found} is newer than the supported {supported}; \
                 update PeekBuddy"
            ),
            Self::NotASegment { application_id } => {
                write!(
                    f,
                    "not a PeekBuddy segment (application_id {application_id:#x})"
                )
            }
            Self::BadVersion => f.write_str("meta.schema_version is not a valid version"),
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(e) => Some(e),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sqlite(e)
    }
}

/// One forward-only migration step.
pub type Step = fn(&Transaction<'_>) -> rusqlite::Result<()>;

/// The migration steps; step `i` upgrades schema `i` to `i + 1`.
pub const STEPS: [Step; 1] = [v1];

fn v1(tx: &Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute_batch(DICTIONARY_DDL)?;
    tx.execute_batch(SEGMENT_DDL)
}

/// Opens (creating if needed) the segment at `path` and migrates it.
///
/// # Errors
///
/// [`StoreError`] when SQLite fails, the file is not a segment or its schema
/// is newer than this build.
pub fn open(path: &Path) -> Result<Connection, StoreError> {
    let mut conn = Connection::open(path)?;
    migrate(&mut conn)?;
    Ok(conn)
}

/// Migrates `conn` to [`SCHEMA_VERSION`]; returns the version it ends at.
///
/// # Errors
///
/// See [`open`].
pub fn migrate(conn: &mut Connection) -> Result<u32, StoreError> {
    migrate_with(conn, &STEPS)
}

/// [`migrate`] with explicit steps, so tests can exercise the framework.
///
/// # Errors
///
/// See [`open`].
pub fn migrate_with(conn: &mut Connection, steps: &[Step]) -> Result<u32, StoreError> {
    let supported = u32::try_from(steps.len()).map_err(|_| StoreError::BadVersion)?;
    let current = current_version(conn)?;
    if current > supported {
        return Err(StoreError::TooNew {
            found: current,
            supported,
        });
    }
    for (index, step) in steps.iter().enumerate().skip(current as usize) {
        let version = i64::try_from(index + 1).map_err(|_| StoreError::BadVersion)?;
        let tx = conn.transaction()?;
        step(&tx)?;
        tx.execute(
            "INSERT INTO meta (key, value) VALUES ('schema_version', ?1) \
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            [version],
        )?;
        tx.commit()?;
    }
    conn.pragma_update(None, "application_id", APPLICATION_ID)?;
    Ok(supported)
}

/// The schema version of `conn`: 0 for an empty database.
fn current_version(conn: &Connection) -> Result<u32, StoreError> {
    let application_id: i32 = conn.pragma_query_value(None, "application_id", |row| row.get(0))?;
    let tables: i64 = conn.query_row(
        "SELECT count(*) FROM sqlite_schema WHERE type = 'table'",
        [],
        |row| row.get(0),
    )?;
    if tables == 0 {
        return Ok(0);
    }
    if application_id != APPLICATION_ID {
        return Err(StoreError::NotASegment { application_id });
    }
    let version: Option<i64> = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    version
        .and_then(|v| u32::try_from(v).ok())
        .ok_or(StoreError::BadVersion)
}
