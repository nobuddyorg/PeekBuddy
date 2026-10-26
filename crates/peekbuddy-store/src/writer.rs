//! The database half of the writer (#52): one transaction per batch group.
//!
//! [`Writer`] owns the live connection and does no threading and no timing;
//! the thread around it ([`crate::pipeline`]) decides when to commit. One row
//! per raw record (F3): nothing is coalesced or de-duplicated here. The event
//! ids are one contiguous sequence assigned inside the commit transaction, and
//! the capture cursors are written in the same transaction as the rows they
//! cover, so a cursor can never be ahead of the committed rows.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

use peekbuddy_core::event::{Event, NameKind, RootIdx};
use peekbuddy_core::name::WideStr;
use rusqlite::{Connection, OptionalExtension, params};

use crate::batch::Batch;
use crate::ids;
use crate::interner::{Interner, NodeKey, TOP_LEVEL};
use crate::names::to_columns;
use crate::schema::{self, INSERT_EVENT, StoreError, UPSERT_STATS, hour_bucket};

/// Cache size in KiB (negative pragma values are KiB): 2 MB, spike #35.
const CACHE_KIB: i64 = 2 * 1024;
/// `journal_size_limit`: the WAL file is truncated to this after a checkpoint.
const JOURNAL_LIMIT: i64 = 64 * 1024 * 1024;
/// A size-triggered `PASSIVE` checkpoint runs once the WAL is this large.
pub const WAL_CHECKPOINT_BYTES: u64 = 64 * 1024 * 1024;

const BACKSLASH: u16 = 0x5C;

/// Why a write failed.
#[derive(Debug)]
pub enum WriteError {
    /// SQLite failed; the transaction was rolled back and nothing was written.
    Sqlite(rusqlite::Error),
    /// The schema layer failed.
    Store(StoreError),
    /// An event names a root that was never registered.
    UnknownRoot(u16),
}

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(error) => write!(f, "SQLite error: {error}"),
            Self::Store(error) => write!(f, "{error}"),
            Self::UnknownRoot(root) => write!(f, "event for unregistered root {root}"),
        }
    }
}

impl std::error::Error for WriteError {}

impl From<rusqlite::Error> for WriteError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl From<StoreError> for WriteError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

/// A volume as the `vol` table stores it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolInfo {
    /// The volume GUID path.
    pub guid: String,
    /// The volume serial number.
    pub serial: i64,
    /// The drive letter, when it has one.
    pub letter: Option<String>,
    /// The USN journal id, when known.
    pub journal_id: Option<i64>,
}

/// A watch root to register before its events arrive.
#[derive(Debug, Clone)]
pub struct RootInfo {
    /// The index events carry in [`Event::root`].
    pub idx: RootIdx,
    /// Its volume.
    pub vol: VolInfo,
    /// The absolute path (a leading `\\?\` is ignored).
    pub path: Vec<u16>,
    /// Whether the whole subtree is watched.
    pub recursive: bool,
}

/// A capture checkpoint written with the batch it covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    /// The root.
    pub root: RootIdx,
    /// The USN position, when the source has one.
    pub usn: Option<i64>,
}

/// What one commit wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Commit {
    /// Rows inserted.
    pub rows: u64,
    /// First id used (meaningless when `rows` is 0).
    pub first_id: i64,
    /// Last id used (`first_id - 1` when `rows` is 0).
    pub last_id: i64,
}

#[derive(Debug, Clone, Copy)]
struct RootRow {
    root: i64,
    vol: i64,
    node: i64,
}

/// The connection, the interner and the id counter.
#[derive(Debug)]
pub struct Writer {
    conn: Connection,
    interner: Interner,
    next_id: i64,
    roots: Vec<Option<RootRow>>,
    stats: HashMap<(i64, i64, i64), (i64, i64, i64)>,
    path: Option<PathBuf>,
}

