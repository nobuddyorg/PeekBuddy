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
use std::sync::Arc;

use peekbuddy_core::changes::{Fold, Summary, op_for};
use peekbuddy_core::event::Kind;
use peekbuddy_core::name::fold_upper;
use peekbuddy_core::query::CancelFlag;
use rusqlite::{Connection, OpenFlags, OptionalExtension};

use super::{BACKSLASH, PathCache, Paths, Reader, kind_of};
use crate::catalog::{Catalog, CatalogError};
use crate::ids::{FileIdColumns, join};
use crate::interner::TOP_LEVEL;
use crate::names::from_columns;

/// Rows scanned between two progress reports and cancel checks.
const STEP: u64 = 8_192;

/// Rows whose paths are resolved before [`Reader::changes_since`] returns.
/// A diff with more lines resolves the rest on demand
/// ([`Reader::resolve`]) and is listed in the order the files were first
/// seen, not alphabetically.
pub const EAGER: usize = 20_000;

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
    /// The path of every key resolved so far (see [`EAGER`]).
    pub shown: HashMap<PathKey, String>,
    /// Where each key was first seen: day file index and node, to resolve
    /// its path later.
    pub exemplars: HashMap<PathKey, (usize, i64)>,
    /// The day files the exemplars point into.
    pub files: Vec<String>,
    /// The watch roots the gaps of the summary name, by [`GapInfo::root`].
    ///
    /// [`GapInfo::root`]: peekbuddy_core::changes::GapInfo::root
    pub roots: HashMap<u32, String>,
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

/// The state of the path hash before any unit.
const SEED: u64 = 0xcbf2_9ce4_8422_2325;

