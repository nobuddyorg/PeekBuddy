//! The last known state of the watched trees (#46), on disk.
//!
//! The reconcile walk compares the real tree with this and updates it. It
//! lives in its own file (`snapshot.db` in the data directory) because the
//! event segments rotate by day and retention deletes them, while the
//! snapshot has to outlive both. Memory never scales with the tree: a walk
//! reads one directory's children at a time.
//!
//! One walk is one transaction. The caller commits only after the rows that
//! explain the changes are durable in the event store, so a crash can replay a
//! reconcile (rows twice) but never advance the snapshot past committed rows.
//!
//! Names are kept as exact UTF-16LE bytes (F17); file ids as 16 bytes.

use std::fmt;
use std::path::Path;

use peekbuddy_core::reconcile::{Listing, Meta, Snapshot};
use rusqlite::{Connection, OptionalExtension, params};

/// The schema version this build writes, in `PRAGMA user_version`.
pub const VERSION: i64 = 1;

const DDL: &str = "\
CREATE TABLE IF NOT EXISTS entry (
    id INTEGER PRIMARY KEY,
    root INTEGER NOT NULL,
    parent INTEGER NOT NULL,
    name BLOB NOT NULL,
    file_id BLOB NOT NULL,
    attrs INTEGER NOT NULL,
    tag INTEGER NOT NULL,
    size INTEGER NOT NULL,
    write_time INTEGER NOT NULL,
    change_time INTEGER NOT NULL
) STRICT;
CREATE UNIQUE INDEX IF NOT EXISTS entry_key ON entry (root, parent, name);
CREATE TABLE IF NOT EXISTS root_state (
    root INTEGER PRIMARY KEY,
    baselined INTEGER NOT NULL
) STRICT;";

/// Why the snapshot could not be opened.
#[derive(Debug)]
pub enum OpenError {
    /// SQLite failed.
    Sqlite(rusqlite::Error),
    /// The file is from a newer build.
    TooNew(i64),
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(error) => write!(f, "SQLite error: {error}"),
            Self::TooNew(version) => {
                write!(
                    f,
                    "snapshot.db is version {version}; this build reads {VERSION}"
                )
            }
        }
    }
}

impl std::error::Error for OpenError {}

impl From<rusqlite::Error> for OpenError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

/// The snapshot database.
#[derive(Debug)]
pub struct SnapshotDb {
    conn: Connection,
}

fn units_to_bytes(name: &[u16]) -> Vec<u8> {
    name.iter().flat_map(|unit| unit.to_le_bytes()).collect()
}

fn bytes_to_units(bytes: &[u8]) -> impl ExactSizeIterator<Item = u16> + '_ {
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&pair| u16::from_le_bytes(pair))
}

fn to_row(meta: &Meta) -> (Vec<u8>, i64, i64, i64, i64, i64) {
    (
        meta.file_id.to_le_bytes().to_vec(),
        i64::from(meta.attributes),
        i64::from(meta.reparse_tag),
        meta.size.cast_signed(),
        meta.write_time,
        meta.change_time,
    )
}

impl SnapshotDb {
    /// Opens (creating) the database at `path`.
    ///
    /// # Errors
    ///
    /// SQLite's error, or [`OpenError::TooNew`].
    pub fn open(path: &Path) -> Result<Self, OpenError> {
        Self::from_connection(Connection::open(path)?)
    }

    /// A database in memory, for tests.
    ///
    /// # Errors
    ///
    /// SQLite's error.
    pub fn open_in_memory() -> Result<Self, OpenError> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(conn: Connection) -> Result<Self, OpenError> {
        let _mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
        conn.execute_batch(
            "PRAGMA synchronous = FULL;\n\
             PRAGMA temp_store = MEMORY;\n\
             PRAGMA busy_timeout = 5000;\n\
             PRAGMA cache_size = -2048;",
        )?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version > VERSION {
            return Err(OpenError::TooNew(version));
        }
        conn.execute_batch(DDL)?;
        conn.execute_batch(&format!("PRAGMA user_version = {VERSION};"))?;
        Ok(Self { conn })
    }

    /// Forgets everything known about `root` (the watch was removed or its
    /// path changed): the next walk is a baseline again.
    ///
    /// # Errors
    ///
    /// SQLite's error.
    pub fn forget_root(&mut self, root: u16) -> rusqlite::Result<()> {
        self.conn
            .execute("DELETE FROM entry WHERE root = ?1", [i64::from(root)])?;
        self.conn
            .execute("DELETE FROM root_state WHERE root = ?1", [i64::from(root)])?;
        Ok(())
    }

