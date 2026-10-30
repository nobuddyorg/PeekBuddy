//! The segment catalog (#54): which day files exist and which ids they hold.
//!
//! `catalog.db` is small and *rebuildable*: every fact in it can be read
//! back from the segment files themselves ([`scan`]), so a lost or corrupt
//! catalog costs a scan at start, never data. The id space is one sequence
//! across all segments (R2): a segment's range is `first_id..=last_id`, the
//! next segment starts at `last_id + 1`, and retention only removes whole
//! oldest segments.

use std::fmt;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};

use crate::schema::{APPLICATION_ID, SCHEMA_VERSION};

const CATALOG_VERSION: i64 = 1;

const DDL: &str = "\
CREATE TABLE IF NOT EXISTS segment (
    file TEXT PRIMARY KEY,
    first_id INTEGER NOT NULL,
    last_id INTEGER NOT NULL,
    first_ts INTEGER NOT NULL,
    last_ts INTEGER NOT NULL,
    rows INTEGER NOT NULL,
    bytes INTEGER NOT NULL,
    sealed INTEGER NOT NULL,
    schema_version INTEGER NOT NULL,
    purge_pending INTEGER NOT NULL DEFAULT 0
) STRICT;";

/// One segment as the catalog knows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentRow {
    /// The file name, without the directory.
    pub file: String,
    /// The first event id, or the id the first event will get when empty.
    pub first_id: i64,
    /// The last event id; `first_id - 1` while the segment is empty.
    pub last_id: i64,
    /// Timestamp (FILETIME ticks) of the first event; 0 when empty.
    pub first_ts: i64,
    /// Timestamp of the last event; 0 when empty.
    pub last_ts: i64,
    /// Event rows.
    pub rows: i64,
    /// Size on disk in bytes.
    pub bytes: i64,
    /// No more writes will happen.
    pub sealed: bool,
    /// The schema version of the file.
    pub schema_version: i64,
    /// Retention wants it gone but the delete was deferred.
    pub purge_pending: bool,
}

/// Why the catalog could not be used.
#[derive(Debug)]
pub enum CatalogError {
    /// SQLite failed.
    Sqlite(rusqlite::Error),
    /// The catalog is from a newer build.
    TooNew(i64),
}

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(error) => write!(f, "SQLite error: {error}"),
            Self::TooNew(version) => write!(
                f,
                "catalog.db is version {version}; this build reads {CATALOG_VERSION}"
            ),
        }
    }
}

impl std::error::Error for CatalogError {}

impl From<rusqlite::Error> for CatalogError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

/// The catalog database.
#[derive(Debug)]
pub struct Catalog {
    conn: Connection,
}

impl Catalog {
    /// Opens (creating) `catalog.db` at `path`.
    ///
    /// # Errors
    ///
    /// SQLite's error, or [`CatalogError::TooNew`].
    pub fn open(path: &Path) -> Result<Self, CatalogError> {
        Self::from_connection(Connection::open(path)?)
    }

    /// A catalog in memory, for tests.
    ///
    /// # Errors
    ///
    /// SQLite's error.
    pub fn open_in_memory() -> Result<Self, CatalogError> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(conn: Connection) -> Result<Self, CatalogError> {
        let _mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
        conn.execute_batch("PRAGMA synchronous = NORMAL; PRAGMA busy_timeout = 5000;")?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version > CATALOG_VERSION {
            return Err(CatalogError::TooNew(version));
        }
        conn.execute_batch(DDL)?;
        conn.execute_batch(&format!("PRAGMA user_version = {CATALOG_VERSION};"))?;
        Ok(Self { conn })
    }

    /// Inserts or replaces a segment.
    ///
    /// # Errors
    ///
    /// SQLite's error.
    pub fn put(&self, row: &SegmentRow) -> rusqlite::Result<()> {
        self.conn
            .prepare_cached(
                "INSERT INTO segment (file, first_id, last_id, first_ts, last_ts, rows, bytes, \
                 sealed, schema_version, purge_pending) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, \
                 ?9, ?10) ON CONFLICT (file) DO UPDATE SET first_id = excluded.first_id, \
                 last_id = excluded.last_id, first_ts = excluded.first_ts, \
                 last_ts = excluded.last_ts, rows = excluded.rows, bytes = excluded.bytes, \
                 sealed = excluded.sealed, schema_version = excluded.schema_version, \
                 purge_pending = excluded.purge_pending",
            )?
            .execute(params![
                row.file,
                row.first_id,
                row.last_id,
                row.first_ts,
                row.last_ts,
                row.rows,
                row.bytes,
                i64::from(row.sealed),
                row.schema_version,
                i64::from(row.purge_pending),
            ])?;
        Ok(())
    }