fn pragmas(conn: &Connection) -> rusqlite::Result<()> {
    // journal_mode answers with a row, so it goes through a query.
    let _mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
    conn.execute_batch(&format!(
        "PRAGMA synchronous = NORMAL;\n\
         PRAGMA foreign_keys = OFF;\n\
         PRAGMA temp_store = MEMORY;\n\
         PRAGMA busy_timeout = 5000;\n\
         PRAGMA mmap_size = 0;\n\
         PRAGMA wal_autocheckpoint = 0;\n\
         PRAGMA journal_size_limit = {JOURNAL_LIMIT};\n\
         PRAGMA cache_size = -{CACHE_KIB};"
    ))
}

impl Writer {
    /// Opens (creating and migrating) the segment at `path`.
    ///
    /// # Errors
    ///
    /// A [`WriteError`] when the file is not a segment, is too new or SQLite
    /// fails.
    pub fn open(path: &Path) -> Result<Self, WriteError> {
        let conn = schema::open(path)?;
        let mut writer = Self::from_connection(conn)?;
        writer.path = Some(path.to_path_buf());
        Ok(writer)
    }

    /// A writer on an already migrated connection (tests use memory).
    ///
    /// # Errors
    ///
    /// SQLite failures while setting the pragmas or reading the last id.
    pub fn from_connection(conn: Connection) -> Result<Self, WriteError> {
        pragmas(&conn)?;
        let next_id: i64 =
            conn.query_row("SELECT coalesce(max(id), 0) + 1 FROM event", [], |row| {
                row.get(0)
            })?;
        Ok(Self {
            conn,
            interner: Interner::new(),
            next_id,
            roots: Vec::new(),
            stats: HashMap::new(),
            path: None,
        })
    }

    /// The id the next row will get.
    #[must_use]
    pub const fn next_id(&self) -> i64 {
        self.next_id
    }

    /// Continues the global id sequence from another segment (#54).
    pub fn set_next_id(&mut self, next_id: i64) {
        self.next_id = next_id.max(self.next_id);
    }

    /// The connection, for reads in tests and for the checkpoint code.
    #[must_use]
    pub const fn connection(&self) -> &Connection {
        &self.conn
    }

