//! Paging through the event log from the viewer (#55).
//!
//! A [`Reader`] answers [`PageRequest`]s over every segment of a data
//! directory. It never holds more than one read-only connection (one segment
//! at a time, no `ATTACH`, ADR 0017) and pages by keyset: `id < cursor`,
//! never `OFFSET`. The segments come from `catalog.db`, newest first for
//! [`Direction::Older`] and oldest first for [`Direction::Newer`]. A segment
//! that retention deleted since the catalog was read is reported in
//! [`Page::retired`] and skipped, not an error.
//!
//! # What the database answers
//!
//! | Filter | How |
//! | --- | --- |
//! | kinds | `kind / 256 IN (...)` |
//! | root | the root id in this segment by path, then `event_root` |
//! | time | `stats` bounds the id range per hour, then `ts` refines |
//! | subtree | a recursive CTE over `node`, then `event_node` |
//! | name | the trigram index for three characters or more, else `LIKE` |
//! | hide | `flags & mask = 0`; [`Reader::hidden`] counts the other side |
//!
//! A glob or regex runs in Rust over the reconstructed path, in chunks of
//! [`CHUNK`] rows, checking the cancel flag between chunks. Every query that
//! cannot use an index to find its first page (a pattern, kinds, a subtree, a
//! name, hidden flags) walks the segment in windows of ids and stops after
//! [`SCAN_BUDGET`] with a resume cursor; the window grows while windows are
//! cheap and shrinks when they are not, so a step ends after about the
//! budget, not after a row count.
//!
//! The subtree and name filters test their dictionary ids with `+node_id`,
//! which keeps SQLite from sorting every match (`USE TEMP B-TREE FOR ORDER
//! BY`) before it can return the first page: the plan walks the ids in order
//! and checks membership. A page that finds its rows in the first window costs
//! the same as without the filter; one that finds none costs one window per
//! step instead of a sort of everything.
//!
//! # Memory
//!
//! Rows come in chunks of at most [`CHUNK`]; the path cache holds at most
//! [`PATH_CACHE_NODES`] nodes and is dropped when it is full.

pub mod changes;
pub mod stats;

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use peekbuddy_core::event::{Event, Flags, Kind, NameKind, NameRef, RootIdx, Source, VolIdx};
use peekbuddy_core::filter::Scratch;
use peekbuddy_core::name::WideStr;
use peekbuddy_core::query::{
    CancelFlag, Direction, Filter, Page, PageRequest, Row, SCAN_BUDGET, fts_phrase, is_indexable,
    like_literal,
};
use rusqlite::types::Value;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params_from_iter};

use crate::catalog::{Catalog, CatalogError, SegmentRow};
use crate::ids::{FileIdColumns, join};
use crate::interner::TOP_LEVEL;
use crate::names::{from_columns, to_columns};
use crate::schema::{TICKS_PER_HOUR, hour_bucket};

/// Rows fetched from SQLite at once for a post-filter.
pub const CHUNK: usize = 1_000;
/// Nodes in the path cache of one segment.
pub const PATH_CACHE_NODES: usize = 65_536;
/// The ids a scan window starts with.
const FIRST_WINDOW: i64 = 4_096;
/// The smallest and the largest scan window, in ids.
const WINDOW_RANGE: (i64, i64) = (256, 4_000_000);

const BACKSLASH: u16 = 0x5C;

/// Reads pages from the segments of one data directory.
#[derive(Debug)]
pub struct Reader {
    dir: PathBuf,
    scan_budget: Duration,
    /// The nodes and roots seen in each segment, kept between calls: a scan
    /// that runs in 4 ms steps would otherwise look up the same directories
    /// again at the start of every step. Nodes never change once written, and
    /// a segment file is never rewritten, so an entry stays right until the
    /// file is deleted.
    caches: Mutex<HashMap<String, PathCache>>,
}

/// Segments whose path caches a reader keeps.
const KEPT_SEGMENTS: usize = 8;