    /// The number of entries known for `root`.
    ///
    /// # Errors
    ///
    /// SQLite's error.
    pub fn count(&self, root: u16) -> rusqlite::Result<u64> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM entry WHERE root = ?1",
            [i64::from(root)],
            |row| row.get(0),
        )?;
        Ok(u64::try_from(n).unwrap_or(0))
    }

    /// Every known path of `root` with its metadata, for tests and tooling.
    ///
    /// # Errors
    ///
    /// SQLite's error.
    pub fn paths(&self, root: u16) -> rusqlite::Result<std::collections::BTreeMap<Vec<u16>, Meta>> {
        let mut statement = self.conn.prepare(
            "SELECT id, parent, name, file_id, attrs, tag, size, write_time, change_time \
             FROM entry WHERE root = ?1",
        )?;
        let mut rows = statement.query([i64::from(root)])?;
        let mut flat: std::collections::HashMap<i64, (i64, Vec<u16>, Meta)> =
            std::collections::HashMap::new();
        while let Some(row) = rows.next()? {
            let meta = read_meta(row, 3)?;
            let name: Vec<u8> = row.get(2)?;
            flat.insert(
                row.get(0)?,
                (row.get(1)?, bytes_to_units(&name).collect(), meta),
            );
        }
        let mut out = std::collections::BTreeMap::new();
        for (parent, name, meta) in flat.values() {
            let mut path = name.clone();
            let mut up = *parent;
            while up != 0 {
                let Some((next, parent_name, _)) = flat.get(&up) else {
                    break;
                };
                let mut joined = parent_name.clone();
                joined.push(u16::from(b'\\'));
                joined.extend_from_slice(&path);
                path = joined;
                up = *next;
            }
            out.insert(path, *meta);
        }
        Ok(out)
    }
}

fn read_meta(row: &rusqlite::Row<'_>, from: usize) -> rusqlite::Result<Meta> {
    let id: Vec<u8> = row.get(from)?;
    let id: [u8; 16] = id.try_into().map_err(|_| rusqlite::Error::InvalidQuery)?;
    Ok(Meta {
        file_id: u128::from_le_bytes(id),
        attributes: u32::try_from(row.get::<_, i64>(from + 1)?).unwrap_or(0),
        reparse_tag: u32::try_from(row.get::<_, i64>(from + 2)?).unwrap_or(0),
        size: row.get::<_, i64>(from + 3)?.cast_unsigned(),
        write_time: row.get(from + 4)?,
        change_time: row.get(from + 5)?,
    })
}

impl Snapshot for SnapshotDb {
    type Error = rusqlite::Error;

    fn is_baselined(&mut self, root: u16) -> rusqlite::Result<bool> {
        let baselined: Option<i64> = self
            .conn
            .query_row(
                "SELECT baselined FROM root_state WHERE root = ?1",
                [i64::from(root)],
                |row| row.get(0),
            )
            .optional()?;
        Ok(baselined == Some(1))
    }

    fn set_baselined(&mut self, root: u16) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO root_state (root, baselined) VALUES (?1, 1) \
             ON CONFLICT (root) DO UPDATE SET baselined = 1",
            [i64::from(root)],
        )?;
        Ok(())
    }

    fn begin(&mut self) -> rusqlite::Result<()> {
        self.conn.execute_batch("BEGIN IMMEDIATE")
    }

    fn commit(&mut self) -> rusqlite::Result<()> {
        self.conn.execute_batch("COMMIT")
    }

    fn rollback(&mut self) {
        let _ = self.conn.execute_batch("ROLLBACK");
    }

    fn children(&mut self, root: u16, parent: i64, out: &mut Listing) -> rusqlite::Result<()> {
        let mut statement = self.conn.prepare_cached(
            "SELECT id, name, file_id, attrs, tag, size, write_time, change_time \
             FROM entry WHERE root = ?1 AND parent = ?2",
        )?;
        let mut rows = statement.query(params![i64::from(root), parent])?;
        while let Some(row) = rows.next()? {
            let name: Vec<u8> = row.get(1)?;
            let meta = read_meta(row, 2)?;
            out.push_units(name.len() / 2, bytes_to_units(&name), meta, row.get(0)?);
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
    ) -> rusqlite::Result<i64> {
        let (file_id, attrs, tag, size, write, change) = to_row(meta);
        if let Some(id) = existing {
            self.conn
                .prepare_cached(
                    "UPDATE entry SET file_id = ?2, attrs = ?3, tag = ?4, size = ?5, \
                 write_time = ?6, change_time = ?7 WHERE id = ?1",
                )?
                .execute(params![id, file_id, attrs, tag, size, write, change])?;
            return Ok(id);
        }
        let mut statement = self.conn.prepare_cached(
            "INSERT INTO entry (root, parent, name, file_id, attrs, tag, size, write_time, \
             change_time) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) \
             ON CONFLICT (root, parent, name) DO UPDATE SET file_id = excluded.file_id, \
             attrs = excluded.attrs, tag = excluded.tag, size = excluded.size, \
             write_time = excluded.write_time, change_time = excluded.change_time \
             RETURNING id",
        )?;
        statement.query_row(
            params![
                i64::from(root),
                parent,
                units_to_bytes(name),
                file_id,
                attrs,
                tag,
                size,
                write,
                change
            ],
            |row| row.get(0),
        )
    }

    fn remove(&mut self, _root: u16, id: i64) -> rusqlite::Result<()> {
        self.conn
            .prepare_cached("DELETE FROM entry WHERE id = ?1")?
            .execute([id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