    /// Registers a root: its volume, its `root` row and the nodes of its path.
    /// Registering the same root again returns the same rows.
    ///
    /// # Errors
    ///
    /// SQLite failures.
    pub fn register_root(&mut self, info: &RootInfo) -> Result<(), WriteError> {
        let tx = self.conn.transaction()?;
        let found_vol = tx
            .query_row(
                "SELECT id FROM vol WHERE guid = ?1",
                [&info.vol.guid],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        let vol_id = if let Some(id) = found_vol {
            id
        } else {
            tx.execute(
                "INSERT INTO vol (guid, serial, letter, journal_id) VALUES (?1, ?2, ?3, ?4)",
                params![
                    info.vol.guid,
                    info.vol.serial,
                    info.vol.letter,
                    info.vol.journal_id
                ],
            )?;
            tx.last_insert_rowid()
        };
        let columns = to_columns(WideStr::new(&info.path));
        let found_root = tx
            .query_row(
                "SELECT id FROM root WHERE path = ?1 AND vol_id = ?2",
                params![columns.text, vol_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        let root_id = if let Some(id) = found_root {
            id
        } else {
            tx.execute(
                "INSERT INTO root (path, path_w, recursive, vol_id) VALUES (?1, ?2, ?3, ?4)",
                params![
                    columns.text,
                    columns.wide,
                    i64::from(info.recursive),
                    vol_id
                ],
            )?;
            tx.last_insert_rowid()
        };
        let mut parent = TOP_LEVEL;
        for part in info.path.split(|unit| *unit == BACKSLASH) {
            if part.is_empty() || part == [0x3F] {
                continue;
            }
            parent = self.interner.intern(
                &tx,
                &NodeKey {
                    vol_id,
                    parent_id: parent,
                    name: WideStr::new(part),
                    name_kind: NameKind::Unknown,
                    file_id: None,
                },
            )?;
        }
        tx.commit()?;
        let slot = usize::from(info.idx.0);
        if self.roots.len() <= slot {
            self.roots.resize(slot + 1, None);
        }
        if let Some(entry) = self.roots.get_mut(slot) {
            *entry = Some(RootRow {
                root: root_id,
                vol: vol_id,
                node: parent,
            });
        }
        Ok(())
    }

    /// The spill position the database has replayed up to: generation and
    /// byte offset, `(0, 0)` before any replay.
    #[must_use]
    pub fn spill_position(&self) -> (u32, u64) {
        let read = |key: &str| -> i64 {
            self.conn
                .query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| {
                    row.get(0)
                })
                .unwrap_or(0)
        };
        (
            u32::try_from(read("spill_gen")).unwrap_or(0),
            u64::try_from(read("spill_offset")).unwrap_or(0),
        )
    }

    /// Writes `batches` and `cursors` in one transaction.
    ///
    /// On any error nothing is written, the interner is cleared (ids that
    /// never reached disk must not be handed out) and the id counter stays.
    ///
    /// # Errors
    ///
    /// [`WriteError`].
    pub fn write(&mut self, batches: &[Batch], cursors: &[Cursor]) -> Result<Commit, WriteError> {
        self.write_with_spill(batches, cursors, None)
    }

    /// Like [`Writer::write`]; with `spill` the replay position is committed
    /// in the same transaction as the rows it covers.
    ///
    /// # Errors
    ///
    /// [`WriteError`].
    pub fn write_with_spill(
        &mut self,
        batches: &[Batch],
        cursors: &[Cursor],
        spill: Option<(u32, u64)>,
    ) -> Result<Commit, WriteError> {
        let result = self.write_inner(batches, cursors, spill);
        if result.is_err() {
            self.interner.clear();
        }
        result
    }

    fn write_inner(
        &mut self,
        batches: &[Batch],
        cursors: &[Cursor],
        spill: Option<(u32, u64)>,
    ) -> Result<Commit, WriteError> {
        let Self {
            conn,
            interner,
            next_id,
            roots,
            stats,
            ..
        } = self;
        stats.clear();
        let tx = conn.transaction()?;
        let first = *next_id;
        let mut id = first;
        let mut last_ts = 0u64;
        {
            let mut insert = tx.prepare_cached(INSERT_EVENT)?;
            for batch in batches {
                for (event, name) in batch.iter() {
                    let root = roots
                        .get(usize::from(event.root.0))
                        .copied()
                        .flatten()
                        .ok_or(WriteError::UnknownRoot(event.root.0))?;
                    let node_id = intern_path(interner, &tx, root, event, name)?;
                    insert_event(&mut insert, id, event, root, node_id)?;
                    let kind = kind_column(event);
                    let entry = stats
                        .entry((hour_bucket(event.ts.cast_signed()), root.root, kind))
                        .or_insert((0, id, id));
                    entry.0 += 1;
                    entry.2 = id;
                    last_ts = event.ts;
                    id += 1;
                }
            }
        }
        for ((hour, root_id, kind), (count, min_id, max_id)) in stats.iter() {
            tx.prepare_cached(UPSERT_STATS)?
                .execute(params![hour, root_id, kind, count, min_id, max_id])?;
        }
        write_cursors(&tx, roots, cursors, id - 1, last_ts)?;
        if let Some((generation, offset)) = spill {
            for (key, value) in [
                ("spill_gen", i64::from(generation)),
                ("spill_offset", offset.cast_signed()),
            ] {
                tx.execute(
                    "INSERT INTO meta (key, value) VALUES (?1, ?2) \
                     ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                    params![key, value],
                )?;
            }
        }
        tx.commit()?;
        *next_id = id;
        let commit = Commit {
            rows: u64::try_from(id - first).unwrap_or(0),
            first_id: first,
            last_id: id - 1,
        };
        #[cfg(debug_assertions)]
        self.check_cursors()?;
        Ok(commit)
    }

    /// A cursor must never point past the last committed row.
    #[cfg(debug_assertions)]
    fn check_cursors(&self) -> rusqlite::Result<()> {
        let ahead: i64 = self.conn.query_row(
            "SELECT count(*) FROM cursor WHERE last_event_id > ?1",
            [self.next_id - 1],
            |row| row.get(0),
        )?;
        debug_assert_eq!(ahead, 0, "a capture cursor is ahead of the committed rows");
        Ok(())
    }

    /// Size of the WAL file in bytes; 0 for a memory database or none yet.
    #[must_use]
    pub fn wal_bytes(&self) -> u64 {
        self.path.as_ref().map_or(0, |path| {
            let mut wal = path.as_os_str().to_owned();
            wal.push("-wal");
            std::fs::metadata(wal).map_or(0, |meta| meta.len())
        })
    }

    /// A `PASSIVE` checkpoint: never blocks readers or the next commit.
    /// Returns `(frames in the log, frames checkpointed)`.
    ///
    /// # Errors
    ///
    /// SQLite failures.
    pub fn checkpoint_passive(&self) -> Result<(i64, i64), WriteError> {
        Ok(self
            .conn
            .query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |row| {
                Ok((row.get::<_, i64>(1)?, row.get::<_, i64>(2)?))
            })?)
    }

    /// Final checkpoint at shutdown: waits for readers, truncates the WAL.
    ///
    /// # Errors
    ///
    /// SQLite failures.
    pub fn checkpoint_truncate(&self) -> Result<(), WriteError> {
        let _: (i64, i64, i64) =
            self.conn
                .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })?;
        Ok(())
    }

    /// Distinct nodes held by the interner, for the probe.
    #[must_use]
    pub const fn interner_stats(&self) -> crate::interner::InternerStats {
        self.interner.stats()
    }
}