impl Reader {
    /// A reader over the segments in `dir` (where `catalog.db` lives).
    #[must_use]
    pub fn new(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
            scan_budget: SCAN_BUDGET,
            caches: Mutex::new(HashMap::new()),
        }
    }

    /// The same reader with another budget for one scan step (tests use a
    /// zero budget to get one window per call).
    #[must_use]
    pub const fn with_scan_budget(mut self, budget: Duration) -> Self {
        self.scan_budget = budget;
        self
    }

    fn take_cache(&self, file: &str) -> PathCache {
        self.caches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(file)
            .unwrap_or_default()
    }

    fn keep_cache(&self, file: &str, cache: PathCache) {
        let mut caches = self.caches.lock().unwrap_or_else(PoisonError::into_inner);
        if caches.len() >= KEPT_SEGMENTS {
            caches.clear();
        }
        caches.insert(file.to_owned(), cache);
    }

    /// The data directory this reader reads.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Answers one request.
    ///
    /// # Errors
    ///
    /// SQLite's error, or the catalog's. A segment that disappeared is not an
    /// error: it is listed in [`Page::retired`].
    pub fn page(&self, request: &PageRequest, cancel: &CancelFlag) -> Result<Page, CatalogError> {
        let catalog = Catalog::open_for_reading(&self.dir.join("catalog.db"))?;
        let mut segments = catalog.list()?;
        segments.retain(|segment| reaches(segment, request));
        if request.direction == Direction::Older {
            segments.reverse();
        }
        let mut walk = Walk::new(request, cancel, self.scan_budget);
        for segment in &segments {
            if walk.finished() {
                break;
            }
            let path = self.dir.join(&segment.file);
            let opened = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY);
            match opened {
                Ok(conn) if path.exists() => {
                    let cache = self.take_cache(&segment.file);
                    let paths = walk.segment(&conn, cache);
                    self.keep_cache(&segment.file, paths.1);
                    paths.0?;
                }
                _ => walk.page.retired.push(segment.file.clone()),
            }
        }
        Ok(walk.into_page())
    }

    /// How many rows the filter hides (`filter.hide`) that pass everything
    /// else in it: the "N hidden" of the view. One `COUNT` per segment; the
    /// cancel flag is checked between segments, and a cancelled count is
    /// `None`.
    ///
    /// # Errors
    ///
    /// SQLite's error, or the catalog's.
    pub fn hidden(
        &self,
        filter: &Filter,
        cancel: &CancelFlag,
    ) -> Result<Option<u64>, CatalogError> {
        let catalog = Catalog::open_for_reading(&self.dir.join("catalog.db"))?;
        let mut total = 0_u64;
        for segment in catalog.list()? {
            if cancel.is_cancelled() {
                return Ok(None);
            }
            let path = self.dir.join(&segment.file);
            let Ok(conn) = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            else {
                continue;
            };
            if let Some(query) = SegmentQuery::hidden(&conn, filter)? {
                total = total.saturating_add(query.count(&conn)?);
            }
        }
        Ok(Some(total))
    }
}

/// The query plan of the page query for `filter` in one segment, one line per
/// step, for the plan snapshots and for diagnostics. Empty when the filter
/// cannot match anything in the segment.
///
/// # Errors
///
/// SQLite's error.
pub fn explain(
    conn: &Connection,
    filter: &Filter,
    direction: Direction,
) -> rusqlite::Result<Vec<String>> {
    let Some(query) = SegmentQuery::new(conn, filter)? else {
        return Ok(Vec::new());
    };
    query.plan(conn, direction)
}

/// As [`explain`], for the statement a node filter with few events uses (the
/// index and a sort); the same as [`explain`] for a filter without nodes.
///
/// # Errors
///
/// SQLite's error.
pub fn explain_selective(
    conn: &Connection,
    filter: &Filter,
    direction: Direction,
) -> rusqlite::Result<Vec<String>> {
    let Some(mut query) = SegmentQuery::new(conn, filter)? else {
        return Ok(Vec::new());
    };
    query.use_index = query.indexed.is_some();
    query.plan(conn, direction)
}

/// Whether `segment` can have ids on the requested side of the cursor. The
/// catalog row of the segment being written lags (`rows` and `last_id` are
/// set when it is sealed), so an unsealed segment is never ruled out by them.
fn reaches(segment: &SegmentRow, request: &PageRequest) -> bool {
    match (request.direction, request.cursor) {
        (_, None) => true,
        (Direction::Older, Some(cursor)) => segment.first_id < cursor,
        (Direction::Newer, Some(cursor)) => !segment.sealed || segment.last_id > cursor,
    }
}

/// The state of one request while it walks the segments.
struct Walk<'a> {
    request: &'a PageRequest,
    cancel: &'a CancelFlag,
    page: Page,
    rows: Vec<Row>,
    cursor: Option<i64>,
    full: bool,
    stopped: bool,
    scratch: Option<Scratch>,
    budget: Duration,
    started: Instant,
    /// Ids the next scan window covers.
    window: i64,
}

impl<'a> Walk<'a> {
    fn new(request: &'a PageRequest, cancel: &'a CancelFlag, budget: Duration) -> Self {
        Self {
            request,
            cancel,
            page: Page {
                generation: request.generation,
                ..Page::default()
            },
            rows: Vec::new(),
            cursor: request.cursor,
            full: false,
            stopped: false,
            scratch: request.filter.pattern.is_some().then(Scratch::new),
            budget,
            started: Instant::now(),
            window: FIRST_WINDOW,
        }
    }

    /// Whether this call has used up its time.
    fn out_of_time(&self) -> bool {
        self.started.elapsed() >= self.budget
    }

    /// The window after one that took `took`: twice as big when it was cheap,
    /// half when it used more than half the budget.
    fn adapt(&mut self, took: Duration) {
        let half = self.budget / 2;
        if took < half / 4 {
            self.window = self.window.saturating_mul(2).min(WINDOW_RANGE.1);
        } else if took > half {
            self.window = (self.window / 2).max(WINDOW_RANGE.0);
        }
    }