/// One more unit into the hash (upper-cased, as the file system compares).
fn extend(mut hash: u64, unit: u16) -> u64 {
    for byte in fold_upper(unit).to_le_bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// FNV-1a over the upper-cased UTF-16 units of a path. Hashing the units of
/// a parent, a backslash and a name in turn gives the same value as hashing
/// the joined path, which is how the scan finds a file's key without
/// building its path.
#[must_use]
pub fn path_key(path: &[u16]) -> PathKey {
    path.iter().fold(SEED, |hash, &unit| extend(hash, unit))
}

/// The hash state after the path of directory node `id`, from the states of
/// the directories seen so far.
fn dir_state(paths: &mut Paths<'_>, id: i64, states: &mut HashMap<i64, u64>) -> u64 {
    let mut chain: Vec<(i64, Arc<[u16]>)> = Vec::new();
    let mut at = id;
    let mut base = None;
    // The tree is acyclic; the bound guards a damaged file.
    for _ in 0..4_096 {
        if at == TOP_LEVEL {
            break;
        }
        if let Some(&known) = states.get(&at) {
            base = Some(known);
            break;
        }
        let Some((up, name)) = paths.node(at, true) else {
            break;
        };
        chain.push((at, name));
        at = up;
    }
    let mut state = base.unwrap_or(SEED);
    let mut has_prefix = base.is_some();
    for (node, name) in chain.into_iter().rev() {
        if has_prefix {
            state = extend(state, BACKSLASH);
        }
        state = name.iter().fold(state, |hash, &unit| extend(hash, unit));
        has_prefix = true;
        states.insert(node, state);
    }
    state
}

/// The key of the file in column `name` (and `wide` when the name is not
/// valid UTF-16 text) below the directory with state `prefix`.
fn leaf_key(prefix: u64, has_prefix: bool, text: &str, wide: Option<&[u8]>) -> Option<u64> {
    let mut state = prefix;
    if has_prefix {
        state = extend(state, BACKSLASH);
    }
    match wide {
        None => Some(text.encode_utf16().fold(state, extend)),
        Some(bytes) => {
            let name = from_columns(text, Some(bytes))?;
            Some(name.into_units().into_iter().fold(state, extend))
        }
    }
}

impl Reader {
    /// The net change of every row with an id greater than `since_id`.
    ///
    /// `progress` is called about every `STEP` rows and when a segment
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
            roots: HashMap::new(),
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
        let mut summary = run.fold.finish();
        // Listed in the order the files were first seen: no strings needed.
        let order = |key: PathKey| run.exemplars.get(&key).copied().unwrap_or_default();
        summary.rows.sort_by_cached_key(|r| (r.kind, order(r.key)));
        let mut set = ChangeSet {
            summary,
            shown: HashMap::new(),
            exemplars: run.exemplars,
            files,
            roots: run.roots,
            since_id,
            last_id: run.last_id,
            cancelled: run.cancelled,
        };
        let eager: Vec<PathKey> = set
            .summary
            .rows
            .iter()
            .take(EAGER)
            .flat_map(|r| std::iter::once(r.key).chain(r.from))
            .collect();
        set.shown = self.resolve(&set, &eager);
        if set.summary.rows.len() <= EAGER {
            // Everything is known: alphabetical, case-folded.
            let shown = &set.shown;
            set.summary
                .rows
                .sort_by_cached_key(|r| (r.kind, shown.get(&r.key).map(|p| p.to_uppercase())));
        }
        Ok(set)
    }

    /// The id of the last row older than `ts` (FILETIME ticks), or 0 when
    /// every row is as old or newer: "since this time" is "after this id".
    /// Ids grow with time, so each segment is searched by halving.
    ///
    /// # Errors
    ///
    /// The catalog's error, or SQLite's.
    pub fn id_before(&self, ts: u64) -> Result<i64, CatalogError> {
        let ts = i64::try_from(ts).unwrap_or(i64::MAX);
        let catalog = Catalog::open_for_reading(&self.dir.join("catalog.db"))?;
        let mut found = 0;
        for segment in catalog.list()? {
            let Ok(conn) = Connection::open_with_flags(
                self.dir.join(&segment.file),
                OpenFlags::SQLITE_OPEN_READ_ONLY,
            ) else {
                continue;
            };
            let (low, high): (Option<i64>, Option<i64>) =
                conn.query_row("SELECT min(id), max(id) FROM event", [], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })?;
            let (Some(mut low), Some(mut high)) = (low, high) else {
                continue;
            };
            let ts_of = |id: i64| -> rusqlite::Result<Option<i64>> {
                conn.query_row(
                    "SELECT ts FROM event WHERE id >= ?1 ORDER BY id LIMIT 1",
                    [id],
                    |r| r.get(0),
                )
                .optional()
            };
            // The newest id whose ts is before `ts`.
            let mut last_older = None;
            while low <= high {
                let mid = low + (high - low) / 2;
                match ts_of(mid)? {
                    Some(at) if at < ts => {
                        last_older = Some(mid);
                        low = mid + 1;
                    }
                    _ => high = mid - 1,
                }
            }
            match last_older {
                Some(id) => found = found.max(id),
                None => break,
            }
        }
        Ok(found)
    }

    /// The path text of `keys` of `set`; a key the set does not know is
    /// left out.
    #[must_use]
    pub fn resolve(&self, set: &ChangeSet, keys: &[PathKey]) -> HashMap<PathKey, String> {
        let (files, exemplars) = (&set.files, &set.exemplars);
        let mut wanted: Vec<PathKey> = keys.to_vec();
        wanted.sort_unstable();
        wanted.dedup();
        let mut shown = HashMap::with_capacity(wanted.len());
        // Grouped by segment, so each file opens once and its paths share
        // one cache of directories.
        let mut by_segment: Vec<(usize, i64, PathKey)> = wanted
            .iter()
            .filter_map(|key| exemplars.get(key).map(|&(seg, node)| (seg, node, *key)))
            .collect();
        by_segment.sort_unstable();
        for group in by_segment.chunk_by(|a, b| a.0 == b.0) {
            let Some(file) = group.first().and_then(|(seg, ..)| files.get(*seg)) else {
                continue;
            };
            let Ok(conn) =
                Connection::open_with_flags(self.dir.join(file), OpenFlags::SQLITE_OPEN_READ_ONLY)
            else {
                continue;
            };
            let mut resolver = Paths::with_cache(&conn, PathCache::default());
            for &(_, node, key) in group {
                let (_, path) = resolver.path_of(node);
                shown.insert(key, String::from_utf16_lossy(&path));
            }
        }
        shown
    }
}

/// The state of one scan across segments.
struct Run {
    fold: Fold<PathKey>,
    /// The first node each key was seen at, to show its path afterwards.
    exemplars: HashMap<PathKey, (usize, i64)>,
    roots: HashMap<u32, String>,
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
            "SELECT e.id, e.kind, e.node_id, e.file_id, e.file_id_hi, e.root_id, e.is_dir, \
             n.parent_id, n.name, n.name_w \
             FROM event e LEFT JOIN node n ON n.id = e.node_id \
             WHERE e.id > ?1 ORDER BY e.id",
        )?;
        let mut rows = statement.query([since_id])?;
        let mut states: HashMap<i64, u64> = HashMap::new();
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
                let root_path = paths.root(root_id);
                let root = u32::try_from(path_key(&root_path) >> 32).unwrap_or(0);
                self.roots
                    .entry(root)
                    .or_insert_with(|| String::from_utf16_lossy(&root_path));
                (0, root)
            } else {
                if states.len() >= KEY_CACHE_NODES {
                    states.clear();
                }
                let key = match row.get::<_, Option<i64>>(7)? {
                    Some(parent) => {
                        let prefix = dir_state(paths, parent, &mut states);
                        let text = row.get_ref(8)?.as_str().unwrap_or_default();
                        let wide = row.get_ref(9)?.as_blob_or_null()?;
                        leaf_key(prefix, parent != TOP_LEVEL, text, wide)
                    }
                    None => None,
                }
                .unwrap_or_else(|| path_key(&paths.path_of(node).1));
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