fn kind_column(event: &Event) -> i64 {
    let (tag, arg) = event.kind.to_wire();
    i64::from(tag) * 256 + i64::from(arg)
}

/// Interns the components of `name` under the root's node; the last one is the
/// event's own node. An empty name is the root itself.
fn intern_path(
    interner: &mut Interner,
    conn: &Connection,
    root: RootRow,
    event: &Event,
    name: &[u16],
) -> rusqlite::Result<i64> {
    let mut parent = root.node;
    let mut parts = name
        .split(|unit| *unit == BACKSLASH)
        .filter(|part| !part.is_empty())
        .peekable();
    while let Some(part) = parts.next() {
        let last = parts.peek().is_none();
        parent = interner.intern(
            conn,
            &NodeKey {
                vol_id: root.vol,
                parent_id: parent,
                name: WideStr::new(part),
                name_kind: if last {
                    event.name_kind
                } else {
                    NameKind::Unknown
                },
                file_id: (last && event.file_id != 0).then(|| ids::split(event.file_id).lo),
            },
        )?;
    }
    Ok(parent)
}

fn insert_event(
    insert: &mut rusqlite::CachedStatement<'_>,
    id: i64,
    event: &Event,
    root: RootRow,
    node_id: i64,
) -> rusqlite::Result<()> {
    let file = ids::split(event.file_id);
    insert.execute(params![
        id,
        event.ts.cast_signed(),
        kind_column(event),
        i64::from(event.source.tag()),
        i64::from(event.raw_mask),
        node_id,
        Option::<i64>::None,
        i64::from(event.is_dir),
        event.size.cast_signed(),
        i64::from(event.attrs),
        (event.reparse_tag != 0).then_some(i64::from(event.reparse_tag)),
        (event.file_id != 0).then_some(file.lo),
        file.hi,
        (event.usn != 0).then_some(event.usn),
        root.vol,
        root.root,
        i64::from(event.flags.bits()),
        Option::<i64>::None,
        Option::<i64>::None,
    ])?;
    Ok(())
}