    fn finished(&self) -> bool {
        self.full || self.stopped
    }

    fn into_page(mut self) -> Page {
        if self.rows.len() > self.request.limit {
            self.rows.truncate(self.request.limit);
            self.page.more = true;
        }
        if self.request.direction == Direction::Newer {
            self.rows.reverse();
        }
        self.page.more |= self.page.resume.is_some();
        self.page.rows = self.rows;
        self.page
    }

    /// Reads from one segment until it is exhausted, the page is full, the
    /// time budget is used up or the request is cancelled.
    fn segment(
        &mut self,
        conn: &Connection,
        cache: PathCache,
    ) -> (Result<(), CatalogError>, PathCache) {
        let mut paths = Paths::with_cache(conn, cache);
        let result = self.scan(conn, &mut paths);
        (result, paths.into_cache())
    }

    fn scan(&mut self, conn: &Connection, paths: &mut Paths<'_>) -> Result<(), CatalogError> {
        let Some(mut query) = SegmentQuery::new(conn, &self.request.filter)? else {
            return Ok(());
        };
        query.choose(conn)?;
        let direction = self.request.direction;
        // Queries that cannot start from an index scan in windows of ids.
        let bounds = if query.walks_windows() {
            let Some(bounds) = segment_ids(conn)? else {
                return Ok(());
            };
            Some(bounds)
        } else {
            None
        };
        let stepped = query.walks_windows() || self.scratch.is_some();
        let limit = self.request.limit;
        loop {
            if self.cancel.is_cancelled() {
                self.page.cancelled = true;
                self.stopped = true;
                return Ok(());
            }
            let want = if self.scratch.is_some() {
                CHUNK
            } else {
                (limit + 1).saturating_sub(self.rows.len()).max(1)
            };
            // The ids this fetch may look at: from `start` to `bound`.
            let (start, bound) = match (bounds, direction) {
                (Some((low, high)), Direction::Older) => {
                    let start = self
                        .cursor
                        .map_or(high.saturating_add(1), |at| at.min(high.saturating_add(1)));
                    (Some(start), start.saturating_sub(self.window).max(low))
                }
                (Some((low, high)), Direction::Newer) => {
                    let start = self
                        .cursor
                        .map_or(low.saturating_sub(1), |at| at.max(low.saturating_sub(1)));
                    (Some(start), start.saturating_add(self.window).min(high))
                }
                (None, Direction::Older) => (self.cursor, i64::MIN),
                (None, Direction::Newer) => (self.cursor, i64::MAX),
            };
            let fetch_started = Instant::now();
            let fetched = query.fetch(conn, start, direction, want, bound)?;
            let took = fetch_started.elapsed();
            let count = fetched.len();
            for found in fetched {
                self.page.scanned += 1;
                self.cursor = Some(found.id);
                let row = paths.complete(&found);
                if self.passes(&row) {
                    self.rows.push(row);
                    if self.rows.len() > limit {
                        self.full = true;
                        return Ok(());
                    }
                }
            }
            let exhausted = count < want;
            if let (Some((low, high)), true) = (bounds, exhausted) {
                // The window had fewer matches than wanted: it is done, and so
                // is the segment when the window reached its end.
                self.cursor = Some(bound);
                self.page.scanned += start
                    .map_or(0, |at| at.abs_diff(bound))
                    .saturating_sub(u64::try_from(count).unwrap_or(0));
                self.adapt(took);
                let at_end = match direction {
                    Direction::Older => bound <= low,
                    Direction::Newer => bound >= high,
                };
                if at_end {
                    return Ok(());
                }
            } else if exhausted {
                return Ok(());
            }
            if stepped && self.out_of_time() {
                self.page.resume = self.cursor;
                self.stopped = true;
                return Ok(());
            }
        }
    }

    /// The glob or regex post-filter; rows pass when there is none.
    fn passes(&mut self, row: &Row) -> bool {
        let (Some(set), Some(scratch)) = (&self.request.filter.pattern, self.scratch.as_mut())
        else {
            return true;
        };
        let event = Event {
            ts: row.ts,
            qpc: 0,
            src_ts: 0,
            source: row.source,
            kind: row.kind,
            raw_mask: row.raw_mask,
            source_info: 0,
            is_dir: row.is_dir,
            size: row.size.unwrap_or(0),
            attrs: row.attrs.unwrap_or(0),
            reparse_tag: row.reparse_tag,
            file_id: row.file_id,
            parent_file_id: 0,
            usn: 0,
            vol: VolIdx(0),
            root: RootIdx(0),
            name: NameRef::EMPTY,
            name_kind: NameKind::Unknown,
            flags: Flags::NONE,
        };
        set.eval(&event, WideStr::new(&row.path), scratch)
            .matched_rule
            .is_some()
    }
}

