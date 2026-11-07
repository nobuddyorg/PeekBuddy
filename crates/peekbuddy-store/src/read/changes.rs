//! "What changed since" over the segments (#82).
//!
//! [`Reader::changes_since`] streams the raw rows after an event id, across
//! day segments, through [`peekbuddy_core::changes::Fold`]. The dictionary is
//! per segment, so the fold is keyed by a 64-bit hash of the case-folded full
//! path, which is the same in every segment (a collision of two paths is
//! about 2^-64 per pair and would merge two rows of the diff, never lose
//! one). Memory is one fold slot and one exemplar node per distinct path, not
//! per raw row.
//!
//! The scan is one indexed pass per segment in id order and runs where the
//! caller runs it (a worker thread of the viewer): it reports progress and
//! stops at its cancel flag, returning what it has with
//! [`ChangeSet::cancelled`] set.

use std::collections::HashMap;

use peekbuddy_core::changes::{Fold, Summary, op_for};
use peekbuddy_core::event::Kind;
use peekbuddy_core::name::fold_upper;
use peekbuddy_core::query::CancelFlag;
use rusqlite::{Connection, OpenFlags};

use super::{PathCache, Paths, Reader, kind_of};
use crate::catalog::{Catalog, CatalogError};
use crate::ids::{FileIdColumns, join};

/// Rows scanned between two progress reports and cancel checks.
const STEP: u64 = 8_192;

/// Nodes whose path hash one segment pass remembers.
const KEY_CACHE_NODES: usize = 262_144;

/// A path in the diff: the hash of its case-folded full path.
pub type PathKey = u64;

/// How far a scan is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    /// Rows read so far.
    pub scanned: u64,
    /// Segments finished.
    pub segments_done: usize,
    /// Segments to read.
    pub segments: usize,
}

/// The net change after an event id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeSet {
    /// The folded rows, keyed by path hash.
    pub summary: Summary<PathKey>,
    /// The path of every key a row mentions.
    pub shown: HashMap<PathKey, String>,
    /// The id the scan started after.
    pub since_id: i64,
    /// The last row id read; `since_id` when there was none.
    pub last_id: i64,
    /// The scan stopped at its cancel flag: the result is partial.
    pub cancelled: bool,
}

impl ChangeSet {
    /// The path text of `key`, empty when unknown.
    #[must_use]
    pub fn path(&self, key: PathKey) -> &str {
        self.shown.get(&key).map_or("", String::as_str)
    }
}