fn write_cursors(
    tx: &rusqlite::Transaction<'_>,
    roots: &[Option<RootRow>],
    cursors: &[Cursor],
    last_id: i64,
    ts: u64,
) -> Result<(), WriteError> {
    for cursor in cursors {
        let root = roots
            .get(usize::from(cursor.root.0))
            .copied()
            .flatten()
            .ok_or(WriteError::UnknownRoot(cursor.root.0))?;
        tx.prepare_cached(
            "INSERT INTO cursor (root_id, vol_id, usn, last_event_id, ts) VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT (root_id, vol_id) DO UPDATE SET usn = excluded.usn, \
             last_event_id = max(last_event_id, excluded.last_event_id), ts = excluded.ts",
        )?
        .execute(params![root.root, root.vol, cursor.usn, last_id.max(0), ts.cast_signed()])?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use peekbuddy_core::event::RootIdx;
    use rusqlite::Connection;

    use super::{Cursor, RootInfo, VolInfo, WriteError, Writer};
    use crate::batch::Batch;
    use crate::batch::tests::{event, units};
    use crate::schema;

    pub(crate) fn memory_writer() -> Writer {
        let mut conn = Connection::open_in_memory().unwrap();
        schema::migrate(&mut conn).unwrap();
        let mut writer = Writer::from_connection(conn).unwrap();
        for (idx, path) in [(0u16, r"\\?\C:\Users\Me\Docs"), (1, r"\\?\C:\Work")] {
            writer
                .register_root(&RootInfo {
                    idx: RootIdx(idx),
                    vol: VolInfo {
                        guid: String::from(r"\\?\Volume{1}\"),
                        serial: 42,
                        letter: Some(String::from("C:")),
                        journal_id: None,
                    },
                    path: units(path),
                    recursive: true,
                })
                .unwrap();
        }
        writer
    }

    fn batch(first: u64, count: u64, root: u16) -> Batch {
        let mut batch = Batch::with_capacity(usize::try_from(count).unwrap() + 1, 8192);
        for ts in first..first + count {
            batch
                .push(event(ts, root), &units(&format!("sub\\file{ts}.txt")))
                .unwrap();
        }
        batch
    }

    fn count(writer: &Writer, sql: &str) -> i64 {
        writer
            .connection()
            .query_row(sql, [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn rows_get_contiguous_ids_and_interned_nodes() {
        let mut writer = memory_writer();
        let first = writer.write(&[batch(1, 3, 0)], &[]).unwrap();
        assert_eq!((first.rows, first.first_id, first.last_id), (3, 1, 3));
        let second = writer.write(&[batch(4, 2, 1)], &[]).unwrap();
        assert_eq!((second.rows, second.first_id, second.last_id), (2, 4, 5));
        assert_eq!(writer.next_id(), 6);
        assert_eq!(count(&writer, "SELECT count(*) FROM event"), 5);
        assert_eq!(
            count(
                &writer,
                "SELECT count(DISTINCT id) FROM event WHERE id BETWEEN 1 AND 5"
            ),
            5
        );
        // `sub` is one node per root, the files are one node each.
        assert_eq!(
            count(&writer, "SELECT count(*) FROM node WHERE name = 'sub'"),
            2
        );
        assert_eq!(
            count(&writer, "SELECT count(*) FROM node WHERE name LIKE 'file%'"),
            5
        );
        // The search index follows.
        assert_eq!(
            count(
                &writer,
                "SELECT count(*) FROM node_fts WHERE node_fts MATCH '\"file3\"'"
            ),
            1
        );
    }

    #[test]
    fn every_raw_record_is_a_row_nothing_is_merged() {
        let mut writer = memory_writer();
        let mut same = Batch::with_capacity(8, 256);
        for ts in 0..4 {
            same.push(event(100 + ts, 0), &units("a.txt")).unwrap();
        }
        writer.write(&[same], &[]).unwrap();
        assert_eq!(count(&writer, "SELECT count(*) FROM event"), 4);
        assert_eq!(
            count(&writer, "SELECT count(DISTINCT node_id) FROM event"),
            1
        );
    }

    #[test]
    fn stats_aggregate_per_hour_root_and_kind() {
        let mut writer = memory_writer();
        writer
            .write(&[batch(1, 3, 0), batch(4, 2, 1)], &[])
            .unwrap();
        assert_eq!(count(&writer, "SELECT sum(n) FROM stats"), 5);
        assert_eq!(count(&writer, "SELECT count(*) FROM stats"), 2);
        assert_eq!(count(&writer, "SELECT max(max_id) FROM stats"), 5);
        writer.write(&[batch(6, 1, 0)], &[]).unwrap();
        assert_eq!(count(&writer, "SELECT sum(n) FROM stats"), 6);
    }

    #[test]
    fn the_cursor_is_written_with_its_batch_and_never_ahead() {
        let mut writer = memory_writer();
        let cursor = [Cursor {
            root: RootIdx(0),
            usn: Some(500),
        }];
        writer.write(&[batch(1, 3, 0)], &cursor).unwrap();
        assert_eq!(count(&writer, "SELECT usn FROM cursor"), 500);
        assert_eq!(count(&writer, "SELECT last_event_id FROM cursor"), 3);
        // A cursor-only commit keeps the position and moves the usn.
        writer
            .write(
                &[],
                &[Cursor {
                    root: RootIdx(0),
                    usn: Some(900),
                }],
            )
            .unwrap();
        assert_eq!(count(&writer, "SELECT usn FROM cursor"), 900);
        assert_eq!(count(&writer, "SELECT last_event_id FROM cursor"), 3);
        assert_eq!(
            count(
                &writer,
                "SELECT count(*) FROM cursor WHERE last_event_id > (SELECT max(id) FROM event)"
            ),
            0
        );
    }

    #[test]
    fn a_failed_write_leaves_nothing_and_the_ids_unused() {
        let mut writer = memory_writer();
        let mut bad = batch(1, 2, 0);
        bad.push(event(9, 7), &units("x")).unwrap();
        let error = writer.write(&[bad], &[]).unwrap_err();
        assert!(matches!(error, WriteError::UnknownRoot(7)));
        assert_eq!(count(&writer, "SELECT count(*) FROM event"), 0);
        assert_eq!(
            count(&writer, "SELECT count(*) FROM node WHERE name LIKE 'file%'"),
            0
        );
        assert_eq!(writer.next_id(), 1);
        // The interner forgot the rolled-back nodes, so the retry works.
        let ok = writer.write(&[batch(1, 2, 0)], &[]).unwrap();
        assert_eq!((ok.first_id, ok.last_id), (1, 2));
        assert_eq!(
            count(&writer, "SELECT count(*) FROM node WHERE name LIKE 'file%'"),
            2
        );
    }

    #[test]
    fn registering_a_root_twice_gives_the_same_rows() {
        let mut writer = memory_writer();
        let before = count(&writer, "SELECT count(*) FROM root")
            + count(&writer, "SELECT count(*) FROM vol");
        writer
            .register_root(&RootInfo {
                idx: RootIdx(0),
                vol: VolInfo {
                    guid: String::from(r"\\?\Volume{1}\"),
                    serial: 42,
                    letter: None,
                    journal_id: Some(3),
                },
                path: units(r"\\?\C:\Users\Me\Docs"),
                recursive: true,
            })
            .unwrap();
        let after = count(&writer, "SELECT count(*) FROM root")
            + count(&writer, "SELECT count(*) FROM vol");
        assert_eq!(before, after);
    }

    #[test]
    fn the_root_itself_is_the_node_of_an_event_with_no_name() {
        let mut writer = memory_writer();
        let mut system = Batch::with_capacity(2, 8);
        system.push(event(1, 0), &[]).unwrap();
        writer.write(&[system], &[]).unwrap();
        let node: String = writer
            .connection()
            .query_row(
                "SELECT name FROM node WHERE id = (SELECT node_id FROM event)",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(node, "Docs");
    }
}