/// A row as SQLite returns it, before its path is built.
struct Found {
    id: i64,
    ts: u64,
    kind: Kind,
    source: Source,
    raw_mask: u32,
    node_id: i64,
    old_node_id: Option<i64>,
    root_id: i64,
    flags: u16,
    is_dir: bool,
    size: Option<u64>,
    attrs: Option<u32>,
    reparse_tag: u32,
    file_id: u128,
    /// The node of the row, from the same statement: its parent and exact
    /// name. `None` when the node row is missing (a damaged file).
    leaf: Option<(i64, Vec<u16>)>,
}

/// The `SELECT` of one segment with the filter resolved against that
/// segment's dictionary (node and root ids differ between files).
struct SegmentQuery {
    /// The statement that walks the ids in order and tests node membership
    /// (`+node_id`): no sort, cost proportional to the ids it looks at.
    sql: String,
    /// For a filter on nodes: the statement that finds the events of the
    /// matching nodes through `event_node` and sorts them; and the one that
    /// counts them (capped), to tell which of the two is cheaper.
    indexed: Option<(String, String)>,
    params: Vec<Value>,
    /// The query has no index to start from: it walks windows of ids.
    windowed: bool,
    /// The sorting statement was chosen for this segment.
    use_index: bool,
}

/// Matching events up to which a node filter is answered through the index
/// and a sort: sorting them costs about as much as looking at that many ids,
/// and a rare filter would otherwise walk every id of the segment.
const SELECTIVE_EVENTS: i64 = 20_000;

impl SegmentQuery {
    /// `None` when the filter cannot match anything in this segment (its root
    /// or directory is not in the dictionary).
    fn new(conn: &Connection, filter: &Filter) -> rusqlite::Result<Option<Self>> {
        Self::build(conn, filter, false)
    }

    /// The count of the rows the filter hides in this segment (those with any
    /// of `filter.hide` that pass everything else). `None` when nothing is
    /// hidden by the filter or the segment cannot match.
    fn hidden(conn: &Connection, filter: &Filter) -> rusqlite::Result<Option<Self>> {
        Self::build(conn, filter, true)
    }

