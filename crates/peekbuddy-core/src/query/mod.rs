//! The read side of the event log, as types (#55).
//!
//! The viewer scrolls and filters a log of millions of rows without holding it
//! in memory. It asks for **pages** by keyset: the next page is `id < cursor`,
//! never an `OFFSET`; the previous page is `id > cursor`. A page is always in
//! display order, newest first. The SQL lives in `peekbuddy-store::read`; the
//! types are here so the viewer and its tests build on any host.
//!
//! # Requests and generations
//!
//! Every request carries a **generation**. The viewer bumps it on each new
//! filter or scroll target; a result whose generation is not the current one
//! is dropped. A request can also be cancelled (a shared flag), which stops a
//! scan at its next step.
//!
//! # Filters
//!
//! Kinds, watch root, time range, a directory subtree and a name substring are
//! answered by the database. A glob or regex is a post-filter in Rust over the
//! reconstructed path with the compiled matcher of #40: it scans in bounded
//! steps and reports progress (`scanned`), so a rare pattern over millions of
//! rows never freezes a frame.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::event::{Kind, Source};
use crate::filter::{FilterSet, KindSet};

/// Rows in a page.
pub const PAGE_SIZE: usize = 200;
/// Rows one call scans for a post-filter before it hands back what it has
/// (and a cursor to continue from).
pub const SCAN_STEP: usize = 20_000;

/// Which way a page goes from its cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Older rows: `id < cursor`, the next page when scrolling back in time.
    Older,
    /// Newer rows: `id > cursor`, the previous page.
    Newer,
}

/// A time range in FILETIME ticks; either end may be open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TimeRange {
    /// Inclusive start.
    pub from: Option<u64>,
    /// Inclusive end.
    pub to: Option<u64>,
}

/// What a page is filtered by. An empty filter lets everything through.
#[derive(Clone, Default)]
pub struct Filter {
    /// Kinds; `None` is every kind.
    pub kinds: Option<KindSet>,
    /// The watch root, by its absolute path (UTF-16).
    pub root: Option<Vec<u16>>,
    /// The time range.
    pub time: TimeRange,
    /// A directory: rows whose node is that directory or below it.
    pub subtree: Option<Vec<u16>>,
    /// A substring of the **name** (the last component), not the path.
    pub name: Option<String>,
    /// Glob or regex rules; a row passes when a rule matches its path.
    pub pattern: Option<Arc<FilterSet>>,
}

impl std::fmt::Debug for Filter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Filter")
            .field("kinds", &self.kinds)
            .field("root", &self.root.as_ref().map(Vec::len))
            .field("time", &self.time)
            .field("subtree", &self.subtree.as_ref().map(Vec::len))
            .field("name", &self.name)
            .field("pattern", &self.pattern.is_some())
            .finish()
    }
}

impl Filter {
    /// Whether any part of the filter is set.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.kinds.is_none_or(KindSet::is_all)
            && self.root.is_none()
            && self.time == TimeRange::default()
            && self.subtree.is_none()
            && self.name.is_none()
            && self.pattern.is_none()
    }
}

/// A request for one page.
#[derive(Debug, Clone)]
pub struct PageRequest {
    /// The generation the answer is for.
    pub generation: u64,
    /// The filter.
    pub filter: Filter,
    /// The id to page from; `None` starts at the newest (or oldest, for
    /// [`Direction::Newer`]) row.
    pub cursor: Option<i64>,
    /// Which way.
    pub direction: Direction,
    /// How many rows at most.
    pub limit: usize,
}

impl PageRequest {
    /// The first page: the newest rows.
    #[must_use]
    pub fn newest(generation: u64, filter: Filter) -> Self {
        Self {
            generation,
            filter,
            cursor: None,
            direction: Direction::Older,
            limit: PAGE_SIZE,
        }
    }
}

/// One row of a page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The event id.
    pub id: i64,
    /// The recorded wall clock, FILETIME ticks.
    pub ts: u64,
    /// The kind.
    pub kind: Kind,
    /// The source.
    pub source: Source,
    /// The raw OS value.
    pub raw_mask: u32,
    /// Whether the subject is a directory.
    pub is_dir: bool,
    /// The size, when known.
    pub size: Option<u64>,
    /// The attributes, when known.
    pub attrs: Option<u32>,
    /// The 128-bit file id (0 when unknown).
    pub file_id: u128,
    /// The name (last component), exact UTF-16: the sidecar when the name is
    /// not valid Unicode, so the viewer can apply F17 escaping.
    pub name: Vec<u16>,
    /// The full path from the volume root, components joined by `\`.
    pub path: Vec<u16>,
}

/// The answer to a [`PageRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Page {
    /// The generation of the request.
    pub generation: u64,
    /// The rows, newest first (display order), whichever direction was asked.
    pub rows: Vec<Row>,
    /// There are more rows in the direction that was asked.
    pub more: bool,
    /// Rows looked at, for the progress of a post-filter ("scanned X").
    pub scanned: u64,
    /// The scan stopped at its step limit before finishing: ask again with
    /// this cursor to continue ([`Page::resume`]).
    pub resume: Option<i64>,
    /// The scan was cancelled.
    pub cancelled: bool,
    /// Segments that were retired while paging. The viewer re-reads the
    /// catalog and re-syncs; it is not an error.
    pub retired: Vec<String>,
}

impl Page {
    /// The id of the first (newest) row.
    #[must_use]
    pub fn newest_id(&self) -> Option<i64> {
        self.rows.first().map(|row| row.id)
    }

    /// The id of the last (oldest) row.
    #[must_use]
    pub fn oldest_id(&self) -> Option<i64> {
        self.rows.last().map(|row| row.id)
    }
}

/// A flag the viewer sets to stop a request that is running.
#[derive(Debug, Clone, Default)]
pub struct CancelFlag(Arc<AtomicBool>);

impl CancelFlag {
    /// A flag that is not set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Asks the request to stop at its next step.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    /// Whether it was asked to stop.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// The text a user typed, as a literal for `LIKE ... ESCAPE '\'`: `%`, `_`
/// and `\` are escaped, nothing is interpolated into SQL.
#[must_use]
pub fn like_literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('%');
    for ch in text.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('%');
    out
}

/// The text as an FTS5 phrase: double quotes doubled, the whole in quotes.
#[must_use]
pub fn fts_phrase(text: &str) -> String {
    format!("\"{}\"", text.replace('"', "\"\""))
}

/// Whether `text` is long enough for the trigram index (three characters).
#[must_use]
pub fn is_indexable(text: &str) -> bool {
    text.chars().count() >= 3
}

#[cfg(test)]
mod tests;
