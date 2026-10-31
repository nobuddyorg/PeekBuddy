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
use std::sync::{Arc, Mutex};

use peekbuddy_core::event::{Event, NameKind, RootIdx};
use peekbuddy_core::name::WideStr;
use rusqlite::{Connection, OptionalExtension, params};

use crate::batch::Batch;
use crate::catalog::{Catalog, SegmentRow, Skipped};
use crate::ids;
use crate::interner::{Interner, NodeKey, TOP_LEVEL};
use crate::names::to_columns;
use crate::retention::{Caps, Retire};
use crate::schema::{self, INSERT_EVENT, SCHEMA_VERSION, StoreError, UPSERT_STATS, hour_bucket};
use crate::segment::{self, Rotation, needs_rotation, next_file, parse_name};
use crate::segments::{SealContext, SegmentState, SystemRows, rotated_row, seal_and_enforce};

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

/// What a row without a watch ([`RootIdx::NONE`]) is stored under: no root,
/// no volume, names at the top level.
const NO_ROOT: RootRow = RootRow {
    root: 0,
    vol: 0,
    node: TOP_LEVEL,
};

/// The connection, the interner and the id counter.
#[derive(Debug)]
pub struct Writer {
    conn: Connection,
    interner: Interner,
    next_id: i64,
    roots: Vec<Option<RootRow>>,
    stats: HashMap<(i64, i64, i64), (i64, i64, i64)>,
    path: Option<PathBuf>,
    /// The roots as registered, to register them again in the next segment.
    registered: Vec<RootInfo>,
    /// Day segments, when the writer rotates (#54).
    segments: Option<SegmentState>,
    /// The timestamps of the first and last event of the last commit.
    first_commit_ts: i64,
    last_commit_ts: i64,
}

/// What a rotating writer needs (#54).
pub struct SegmentsConfig {
    /// Directory, day function and size cap.
    pub rotation: Rotation,
    /// The retention caps.
    pub caps: Caps,
    /// The viewer's side of the retirement handshake.
    pub retire: Arc<dyn Retire + Send + Sync>,
}