    fn build(conn: &Connection, filter: &Filter, hidden: bool) -> rusqlite::Result<Option<Self>> {
        let mut clauses: Vec<String> = Vec::new();
        let mut windowed = false;
        // ?1 cursor, ?2 limit, ?3 the end of the id window; filter values from
        // ?4 on.
        let mut params: Vec<Value> = Vec::new();
        let mut bind = |value: Value| {
            params.push(value);
            params.len() + 3
        };
        if let Some(kinds) = filter.kinds.filter(|kinds| !kinds.is_all()) {
            let tags: Vec<String> = kinds.tags().iter().map(u8::to_string).collect();
            clauses.push(format!("e.kind / 256 IN ({})", tags.join(",")));
            windowed = true;
        }
        if let Some(root) = &filter.root {
            let columns = to_columns(WideStr::new(root));
            let found: Option<i64> = conn
                .query_row(
                    "SELECT id FROM root WHERE path = ?1 ORDER BY id LIMIT 1",
                    [&columns.text],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(id) = found else {
                return Ok(None);
            };
            clauses.push(format!("e.root_id = ?{}", bind(Value::Integer(id))));
        }
        if filter.time.from.is_some() || filter.time.to.is_some() {
            let Some((low, high)) = id_bounds(conn, filter)? else {
                return Ok(None);
            };
            let low = bind(Value::Integer(low));
            let high = bind(Value::Integer(high));
            clauses.push(format!("e.id BETWEEN ?{low} AND ?{high}"));
            if let Some(from) = filter.time.from {
                let at = bind(Value::Integer(to_i64(from)));
                clauses.push(format!("e.ts >= ?{at}"));
            }
            if let Some(to) = filter.time.to {
                let at = bind(Value::Integer(to_i64(to)));
                clauses.push(format!("e.ts <= ?{at}"));
            }
        }
        let mut prefix = String::new();
        let mut nodes: Vec<String> = Vec::new();
        if let Some(dir) = &filter.subtree {
            let Some(node) = find_node(conn, dir)? else {
                return Ok(None);
            };
            let at = bind(Value::Integer(node));
            prefix = format!(
                "WITH RECURSIVE sub(id) AS (SELECT ?{at} UNION ALL \
                 SELECT n.id FROM node n JOIN sub ON n.parent_id = sub.id) "
            );
            nodes.push("e.node_id IN (SELECT id FROM sub)".to_owned());
            windowed = true;
        }
        if let Some(name) = filter.name.as_deref().filter(|name| !name.is_empty()) {
            if is_indexable(name) {
                let at = bind(Value::Text(fts_phrase(name)));
                nodes.push(format!(
                    "e.node_id IN (SELECT rowid FROM node_fts WHERE node_fts MATCH ?{at})"
                ));
            } else {
                let at = bind(Value::Text(like_literal(name)));
                nodes.push(format!(
                    "e.node_id IN (SELECT id FROM node WHERE name LIKE ?{at} ESCAPE '\\')"
                ));
            }
        }
        windowed |= filter.name.as_deref().is_some_and(|name| !name.is_empty());
        let mask = filter.hide.bits();
        if mask != 0 {
            let test = if hidden { "!= 0" } else { "= 0" };
            clauses.push(format!("e.flags & {mask} {test}"));
            windowed |= !hidden;
        } else if hidden {
            return Ok(None);
        }
        // The leaf node rides along (its parent and name), so a row costs no
        // second statement; the join is by primary key from the event side.
        let select = if hidden {
            "SELECT COUNT(*) FROM event e"
        } else {
            "SELECT e.id, e.ts, e.kind, e.source, e.raw_mask, e.node_id, e.is_dir, \
             e.size, e.attrs, e.file_id, e.file_id_hi, e.old_node_id, e.root_id, e.flags, \
             e.reparse_tag, n.parent_id, n.name, n.name_w \
             FROM event e LEFT JOIN node n ON n.id = e.node_id"
        };
        let (sql, indexed) = statements(&prefix, select, &clauses, &nodes, hidden);
        Ok(Some(Self {
            sql,
            indexed,
            params,
            windowed,
            use_index: false,
        }))
    }

    /// Picks the statement for this segment: a node filter whose events are
    /// few is answered through the index and a sort, any other walks the ids.
    fn choose(&mut self, conn: &Connection) -> rusqlite::Result<()> {
        self.use_index = false;
        if let Some((_, probe)) = &self.indexed {
            let head = [Value::Null, Value::Null, Value::Null];
            let found: i64 = conn.prepare_cached(probe)?.query_row(
                params_from_iter(head.iter().chain(self.params.iter())),
                |row| row.get(0),
            )?;
            self.use_index = found <= SELECTIVE_EVENTS;
        }
        Ok(())
    }

    /// Whether the statement in use walks windows of ids.
    fn walks_windows(&self) -> bool {
        self.windowed && !self.use_index
    }

    /// The number a counting query returns.
    fn count(&self, conn: &Connection) -> rusqlite::Result<u64> {
        let mut statement = conn.prepare_cached(&self.sql(Direction::Newer))?;
        let head = [
            Value::Integer(i64::MIN),
            Value::Integer(i64::MAX),
            Value::Integer(i64::MAX),
        ];
        let found: i64 = statement.query_row(
            params_from_iter(head.iter().chain(self.params.iter())),
            |row| row.get(0),
        )?;
        Ok(u64::try_from(found).unwrap_or(0))
    }

    fn plan(&self, conn: &Connection, direction: Direction) -> rusqlite::Result<Vec<String>> {
        let sql = format!("EXPLAIN QUERY PLAN {}", self.sql(direction));
        let mut statement = conn.prepare(&sql)?;
        let head = [
            Value::Integer(i64::MAX),
            Value::Integer(200),
            Value::Integer(i64::MIN),
        ];
        let rows = statement.query_map(
            params_from_iter(head.iter().chain(self.params.iter())),
            |row| row.get::<_, String>(3),
        )?;
        rows.collect()
    }

    fn sql(&self, direction: Direction) -> String {
        let (cmp, win, order) = match direction {
            Direction::Older => ("<", ">=", "DESC"),
            Direction::Newer => (">", "<=", "ASC"),
        };
        let text = match (&self.indexed, self.use_index) {
            (Some((index, _)), true) => index,
            _ => &self.sql,
        };
        text.replace("{cmp}", cmp)
            .replace("{win}", win)
            .replace("{order}", order)
    }

    fn fetch(
        &self,
        conn: &Connection,
        cursor: Option<i64>,
        direction: Direction,
        limit: usize,
        bound: i64,
    ) -> rusqlite::Result<Vec<Found>> {
        let start = match direction {
            Direction::Older => i64::MAX,
            Direction::Newer => i64::MIN,
        };
        let mut statement = conn.prepare_cached(&self.sql(direction))?;
        let head = [
            Value::Integer(cursor.unwrap_or(start)),
            Value::Integer(i64::try_from(limit).unwrap_or(i64::MAX)),
            Value::Integer(bound),
        ];
        let rows = statement.query_map(
            params_from_iter(head.iter().chain(self.params.iter())),
            |row| {
                let kind: i64 = row.get(2)?;
                let source: i64 = row.get(3)?;
                let lo: Option<i64> = row.get(9)?;
                let hi: Option<i64> = row.get(10)?;
                Ok(Found {
                    id: row.get(0)?,
                    ts: u64::try_from(row.get::<_, i64>(1)?).unwrap_or(0),
                    kind: kind_of(kind),
                    source: u8::try_from(source)
                        .ok()
                        .and_then(Source::from_tag)
                        .unwrap_or(Source::System),
                    raw_mask: u32::try_from(row.get::<_, i64>(4)?).unwrap_or(0),
                    node_id: row.get(5)?,
                    is_dir: row.get::<_, i64>(6)? != 0,
                    size: row
                        .get::<_, Option<i64>>(7)?
                        .and_then(|size| u64::try_from(size).ok()),
                    attrs: row
                        .get::<_, Option<i64>>(8)?
                        .and_then(|attrs| u32::try_from(attrs).ok()),
                    file_id: join(FileIdColumns {
                        lo: lo.unwrap_or(0),
                        hi,
                    }),
                    old_node_id: row.get(11)?,
                    root_id: row.get(12)?,
                    flags: u16::try_from(row.get::<_, i64>(13)?).unwrap_or(0),
                    reparse_tag: row
                        .get::<_, Option<i64>>(14)?
                        .and_then(|tag| u32::try_from(tag).ok())
                        .unwrap_or(0),
                    leaf: match (
                        row.get::<_, Option<i64>>(15)?,
                        row.get::<_, Option<String>>(16)?,
                    ) {
                        (Some(parent), Some(text)) => {
                            let wide: Option<Vec<u8>> = row.get(17)?;
                            from_columns(&text, wide.as_deref())
                                .map(|name| (parent, name.into_units()))
                        }
                        _ => None,
                    },
                })
            },
        )?;
        rows.collect()
    }
}

/// The two statements of a query: the id walk, and for a filter on nodes the
/// index statement with the capped count that decides between them.
fn statements(
    prefix: &str,
    select: &str,
    clauses: &[String],
    nodes: &[String],
    hidden: bool,
) -> (String, Option<(String, String)>) {
    let mut sql = format!("{prefix}{select} WHERE e.id {{cmp}} ?1 AND e.id {{win}} ?3");
    let mut index = format!("{prefix}{select} WHERE e.id {{cmp}} ?1");
    let mut probe = String::from("SELECT count(*) FROM (SELECT 1 FROM event e WHERE 1");
    for clause in clauses {
        for text in [&mut sql, &mut index, &mut probe] {
            text.push_str(" AND ");
            text.push_str(clause);
        }
    }
    for clause in nodes {
        // The unary plus keeps the planner from the `event_node` index:
        // the ids are walked in order and the node is looked up.
        sql.push_str(" AND +");
        sql.push_str(clause);
        for text in [&mut index, &mut probe] {
            text.push_str(" AND ");
            text.push_str(clause);
        }
    }
    if !hidden {
        sql.push_str(" ORDER BY e.id {order}");
        index.push_str(" ORDER BY e.id {order}");
    }
    sql.push_str(" LIMIT ?2");
    index.push_str(" LIMIT ?2");
    let _ = write!(probe, " LIMIT {})", SELECTIVE_EVENTS + 1);
    let indexed = (!nodes.is_empty() && !hidden).then(|| (index, format!("{prefix}{probe}")));
    (sql, indexed)
}

/// The lowest and the highest event id of the segment; `None` when it is
/// empty.
fn segment_ids(conn: &Connection) -> rusqlite::Result<Option<(i64, i64)>> {
    conn.query_row("SELECT min(id), max(id) FROM event", [], |row| {
        Ok((row.get::<_, Option<i64>>(0)?, row.get::<_, Option<i64>>(1)?))
    })
    .map(|(low, high)| low.zip(high))
}

fn to_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn kind_of(packed: i64) -> Kind {
    let tag = u8::try_from(packed / 256).unwrap_or(u8::MAX);
    let arg = u8::try_from(packed % 256).unwrap_or(0);
    Kind::from_wire(tag, arg).unwrap_or(Kind::Other)
}

/// The id range the hours of the time filter cover, from `stats`; `None`
/// when no hour in the range has an event.
fn id_bounds(conn: &Connection, filter: &Filter) -> rusqlite::Result<Option<(i64, i64)>> {
    let from = filter
        .time
        .from
        .map_or(i64::MIN, |ts| hour_bucket(to_i64(ts)));
    let to = filter.time.to.map_or(i64::MAX, |ts| {
        hour_bucket(to_i64(ts)).min(i64::MAX - TICKS_PER_HOUR)
    });
    conn.query_row(
        "SELECT min(min_id), max(max_id) FROM stats WHERE hour_ts BETWEEN ?1 AND ?2",
        [from, to],
        |row| Ok((row.get::<_, Option<i64>>(0)?, row.get::<_, Option<i64>>(1)?)),
    )
    .map(|(low, high)| low.zip(high))
}

/// The node of the directory `path` (components joined by `\`, a `\\?\`
/// prefix ignored), walking from the top level.
fn find_node(conn: &Connection, path: &[u16]) -> rusqlite::Result<Option<i64>> {
    let mut parent = TOP_LEVEL;
    let mut statement = conn.prepare_cached(
        "SELECT id FROM node WHERE parent_id = ?1 AND name = ?2 \
         AND coalesce(name_w, x'') = coalesce(?3, x'')",
    )?;
    for part in path.split(|unit| *unit == BACKSLASH) {
        if part.is_empty() || part == [0x3F] {
            continue;
        }
        let columns = to_columns(WideStr::new(part));
        let found: Option<i64> = statement
            .query_row(
                rusqlite::params![parent, columns.text, columns.wide],
                |row| row.get(0),
            )
            .optional()?;
        match found {
            Some(id) => parent = id,
            None => return Ok(None),
        }
    }
    Ok((parent != TOP_LEVEL).then_some(parent))
}

/// Builds paths and names from a segment's node tree, with a bounded cache.
struct Paths<'a> {
    conn: &'a Connection,
    cache: PathCache,
}

