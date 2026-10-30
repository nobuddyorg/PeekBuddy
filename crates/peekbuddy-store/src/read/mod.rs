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
//!
//! A glob or regex runs in Rust over the reconstructed path, in chunks of
//! [`CHUNK`] rows, checking the cancel flag between chunks and stopping after
//! [`SCAN_STEP`] rows with a resume cursor.
//!
//! # Memory
//!
//! Rows come in chunks of at most [`CHUNK`]; the path cache holds at most
//! [`PATH_CACHE_NODES`] nodes and is dropped when it is full.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use peekbuddy_core::event::{Event, Flags, Kind, NameKind, NameRef, RootIdx, Source, VolIdx};
use peekbuddy_core::filter::Scratch;
use peekbuddy_core::name::WideStr;
use peekbuddy_core::query::{
    CancelFlag, Direction, Filter, Page, PageRequest, Row, SCAN_STEP, fts_phrase, is_indexable,
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
pub const PATH_CACHE_NODES: usize = 8_192;

const BACKSLASH: u16 = 0x5C;

/// Reads pages from the segments of one data directory.
#[derive(Debug)]
pub struct Reader {
    dir: PathBuf,
}

impl Reader {
    /// A reader over the segments in `dir` (where `catalog.db` lives).
    #[must_use]
    pub fn new(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
        }
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
        let catalog = Catalog::open(&self.dir.join("catalog.db"))?;
        let mut segments = catalog.list()?;
        segments.retain(|segment| reaches(segment, request));
        if request.direction == Direction::Older {
            segments.reverse();
        }
        let mut walk = Walk::new(request, cancel);
        for segment in &segments {
            if walk.finished() {
                break;
            }
            let path = self.dir.join(&segment.file);
            let opened = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY);
            match opened {
                Ok(conn) if path.exists() => walk.segment(&conn)?,
                _ => walk.page.retired.push(segment.file.clone()),
            }
        }
        Ok(walk.into_page())
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
}

impl<'a> Walk<'a> {
    fn new(request: &'a PageRequest, cancel: &'a CancelFlag) -> Self {
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
    /// scan step is used up or the request is cancelled.
    fn segment(&mut self, conn: &Connection) -> Result<(), CatalogError> {
        let Some(query) = SegmentQuery::new(conn, &self.request.filter)? else {
            return Ok(());
        };
        let mut paths = Paths::new(conn);
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
            let fetched = query.fetch(conn, self.cursor, self.request.direction, want)?;
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
            if count < want {
                return Ok(());
            }
            if self.scratch.is_some() && self.page.scanned >= SCAN_STEP as u64 {
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
            reparse_tag: 0,
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
    file_id: u128,
}

/// The `SELECT` of one segment with the filter resolved against that
/// segment's dictionary (node and root ids differ between files).
struct SegmentQuery {
    sql: String,
    params: Vec<Value>,
}

impl SegmentQuery {
    /// `None` when the filter cannot match anything in this segment (its root
    /// or directory is not in the dictionary).
    fn new(conn: &Connection, filter: &Filter) -> rusqlite::Result<Option<Self>> {
        let mut clauses: Vec<String> = Vec::new();
        // ?1 cursor, ?2 limit; filter values from ?3 on.
        let mut params: Vec<Value> = Vec::new();
        let mut bind = |value: Value| {
            params.push(value);
            params.len() + 2
        };
        if let Some(kinds) = filter.kinds.filter(|kinds| !kinds.is_all()) {
            let tags: Vec<String> = kinds.tags().iter().map(u8::to_string).collect();
            clauses.push(format!("e.kind / 256 IN ({})", tags.join(",")));
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
        if let Some(dir) = &filter.subtree {
            let Some(node) = find_node(conn, dir)? else {
                return Ok(None);
            };
            let at = bind(Value::Integer(node));
            prefix = format!(
                "WITH RECURSIVE sub(id) AS (SELECT ?{at} UNION ALL \
                 SELECT n.id FROM node n JOIN sub ON n.parent_id = sub.id) "
            );
            clauses.push("e.node_id IN (SELECT id FROM sub)".to_owned());
        }
        if let Some(name) = filter.name.as_deref().filter(|name| !name.is_empty()) {
            if is_indexable(name) {
                let at = bind(Value::Text(fts_phrase(name)));
                clauses.push(format!(
                    "e.node_id IN (SELECT rowid FROM node_fts WHERE node_fts MATCH ?{at})"
                ));
            } else {
                let at = bind(Value::Text(like_literal(name)));
                clauses.push(format!(
                    "e.node_id IN (SELECT id FROM node WHERE name LIKE ?{at} ESCAPE '\\')"
                ));
            }
        }
        let mut sql = format!(
            "{prefix}SELECT e.id, e.ts, e.kind, e.source, e.raw_mask, e.node_id, e.is_dir, \
             e.size, e.attrs, e.file_id, e.file_id_hi, e.old_node_id, e.root_id, e.flags \
             FROM event e WHERE e.id {{cmp}} ?1"
        );
        for clause in clauses {
            sql.push_str(" AND ");
            sql.push_str(&clause);
        }
        sql.push_str(" ORDER BY e.id {order} LIMIT ?2");
        Ok(Some(Self { sql, params }))
    }

    fn plan(&self, conn: &Connection, direction: Direction) -> rusqlite::Result<Vec<String>> {
        let sql = format!("EXPLAIN QUERY PLAN {}", self.sql(direction));
        let mut statement = conn.prepare(&sql)?;
        let head = [Value::Integer(i64::MAX), Value::Integer(200)];
        let rows = statement.query_map(
            params_from_iter(head.iter().chain(self.params.iter())),
            |row| row.get::<_, String>(3),
        )?;
        rows.collect()
    }

    fn sql(&self, direction: Direction) -> String {
        let (cmp, order) = match direction {
            Direction::Older => ("<", "DESC"),
            Direction::Newer => (">", "ASC"),
        };
        self.sql.replace("{cmp}", cmp).replace("{order}", order)
    }

    fn fetch(
        &self,
        conn: &Connection,
        cursor: Option<i64>,
        direction: Direction,
        limit: usize,
    ) -> rusqlite::Result<Vec<Found>> {
        let start = match direction {
            Direction::Older => i64::MAX,
            Direction::Newer => i64::MIN,
        };
        let mut statement = conn.prepare_cached(&self.sql(direction))?;
        let head = [
            Value::Integer(cursor.unwrap_or(start)),
            Value::Integer(i64::try_from(limit).unwrap_or(i64::MAX)),
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
                })
            },
        )?;
        rows.collect()
    }
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
    cache: HashMap<i64, (i64, Vec<u16>)>,
    roots: HashMap<i64, Vec<u16>>,
}

impl<'a> Paths<'a> {
    fn new(conn: &'a Connection) -> Self {
        Self {
            conn,
            cache: HashMap::new(),
            roots: HashMap::new(),
        }
    }