/// FNV-1a over the upper-cased UTF-16 units of a path.
#[must_use]
pub fn path_key(path: &[u16]) -> PathKey {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &unit in path {
        for byte in fold_upper(unit).to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

impl Reader {
    /// The net change of every row with an id greater than `since_id`.
    ///
    /// `progress` is called about every [`STEP`] rows and when a segment
    /// finishes.
    ///
    /// # Errors
    ///
    /// The catalog's error, or SQLite's on a segment that exists.
    pub fn changes_since(
        &self,
        since_id: i64,
        cancel: &CancelFlag,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<ChangeSet, CatalogError> {
        let catalog = Catalog::open_for_reading(&self.dir.join("catalog.db"))?;
        let mut segments = catalog.list()?;
        segments.retain(|s| !s.sealed || s.last_id > since_id);
        let files: Vec<String> = segments.into_iter().map(|s| s.file).collect();
        let mut run = Run {
            fold: Fold::new(),
            exemplars: HashMap::new(),
            scanned: 0,
            last_id: since_id,
            cancelled: false,
        };
        for (index, file) in files.iter().enumerate() {
            let path = self.dir.join(file);
            let Ok(conn) = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            else {
                continue;
            };
            let cache = self.take_cache(file);
            let (result, cache) =
                run.segment(&conn, cache, since_id, index, cancel, &mut |scanned| {
                    progress(Progress {
                        scanned,
                        segments_done: index,
                        segments: files.len(),
                    });
                });
            self.keep_cache(file, cache);
            result?;
            progress(Progress {
                scanned: run.scanned,
                segments_done: index + 1,
                segments: files.len(),
            });
            if run.cancelled {
                break;
            }
        }
        let summary = run.fold.finish();
        let shown = self.resolve(&files, &summary, &run.exemplars);
        Ok(ChangeSet {
            summary,
            shown,
            since_id,
            last_id: run.last_id,
            cancelled: run.cancelled,
        })
    }

    /// The path text of every key a row of the summary mentions.
    fn resolve(
        &self,
        files: &[String],
        summary: &Summary<PathKey>,
        exemplars: &HashMap<PathKey, (usize, i64)>,
    ) -> HashMap<PathKey, String> {
        let mut wanted: Vec<PathKey> = Vec::new();
        for row in &summary.rows {
            wanted.push(row.key);
            wanted.extend(row.from);
        }
        wanted.sort_unstable();
        wanted.dedup();
        let mut shown = HashMap::with_capacity(wanted.len());
        let mut open: Option<(usize, Connection)> = None;
        // Group by segment so each file opens once.
        let mut by_segment: Vec<(usize, i64, PathKey)> = wanted
            .iter()
            .filter_map(|key| exemplars.get(key).map(|&(seg, node)| (seg, node, *key)))
            .collect();
        by_segment.sort_unstable();
        for (seg, node, key) in by_segment {
            if open.as_ref().is_none_or(|(at, _)| *at != seg) {
                open = files.get(seg).and_then(|file| {
                    Connection::open_with_flags(
                        self.dir.join(file),
                        OpenFlags::SQLITE_OPEN_READ_ONLY,
                    )
                    .ok()
                    .map(|conn| (seg, conn))
                });
            }
            let Some((_, conn)) = open.as_ref() else {
                continue;
            };
            let (_, path) = Paths::with_cache(conn, PathCache::default()).path_of(node);
            shown.insert(key, String::from_utf16_lossy(&path));
        }
        shown
    }
}

/// The state of one scan across segments.
struct Run {
    fold: Fold<PathKey>,
    /// The first node each key was seen at, to show its path afterwards.
    exemplars: HashMap<PathKey, (usize, i64)>,
    scanned: u64,
    last_id: i64,
    cancelled: bool,
}

impl Run {
    fn segment(
        &mut self,
        conn: &Connection,
        cache: PathCache,
        since_id: i64,
        index: usize,
        cancel: &CancelFlag,
        report: &mut dyn FnMut(u64),
    ) -> (rusqlite::Result<()>, PathCache) {
        let mut paths = Paths::with_cache(conn, cache);
        let result = self.scan(conn, &mut paths, since_id, index, cancel, report);
        (result, paths.into_cache())
    }

    fn scan(
        &mut self,
        conn: &Connection,
        paths: &mut Paths<'_>,
        since_id: i64,
        index: usize,
        cancel: &CancelFlag,
        report: &mut dyn FnMut(u64),
    ) -> rusqlite::Result<()> {
        let mut statement = conn.prepare(
            "SELECT id, kind, node_id, file_id, file_id_hi, root_id, is_dir \
             FROM event WHERE id > ?1 ORDER BY id",
        )?;
        let mut rows = statement.query([since_id])?;
        let mut keys: HashMap<i64, PathKey> = HashMap::new();
        while let Some(row) = rows.next()? {
            self.scanned += 1;
            let id: i64 = row.get(0)?;
            self.last_id = self.last_id.max(id);
            if self.scanned.is_multiple_of(STEP) {
                report(self.scanned);
                if cancel.is_cancelled() {
                    self.cancelled = true;
                    return Ok(());
                }
            }
            let kind = kind_of(row.get(1)?);
            let is_dir: bool = row.get(6)?;
            // Rows that say nothing about a path need no path.
            if op_for(kind, is_dir, (), 0, 0).is_none() {
                continue;
            }
            let about_root = kind.is_gap() || kind == Kind::ReconcileFinished;
            let node: i64 = row.get(2)?;
            let (key, root) = if about_root {
                let root_id: i64 = row.get(5)?;
                // A root is told apart by its path, the same in every segment.
                let root = u32::try_from(path_key(&paths.root(root_id)) >> 32).unwrap_or(0);
                (0, root)
            } else {
                if keys.len() >= KEY_CACHE_NODES {
                    keys.clear();
                }
                let key = *keys
                    .entry(node)
                    .or_insert_with(|| path_key(&paths.path_of(node).1));
                (key, 0)
            };
            let file_id = join(FileIdColumns {
                lo: row.get::<_, Option<i64>>(3)?.unwrap_or(0),
                hi: row.get(4)?,
            });
            if let Some(op) = op_for(kind, is_dir, key, file_id, root) {
                if !about_root {
                    self.exemplars.entry(key).or_insert((index, node));
                }
                self.fold.push(op);
            }
        }
        Ok(())
    }
}