/// What a segment's node tree and roots gave so far.
#[derive(Debug, Default)]
struct PathCache {
    nodes: HashMap<i64, (i64, Arc<[u16]>)>,
    roots: HashMap<i64, Vec<u16>>,
}

impl<'a> Paths<'a> {
    #[cfg(test)]
    fn new(conn: &'a Connection) -> Self {
        Self::with_cache(conn, PathCache::default())
    }

    fn with_cache(conn: &'a Connection, cache: PathCache) -> Self {
        Self { conn, cache }
    }

    fn into_cache(self) -> PathCache {
        self.cache
    }

    /// The parent and exact name of a node. `keep` caches it: the parents on
    /// a path are met again by the next row, the leaf almost never.
    fn node(&mut self, id: i64, keep: bool) -> Option<(i64, Arc<[u16]>)> {
        if let Some(hit) = self.cache.nodes.get(&id) {
            return Some(hit.clone());
        }
        let (parent, text, wide): (i64, String, Option<Vec<u8>>) = self
            .conn
            .prepare_cached("SELECT parent_id, name, name_w FROM node WHERE id = ?1")
            .ok()?
            .query_row([id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .optional()
            .ok()??;
        let name: Arc<[u16]> = from_columns(&text, wide.as_deref())?.into_units().into();
        if keep {
            if self.cache.nodes.len() >= PATH_CACHE_NODES {
                self.cache.nodes.clear();
            }
            self.cache.nodes.insert(id, (parent, Arc::clone(&name)));
        }
        Some((parent, name))
    }

    /// The name and full path of a node, found by its id.
    fn path_of(&mut self, node_id: i64) -> (Vec<u16>, Vec<u16>) {
        match self.node(node_id, false) {
            Some((parent, name)) => self.path_below(parent, &name),
            None => (Vec::new(), Vec::new()),
        }
    }

    /// The name and full path of a node whose parent and name are known.
    fn path_below(&mut self, parent: i64, name: &[u16]) -> (Vec<u16>, Vec<u16>) {
        let mut parents: Vec<Arc<[u16]>> = Vec::new();
        let mut at = parent;
        // The tree is acyclic by construction; the bound guards a damaged file.
        for _ in 0..4_096 {
            if at == TOP_LEVEL {
                break;
            }
            let Some((up, part)) = self.node(at, true) else {
                break;
            };
            parents.push(part);
            at = up;
        }
        let mut path = Vec::with_capacity(
            parents.iter().map(|part| part.len() + 1).sum::<usize>() + name.len(),
        );
        for part in parents.iter().rev() {
            path.extend_from_slice(part);
            path.push(BACKSLASH);
        }
        path.extend_from_slice(name);
        (name.to_vec(), path)
    }

    /// The path of a watch root as stored (cached: a segment has a few).
    fn root(&mut self, id: i64) -> Vec<u16> {
        if let Some(hit) = self.cache.roots.get(&id) {
            return hit.clone();
        }
        let found: Option<(String, Option<Vec<u8>>)> = self
            .conn
            .prepare_cached("SELECT path, path_w FROM root WHERE id = ?1")
            .ok()
            .and_then(|mut statement| {
                statement
                    .query_row([id], |row| Ok((row.get(0)?, row.get(1)?)))
                    .optional()
                    .ok()
                    .flatten()
            });
        let path = found
            .and_then(|(text, wide)| from_columns(&text, wide.as_deref()))
            .map_or_else(Vec::new, peekbuddy_core::name::WideName::into_units);
        self.cache.roots.insert(id, path.clone());
        path
    }

    /// The row with its name and full path.
    fn complete(&mut self, found: &Found) -> Row {
        let (name, path) = match &found.leaf {
            Some((parent, name)) => self.path_below(*parent, name),
            None => self.path_of(found.node_id),
        };
        let old_path = found
            .old_node_id
            .map_or_else(Vec::new, |old| self.path_of(old).1);
        let root = self.root(found.root_id);
        Row {
            id: found.id,
            ts: found.ts,
            kind: found.kind,
            source: found.source,
            raw_mask: found.raw_mask,
            is_dir: found.is_dir,
            size: found.size,
            attrs: found.attrs,
            reparse_tag: found.reparse_tag,
            file_id: found.file_id,
            name,
            path,
            old_path,
            root,
            flags: found.flags,
        }
    }
}

#[cfg(test)]
mod path_cache_tests {
    use rusqlite::params;

    use super::*;

    /// A node table with just the columns `Paths` reads.
    fn tree(nodes: i64) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE node (id INTEGER PRIMARY KEY, parent_id INTEGER NOT NULL, \
             name TEXT NOT NULL, name_w BLOB);",
        )
        .unwrap();
        let mut insert = conn
            .prepare("INSERT INTO node (id, parent_id, name) VALUES (?1, ?2, ?3)")
            .unwrap();
        for id in 1..=nodes {
            insert.execute(params![id, 0, format!("n{id}")]).unwrap();
        }
        drop(insert);
        conn
    }