    /// Every segment, oldest first (by id).
    ///
    /// # Errors
    ///
    /// SQLite's error.
    pub fn list(&self) -> rusqlite::Result<Vec<SegmentRow>> {
        let mut statement = self.conn.prepare_cached(
            "SELECT file, first_id, last_id, first_ts, last_ts, rows, bytes, sealed, \
             schema_version, purge_pending FROM segment ORDER BY first_id, file",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(SegmentRow {
                file: row.get(0)?,
                first_id: row.get(1)?,
                last_id: row.get(2)?,
                first_ts: row.get(3)?,
                last_ts: row.get(4)?,
                rows: row.get(5)?,
                bytes: row.get(6)?,
                sealed: row.get::<_, i64>(7)? != 0,
                schema_version: row.get(8)?,
                purge_pending: row.get::<_, i64>(9)? != 0,
            })
        })?;
        rows.collect()
    }

    /// Every segment, newest first: the order a scroll back in time reads
    /// them.
    ///
    /// # Errors
    ///
    /// SQLite's error.
    pub fn newest_first(&self) -> rusqlite::Result<Vec<SegmentRow>> {
        let mut all = self.list()?;
        all.reverse();
        Ok(all)
    }

    /// Marks a segment as sealed.
    ///
    /// # Errors
    ///
    /// SQLite's error.
    pub fn seal(&self, file: &str, bytes: i64) -> rusqlite::Result<()> {
        self.conn
            .prepare_cached("UPDATE segment SET sealed = 1, bytes = ?2 WHERE file = ?1")?
            .execute(params![file, bytes])?;
        Ok(())
    }

    /// Marks a segment whose delete was deferred.
    ///
    /// # Errors
    ///
    /// SQLite's error.
    pub fn defer_purge(&self, file: &str) -> rusqlite::Result<()> {
        self.conn
            .prepare_cached("UPDATE segment SET purge_pending = 1 WHERE file = ?1")?
            .execute([file])?;
        Ok(())
    }

    /// Forgets a segment.
    ///
    /// # Errors
    ///
    /// SQLite's error.
    pub fn remove(&self, file: &str) -> rusqlite::Result<()> {
        self.conn
            .prepare_cached("DELETE FROM segment WHERE file = ?1")?
            .execute([file])?;
        Ok(())
    }

    /// One segment by file name.
    ///
    /// # Errors
    ///
    /// SQLite's error.
    pub fn get(&self, file: &str) -> rusqlite::Result<Option<SegmentRow>> {
        Ok(self.list()?.into_iter().find(|row| row.file == file))
    }

    /// The id after the newest segment's last id (1 for an empty catalog).
    ///
    /// # Errors
    ///
    /// SQLite's error.
    pub fn next_id(&self) -> rusqlite::Result<i64> {
        let last: Option<i64> = self
            .conn
            .query_row("SELECT max(last_id) FROM segment", [], |row| row.get(0))
            .optional()?
            .flatten();
        Ok(last.map_or(1, |id| id + 1))
    }
}

/// A file in the data directory that was left out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    /// The file name.
    pub file: String,
    /// Why: not a database, foreign `application_id`, newer schema, ...
    pub reason: String,
}

/// What [`scan`] found.
#[derive(Debug, Default)]
pub struct Scan {
    /// The segments that are usable, oldest first.
    pub segments: Vec<SegmentRow>,
    /// The files that were skipped, each with a reason (a health event).
    pub skipped: Vec<Skipped>,
}

/// Whether `name` looks like a segment file (`events-YYYY-MM-DD.db` or
/// `events-YYYY-MM-DD.N.db`).
#[must_use]
pub fn is_segment_name(name: &str) -> bool {
    let Some(rest) = name
        .strip_prefix("events-")
        .and_then(|r| r.strip_suffix(".db"))
    else {
        return false;
    };
    let (date, extra) = rest
        .split_once('.')
        .map_or((rest, None), |(d, n)| (d, Some(n)));
    let date_ok = date.len() == 10
        && date.bytes().enumerate().all(|(i, b)| match i {
            4 | 7 => b == b'-',
            _ => b.is_ascii_digit(),
        });
    date_ok && extra.is_none_or(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Reads a segment file read-only and describes it, or says why not.
///
/// # Errors
///
/// The reason the file is not a usable segment.
pub fn describe(path: &Path) -> Result<SegmentRow, String> {
    let file = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| String::from("not a UTF-8 file name"))?
        .to_owned();
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| format!("cannot be opened: {error}"))?;
    let application_id: i64 = conn
        .query_row("PRAGMA application_id", [], |row| row.get(0))
        .map_err(|error| format!("not a database: {error}"))?;
    if application_id != i64::from(APPLICATION_ID) {
        return Err(format!("foreign application_id {application_id:#x}"));
    }
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|error| format!("unreadable: {error}"))?;
    if version > i64::from(SCHEMA_VERSION) {
        return Err(format!("schema version {version} is newer than this build"));
    }
    let (rows, first_id, last_id, first_ts, last_ts): (i64, Option<i64>, Option<i64>, i64, i64) =
        conn.query_row(
            "SELECT count(*), min(id), max(id), coalesce(min(ts), 0), coalesce(max(ts), 0) \
             FROM event",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .map_err(|error| format!("damaged: {error}"))?;
    let bytes = std::fs::metadata(path).map_or(0, |meta| i64::try_from(meta.len()).unwrap_or(0));
    Ok(SegmentRow {
        file,
        first_id: first_id.unwrap_or(0),
        last_id: last_id.unwrap_or(0),
        first_ts,
        last_ts,
        rows,
        bytes,
        sealed: false,
        schema_version: version,
        purge_pending: false,
    })
}

/// Reads every segment file of `dir`. Files that are not segments are left
/// alone; files that look like segments but cannot be used are reported in
/// [`Scan::skipped`] and never touched.
///
/// # Errors
///
/// The error of reading the directory.
pub fn scan(dir: &Path) -> std::io::Result<Scan> {
    let mut out = Scan::default();
    let mut names: Vec<PathBuf> = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(is_segment_name)
        {
            names.push(path);
        }
    }
    names.sort();
    for path in names {
        match describe(&path) {
            Ok(row) => out.segments.push(row),
            Err(reason) => out.skipped.push(Skipped {
                file: path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                reason,
            }),
        }
    }
    out.segments
        .sort_by_key(|row| (row.first_id, row.file.clone()));
    Ok(out)
}

#[cfg(test)]
mod tests;