    /// The parent and exact name of a node.
    fn node(&mut self, id: i64) -> Option<(i64, Vec<u16>)> {
        if let Some(hit) = self.cache.get(&id) {
            return Some(hit.clone());
        }
        let (parent, text, wide): (i64, String, Option<Vec<u8>>) = self
            .conn
            .prepare_cached("SELECT parent_id, name, name_w FROM node WHERE id = ?1")
            .ok()?
            .query_row([id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .optional()
            .ok()??;
        let name = from_columns(&text, wide.as_deref())?.into_units();
        if self.cache.len() >= PATH_CACHE_NODES {
            self.cache.clear();
        }
        self.cache.insert(id, (parent, name.clone()));
        Some((parent, name))
    }

    /// The name and full path of a node.
    fn path_of(&mut self, node_id: i64) -> (Vec<u16>, Vec<u16>) {
        let mut parts: Vec<Vec<u16>> = Vec::new();
        let mut at = node_id;
        let mut name = Vec::new();
        // The tree is acyclic by construction; the bound guards a damaged file.
        for depth in 0..4_096 {
            let Some((parent, part)) = self.node(at) else {
                break;
            };
            if depth == 0 {
                name.clone_from(&part);
            }
            parts.push(part);
            if parent == TOP_LEVEL {
                break;
            }
            at = parent;
        }
        let mut path = Vec::new();
        for (index, part) in parts.iter().rev().enumerate() {
            if index > 0 {
                path.push(BACKSLASH);
            }
            path.extend_from_slice(part);
        }
        (name, path)
    }

    /// The path of a watch root as stored (cached: a segment has a few).
    fn root(&mut self, id: i64) -> Vec<u16> {
        if let Some(hit) = self.roots.get(&id) {
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
        self.roots.insert(id, path.clone());
        path
    }

    /// The row with its name and full path.
    fn complete(&mut self, found: &Found) -> Row {
        let (name, path) = self.path_of(found.node_id);
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
            file_id: found.file_id,
            name,
            path,
            old_path,
            root,
            flags: found.flags,
        }
    }
}