    #[test]
    fn the_path_cache_never_outgrows_its_bound_whatever_the_log_size() {
        let nodes = i64::try_from(PATH_CACHE_NODES * 3).unwrap();
        let conn = tree(nodes);
        let mut paths = Paths::new(&conn);
        for id in 1..=nodes {
            let (parent, name) = paths.node(id, true).unwrap();
            assert_eq!(parent, TOP_LEVEL);
            assert_eq!(String::from_utf16_lossy(&name), format!("n{id}"));
            assert!(paths.cache.nodes.len() <= PATH_CACHE_NODES);
        }
        // The same bound for a log a tenth of the size: memory follows the
        // constant, not the log.
        let small = tree(i64::try_from(PATH_CACHE_NODES / 10).unwrap());
        let mut paths = Paths::new(&small);
        for id in 1..=i64::try_from(PATH_CACHE_NODES / 10).unwrap() {
            paths.node(id, true).unwrap();
        }
        assert_eq!(paths.cache.nodes.len(), PATH_CACHE_NODES / 10);
    }

    #[test]
    fn a_chain_of_nodes_gives_the_joined_path_and_the_leaf_name() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE node (id INTEGER PRIMARY KEY, parent_id INTEGER NOT NULL, \
             name TEXT NOT NULL, name_w BLOB);
             INSERT INTO node VALUES (1, 0, 'a', NULL), (2, 1, 'b', NULL), (3, 2, 'c.txt', NULL);",
        )
        .unwrap();
        let mut paths = Paths::new(&conn);
        let (name, path) = paths.path_of(3);
        assert_eq!(String::from_utf16_lossy(&name), "c.txt");
        assert_eq!(String::from_utf16_lossy(&path), r"a\b\c.txt");
    }

    #[test]
    fn a_packed_kind_gives_back_every_tag_and_payload() {
        let mut kinds = vec![
            Kind::Created,
            Kind::Deleted,
            Kind::ReconciledRenamed,
            Kind::StoreRecovered,
        ];
        for tag in [9, 10] {
            kinds.extend((0..=u8::MAX).filter_map(|arg| Kind::from_wire(tag, arg)));
        }
        for kind in kinds {
            let (tag, arg) = kind.to_wire();
            assert_eq!(kind_of(i64::from(tag) * 256 + i64::from(arg)), kind);
        }
        assert_eq!(kind_of(i64::from(u8::MAX) * 256 + 255), Kind::Other);
    }

    #[test]
    fn a_root_is_read_once_and_kept_in_the_cache() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE node (id INTEGER PRIMARY KEY, parent_id INTEGER NOT NULL,              name TEXT NOT NULL, name_w BLOB);
             CREATE TABLE root (id INTEGER PRIMARY KEY, path TEXT NOT NULL, path_w BLOB);
             INSERT INTO root VALUES (7, 'C:\\data', NULL);",
        )
        .unwrap();
        let mut paths = Paths::new(&conn);
        let expected: Vec<u16> = r"C:\data".encode_utf16().collect();
        assert_eq!(paths.root(7), expected);
        conn.execute("DELETE FROM root", []).unwrap();
        assert_eq!(paths.root(7), expected);
        assert_eq!(paths.root(8), Vec::<u16>::new());
        let cache = paths.into_cache();
        assert_eq!(cache.roots.len(), 2);
    }

    #[test]
    fn a_path_with_a_device_prefix_finds_its_directory_node() {
        let conn = tree(0);
        conn.execute_batch("INSERT INTO node VALUES (1, 0, 'a', NULL), (2, 1, 'b', NULL);")
            .unwrap();
        let wide = |text: &str| text.encode_utf16().collect::<Vec<u16>>();
        assert_eq!(find_node(&conn, &wide(r"\\?\a\b")).unwrap(), Some(2));
        assert_eq!(find_node(&conn, &wide(r"a\\b")).unwrap(), Some(2));
        assert_eq!(find_node(&conn, &wide(r"a\c")).unwrap(), None);
        assert_eq!(find_node(&conn, &wide("")).unwrap(), None);
    }
}