/// SQLite's default and largest page count.
const MAX_PAGES: u32 = 4_294_967_294;

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
            registered: Vec::new(),
            segments: None,
            first_commit_ts: 0,
            last_commit_ts: 0,
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
        self.registered.retain(|known| known.idx != info.idx);
        self.registered.push(info.clone());
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
        self.rotate_if_needed(batches);
        let result = self.write_inner(batches, cursors, spill);
        if let (Ok(commit), Some(seg)) = (&result, self.segments.as_mut())
            && commit.rows > 0
        {
            seg.last_ts = seg.last_ts.max(self.last_commit_ts);
            if seg.first_ts == 0 {
                seg.first_ts = self.first_commit_ts;
            }
        }
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
        let extra = self.take_system_rows();
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
        let mut first_ts = 0u64;
        {
            let mut insert = tx.prepare_cached(INSERT_EVENT)?;
            for batch in extra.iter().chain(batches) {
                for (event, name) in batch.iter() {
                    let root = if event.root == RootIdx::NONE {
                        NO_ROOT
                    } else {
                        roots
                            .get(usize::from(event.root.0))
                            .copied()
                            .flatten()
                            .ok_or(WriteError::UnknownRoot(event.root.0))?
                    };
                    let node_id = intern_path(interner, &tx, root, event, name)?;
                    insert_event(&mut insert, id, event, root, node_id)?;
                    let kind = kind_column(event);
                    let entry = stats
                        .entry((hour_bucket(event.ts.cast_signed()), root.root, kind))
                        .or_insert((0, id, id));
                    entry.0 += 1;
                    entry.2 = id;
                    if first_ts == 0 {
                        first_ts = event.ts;
                    }
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
        self.first_commit_ts = first_ts.cast_signed();
        self.last_commit_ts = last_ts.cast_signed();
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

    /// Caps the database at `pages` pages, so a write that needs more fails
    /// with `SQLITE_FULL` like a full volume; `None` lifts the cap. For fault
    /// injection in tests; the limit applies to this connection only.
    ///
    /// # Errors
    ///
    /// SQLite's error.
    pub fn limit_pages(&self, pages: Option<u32>) -> Result<(), WriteError> {
        let pages = pages.unwrap_or(MAX_PAGES);
        let _applied: i64 =
            self.conn
                .query_row(&format!("PRAGMA max_page_count = {pages}"), [], |row| {
                    row.get(0)
                })?;
        Ok(())
    }

    /// The number of pages the database has now.
    ///
    /// # Errors
    ///
    /// SQLite's error.
    pub fn page_count(&self) -> Result<u32, WriteError> {
        Ok(self
            .conn
            .query_row("PRAGMA page_count", [], |row| row.get(0))?)
    }

    /// Makes every write fail with `SQLITE_READONLY` (or lifts that); the
    /// second kind of write error besides a full disk, for fault injection.
    ///
    /// # Errors
    ///
    /// SQLite's error.
    pub fn set_query_only(&self, on: bool) -> Result<(), WriteError> {
        self.conn
            .execute_batch(if on {
                "PRAGMA query_only = ON"
            } else {
                "PRAGMA query_only = OFF"
            })
            .map_err(WriteError::from)
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

    /// Opens the day segments under `config.rotation.dir`: brings the catalog
    /// in line with the files, continues the id sequence from the newest
    /// segment (or creates the first one for the day of `now`, FILETIME
    /// ticks), and applies the retention caps once. Files that look like
    /// segments but cannot be used are left alone and returned.
    ///
    /// # Errors
    ///
    /// A [`WriteError`] when the directory, the catalog or the segment cannot
    /// be opened.
    pub fn open_segments(
        config: SegmentsConfig,
        now: u64,
    ) -> Result<(Self, Vec<Skipped>), WriteError> {
        let SegmentsConfig {
            rotation,
            caps,
            retire,
        } = config;
        let io = |error: std::io::Error| WriteError::Store(StoreError::Io(error.to_string()));
        std::fs::create_dir_all(&rotation.dir).map_err(io)?;
        let catalog = Catalog::open(&rotation.dir.join("catalog.db"))
            .map_err(|error| WriteError::Store(StoreError::Io(error.to_string())))?;
        let (rows, skipped) = segment::reconcile_catalog(&rotation.dir, &catalog)
            .map_err(|error| WriteError::Store(StoreError::Io(error.to_string())))?;
        let today = (rotation.day_of)(now);
        let current = rows.last().cloned();
        let (file, existing) = match current {
            Some(row) => (row.file.clone(), Some(row)),
            None => (today.file_name(0), None),
        };
        let mut writer = Self::open(&rotation.dir.join(&file))?;
        let next_id = catalog.next_id().map_err(WriteError::Sqlite)?;
        writer.set_next_id(next_id);
        let day = parse_name(&file).map_or(today, |(day, _)| day);
        let first_id = existing
            .as_ref()
            .filter(|row| row.rows > 0)
            .map_or(writer.next_id(), |row| row.first_id);
        let (first_ts, last_ts) = existing
            .as_ref()
            .map_or((0, 0), |row| (row.first_ts, row.last_ts));
        catalog
            .put(&SegmentRow {
                file: file.clone(),
                first_id,
                last_id: writer.next_id() - 1,
                first_ts,
                last_ts,
                rows: writer.next_id() - first_id,
                bytes: existing.as_ref().map_or(0, |row| row.bytes),
                sealed: false,
                schema_version: i64::from(SCHEMA_VERSION),
                purge_pending: false,
            })
            .map_err(WriteError::Sqlite)?;
        let system: SystemRows = Arc::default();
        let context = SealContext {
            dir: rotation.dir.clone(),
            catalog: Arc::new(Mutex::new(catalog)),
            caps,
            retire,
            system,
        };
        let _ = context.enforce(now.cast_signed());
        writer.segments = Some(SegmentState {
            rotation,
            context,
            file,
            day,
            first_id,
            first_ts,
            last_ts,
            sealers: Vec::new(),
        });
        Ok((writer, skipped))
    }

    /// The name of the segment file being written, when the writer rotates.
    #[must_use]
    pub fn segment_file(&self) -> Option<&str> {
        self.segments.as_ref().map(|seg| seg.file.as_str())
    }

    /// The catalog of a rotating writer.
    #[must_use]
    pub fn catalog(&self) -> Option<Arc<Mutex<Catalog>>> {
        self.segments
            .as_ref()
            .map(|seg| Arc::clone(&seg.context.catalog))
    }

    /// Writes the system rows that are waiting (retention purges whose
    /// sealing thread finished after the last commit).
    ///
    /// # Errors
    ///
    /// [`WriteError`].
    pub fn flush_system_rows(&mut self) -> Result<(), WriteError> {
        let waiting = self
            .segments
            .as_ref()
            .is_some_and(|seg| seg.context.system.lock().is_ok_and(|rows| !rows.is_empty()));
        if waiting {
            self.write(&[], &[])?;
        }
        Ok(())
    }

    /// Waits for the sealing threads of earlier rotations.
    pub fn wait_background(&mut self) {
        if let Some(seg) = self.segments.as_mut() {
            for thread in seg.sealers.drain(..) {
                let _ = thread.join();
            }
        }
    }

    /// The rows rotation and retention queued, as a batch for the next commit.
    fn take_system_rows(&self) -> Option<Batch> {
        let seg = self.segments.as_ref()?;
        let rows = std::mem::take(&mut *seg.context.system.lock().ok()?);
        if rows.is_empty() {
            return None;
        }
        let mut batch = Batch::with_capacity(rows.len(), 64 * rows.len() + 256);
        for row in rows {
            let _ = batch.push(row.event, &row.name);
        }
        Some(batch)
    }

    /// Rotates before a write whose first event belongs to a later day, or
    /// when the file is over the size cap. A failure keeps writing to the
    /// current segment: losing events is worse than a big file.
    fn rotate_if_needed(&mut self, batches: &[Batch]) {
        let Some(ts) = batches
            .iter()
            .find_map(|batch| batch.iter().next().map(|(event, _)| event.ts))
        else {
            return;
        };
        let Some(seg) = self.segments.as_ref() else {
            return;
        };
        let day = (seg.rotation.day_of)(ts);
        let size = self.path.as_ref().map_or(0, |path| {
            let main = std::fs::metadata(path).map_or(0, |meta| meta.len());
            main + self.wal_bytes()
        });
        // An empty file is rotated for a new day (it is dropped) but never
        // for its size: there is nothing to seal.
        let empty = self.next_id == seg.first_id;
        let size = if empty { 0 } else { size };
        if needs_rotation(seg.day, day, size, seg.rotation.size_cap) {
            let _ = self.rotate(ts, day);
        }
    }

    /// Closes the current segment and continues in the next one.
    fn rotate(&mut self, ts: u64, day: segment::DayKey) -> Result<(), WriteError> {
        let Some(mut seg) = self.segments.take() else {
            return Ok(());
        };
        let outcome = self.rotate_inner(&mut seg, ts, day);
        self.segments = Some(seg);
        outcome
    }

    fn rotate_inner(
        &mut self,
        seg: &mut SegmentState,
        ts: u64,
        day: segment::DayKey,
    ) -> Result<(), WriteError> {
        let dir = seg.rotation.dir.clone();
        let _ = self.checkpoint_truncate();
        let empty = self.next_id == seg.first_id;
        let old_path = dir.join(&seg.file);
        let old_bytes =
            std::fs::metadata(&old_path).map_or(0, |meta| i64::try_from(meta.len()).unwrap_or(0));
        let old_row = SegmentRow {
            file: seg.file.clone(),
            first_id: seg.first_id,
            last_id: self.next_id - 1,
            first_ts: seg.first_ts,
            last_ts: seg.last_ts,
            rows: self.next_id - seg.first_id,
            bytes: old_bytes,
            sealed: false,
            schema_version: i64::from(SCHEMA_VERSION),
            purge_pending: false,
        };
        let catalog_rows = {
            let catalog = seg.context.catalog.lock().map_err(|_| poisoned())?;
            catalog.list().map_err(WriteError::Sqlite)?
        };
        let new_file = next_file(&seg.file, day, &catalog_rows);
        let new_path = dir.join(&new_file);
        let new_conn = schema::open(&new_path)?;
        pragmas(&new_conn)?;
        let carried = carry_over(&self.conn)?;
        let old_roots = self.roots.clone();
        // Swap: from here on the writer is in the new file.
        drop(std::mem::replace(&mut self.conn, new_conn));
        self.interner = Interner::new();
        self.stats.clear();
        self.roots.clear();
        for info in std::mem::take(&mut self.registered) {
            self.register_root(&info)?;
        }
        restore_carried(&self.conn, &carried, &old_roots, &self.roots)?;
        let catalog = seg.context.catalog.lock().map_err(|_| poisoned())?;
        if empty {
            drop(std::fs::remove_file(&old_path));
            for suffix in ["-wal", "-shm"] {
                drop(std::fs::remove_file(
                    dir.join(format!("{}{suffix}", seg.file)),
                ));
            }
            catalog.remove(&seg.file).map_err(WriteError::Sqlite)?;
        } else {
            catalog.put(&old_row).map_err(WriteError::Sqlite)?;
        }
        catalog
            .put(&SegmentRow {
                file: new_file.clone(),
                first_id: self.next_id,
                last_id: self.next_id - 1,
                first_ts: 0,
                last_ts: 0,
                rows: 0,
                bytes: 0,
                sealed: false,
                schema_version: i64::from(SCHEMA_VERSION),
                purge_pending: false,
            })
            .map_err(WriteError::Sqlite)?;
        drop(catalog);
        if !empty {
            if let Ok(mut rows) = seg.context.system.lock() {
                rows.push(rotated_row(ts, &old_row, &new_file));
            }
            let context = seg.context.clone();
            let sealed_file = seg.file.clone();
            if let Ok(thread) = std::thread::Builder::new()
                .name(String::from("seal"))
                .spawn(move || seal_and_enforce(&context, &sealed_file, ts.cast_signed()))
            {
                seg.sealers.push(thread);
            }
        }
        seg.file = new_file;
        seg.day = day;
        seg.first_id = self.next_id;
        seg.first_ts = 0;
        seg.last_ts = 0;
        self.path = Some(new_path);
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

fn poisoned() -> WriteError {
    WriteError::Store(StoreError::Io(String::from("the catalog lock is poisoned")))
}

/// What a new segment inherits from the one before: the replay position of
/// the spill file and the capture cursors.
struct Carried {
    meta: Vec<(String, i64)>,
    cursors: Vec<(i64, i64, Option<i64>, i64, i64)>,
}

fn carry_over(conn: &Connection) -> Result<Carried, WriteError> {
    let mut meta = Vec::new();
    for key in ["spill_gen", "spill_offset"] {
        let value: Option<i64> = conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()?;
        if let Some(value) = value {
            meta.push((key.to_owned(), value));
        }
    }
    let mut statement =
        conn.prepare("SELECT root_id, vol_id, usn, last_event_id, ts FROM cursor")?;
    let cursors = statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Carried { meta, cursors })
}

fn restore_carried(
    conn: &Connection,
    carried: &Carried,
    old_roots: &[Option<RootRow>],
    new_roots: &[Option<RootRow>],
) -> Result<(), WriteError> {
    for (key, value) in &carried.meta {
        conn.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2) \
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
    }
    for &(root_id, vol_id, usn, last_event_id, ts) in &carried.cursors {
        // The ids differ between files: go through the root's index.
        let Some(index) = old_roots
            .iter()
            .position(|row| row.is_some_and(|r| r.root == root_id && r.vol == vol_id))
        else {
            continue;
        };
        let Some(Some(new)) = new_roots.get(index) else {
            continue;
        };
        conn.execute(
            "INSERT INTO cursor (root_id, vol_id, usn, last_event_id, ts) \
             VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT (root_id, vol_id) DO NOTHING",
            params![new.root, new.vol, usn, last_event_id, ts],
        )?;
    }
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
    use crate::schema::{self, StoreError};

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
    fn errors_display_their_cause() {
        assert_eq!(
            WriteError::UnknownRoot(7).to_string(),
            "event for unregistered root 7"
        );
        let sqlite = WriteError::Sqlite(rusqlite::Error::QueryReturnedNoRows);
        assert!(sqlite.to_string().starts_with("SQLite error: "));
        let store = WriteError::Store(StoreError::Io(String::from("disk gone")));
        assert!(store.to_string().contains("disk gone"));
    }

    #[test]
    fn the_id_sequence_only_moves_forward() {
        let mut writer = memory_writer();
        writer.set_next_id(100);
        assert_eq!(writer.next_id(), 100);
        writer.set_next_id(5);
        assert_eq!(writer.next_id(), 100);
    }

    #[test]
    fn the_page_count_is_the_databases() {
        let writer = memory_writer();
        let pragma: u32 = writer
            .connection()
            .query_row("PRAGMA page_count", [], |row| row.get(0))
            .unwrap();
        assert!(pragma > 1);
        assert_eq!(writer.page_count().unwrap(), pragma);
    }

    #[test]
    fn the_kind_column_is_tag_times_256_plus_payload() {
        let mut writer = memory_writer();
        let mut overflow = event(1, 0);
        overflow.kind = peekbuddy_core::event::Kind::from_wire(9, 2).unwrap();
        let mut batch = Batch::with_capacity(2, 64);
        batch.push(overflow, &units("a.txt")).unwrap();
        writer.write(&[batch], &[]).unwrap();
        assert_eq!(count(&writer, "SELECT kind FROM event"), 9 * 256 + 2);
    }

    #[test]
    fn only_the_last_component_carries_the_file_id() {
        let mut writer = memory_writer();
        let mut with_id = Batch::with_capacity(4, 256);
        let mut one = event(1, 0);
        one.file_id = 0x0102_0304;
        with_id.push(one, &units(r"dir\inner.txt")).unwrap();
        let mut none = event(2, 0);
        none.file_id = 0;
        with_id.push(none, &units(r"dir\other.txt")).unwrap();
        writer.write(&[with_id], &[]).unwrap();
        assert_eq!(
            count(
                &writer,
                "SELECT count(*) FROM node WHERE name = 'dir' AND file_id IS NULL"
            ),
            1
        );
        assert_eq!(
            count(
                &writer,
                "SELECT count(*) FROM node WHERE name = 'inner.txt' AND file_id IS NOT NULL"
            ),
            1
        );
        assert_eq!(
            count(
                &writer,
                "SELECT count(*) FROM node WHERE name = 'other.txt' AND file_id IS NULL"
            ),
            1
        );
    }

    #[test]
    fn a_row_without_a_watch_is_stored_without_a_registered_root() {
        let mut conn = Connection::open_in_memory().unwrap();
        schema::migrate(&mut conn).unwrap();
        let mut writer = Writer::from_connection(conn).unwrap();
        let mut batch = Batch::with_capacity(2, 256);
        let mut system = event(1, 0);
        system.root = RootIdx::NONE;
        system.source = peekbuddy_core::event::Source::System;
        batch.push(system, &units("crash-1.log")).unwrap();
        let commit = writer.write(&[batch], &[]).unwrap();
        assert_eq!(commit.rows, 1);
        assert_eq!(count(&writer, "SELECT root_id FROM event"), 0);
        // A row for a root that was never registered is still refused.
        let mut bad = Batch::with_capacity(2, 256);
        bad.push(event(2, 7), &units("x")).unwrap();
        assert!(matches!(
            writer.write(&[bad], &[]),
            Err(WriteError::UnknownRoot(7))
        ));
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
