//! The rows the grid holds (#66): a window onto a log of millions of rows.
//!
//! Pure data, no egui and no I/O, so every rule here is a plain unit test and
//! a property test.
//!
//! # The model
//!
//! The grid shows rows **newest first**; a row's *virtual index* is its place
//! in that order (0 is the newest). The log is far too big to hold, so the
//! timeline keeps one contiguous **island** of rows (`rows`, strictly
//! decreasing ids) and the virtual index of its first row (`top`). Everything
//! outside the island is a placeholder until a fetch fills it in.
//!
//! The database is read by keyset, never by offset, so a page is always "the
//! rows next to an id". [`Timeline::plan`] says which fetch the visible range
//! needs:
//!
//! | Situation | Fetch |
//! | --- | --- |
//! | the view reaches the island's old edge | [`Fetch::Older`] from the oldest row |
//! | the view reaches the island's new edge | [`Fetch::Newer`] from the newest row |
//! | the view is far from the island (scrollbar drag, End key) | [`Fetch::Seek`] to a virtual index |
//! | the live tail has rows | [`Timeline::ingest_tail`], fed by the worker's own polls |
//!
//! A seek turns the virtual index into an id by arithmetic: event ids are one
//! contiguous sequence (R2), so with no filter row `i` has id `newest - i`.
//! With a filter that hides rows the index is an estimate and the island lands
//! near it; the rows in the island are always right, only their index is not.
//!
//! # Rules that make it safe
//!
//! * Ids in `rows` are strictly decreasing: a duplicate or an out-of-order row
//!   is dropped, never shown twice.
//! * At most [`MAX_ROWS`] rows are held, however far the user scrolls: the end
//!   of the island farther from the view is dropped.
//! * A result that answers a question nobody asks any more (the filter
//!   changed, the island moved) is ignored: every request carries its cursor
//!   and is matched against what is pending.
//! * While the user is away from the newest rows, new rows are only counted
//!   (the "N new events" pill): the island does not move under the pointer.

use std::collections::VecDeque;
use std::ops::Range;

/// Rows in one fetch.
pub const PAGE: usize = peekbuddy_core::query::PAGE_SIZE;
/// Pages the island may span; 64 pages are 12,800 rows.
pub const MAX_PAGES: usize = 64;
/// Rows the island may hold.
pub const MAX_ROWS: usize = PAGE * MAX_PAGES;
/// How close the view may come to an edge of the island before the next page
/// is fetched.
pub const PREFETCH: u64 = 80;
/// How far from the island the view may be before it seeks instead of
/// extending: four pages.
pub const FAR: u64 = (PAGE as u64) * 4;
/// Rows of slack below the island's old edge in a view whose size is unknown,
/// so the scrollbar can still be dragged a little further.
pub const SLACK: u64 = PAGE as u64;

/// A row that has an id.
pub trait Identified {
    /// The event id.
    fn id(&self) -> i64;
}

/// What to ask the database for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fetch {
    /// The page of rows with `id < cursor`.
    Older {
        /// The oldest id held.
        cursor: i64,
    },
    /// The page of rows with `id > cursor`, nearest first.
    Newer {
        /// The newest id held.
        cursor: i64,
    },
    /// The page that starts at about virtual row `index`.
    Seek {
        /// The virtual index to land on.
        index: u64,
        /// The keyset cursor the index turned into: rows with `id < cursor`;
        /// `None` for the newest page.
        cursor: Option<i64>,
    },
}

/// What a fetch brought.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched<R> {
    /// The rows, newest first.
    pub rows: Vec<R>,
    /// There are more rows beyond these in the direction asked.
    pub more: bool,
}

/// What became of a batch from the live tail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tail {
    /// The rows joined the island (it was at the newest rows).
    Merged(u64),
    /// The island is away from the newest rows: the rows are only counted.
    Counted(u64),
    /// The batch skipped ids; it was refused. Ask again from
    /// [`Timeline::tail_cursor`].
    Hole,
    /// Nothing new.
    Nothing,
}

/// What the store knows about its size: it comes from the catalog, not from
/// counting rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Extent {
    /// The newest event id.
    pub newest: i64,
    /// The oldest event id still stored.
    pub oldest: i64,
}

impl Extent {
    /// How many ids lie between the two (the row count of an unfiltered view).
    #[must_use]
    pub fn span(&self) -> u64 {
        u64::try_from(self.newest.saturating_sub(self.oldest).saturating_add(1)).unwrap_or(0)
    }
}

/// The window of rows.
#[derive(Debug)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "four independent facts about the view: seekable, exact, and which ends are held"
)]
pub struct Timeline<R> {
    rows: VecDeque<R>,
    /// Virtual index of `rows[0]`.
    top: u64,
    /// Rows in the view: exact when `seekable`, else what is known plus slack.
    total: u64,
    extent: Option<Extent>,
    /// The view is the whole log (maybe minus hidden rows): the index of a
    /// row follows from its id, so the scrollbar can be dragged anywhere.
    seekable: bool,
    /// The view's size is exactly the id span (nothing hidden, nothing
    /// filtered): a tail batch must continue the island without a hole.
    exact: bool,
    at_newest: bool,
    at_oldest: bool,
    /// The newest id of the view seen so far, by any fetch.
    tail_cursor: Option<i64>,
    /// Rows that arrived while the island was away from the newest rows.
    unseen: u64,
    /// Rows the filter hides that the id span still counts.
    hidden: u64,
    pending_older: Option<i64>,
    pending_newer: Option<i64>,
    pending_seek: Option<u64>,
    /// Counts every change to the held rows or their place, so a view
    /// derived from them (the compact folds) knows when to be made again.
    revision: u64,
}

impl<R: Identified> Timeline<R> {
    /// An empty timeline for a fresh view. `seekable` views know their size
    /// (`extent`); `exact` ones are exactly as big as the id span.
    #[must_use]
    pub fn new(extent: Option<Extent>, seekable: bool, exact: bool) -> Self {
        let total = match (&extent, seekable) {
            (Some(extent), true) => extent.span(),
            _ => 0,
        };
        Self {
            rows: VecDeque::new(),
            top: 0,
            total,
            extent,
            seekable,
            exact: exact && seekable,
            at_newest: false,
            at_oldest: false,
            tail_cursor: None,
            unseen: 0,
            hidden: 0,
            pending_older: None,
            pending_newer: None,
            pending_seek: None,
            revision: 0,
        }
    }

    /// Changes whenever the held rows or the virtual index of the first one
    /// change.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Rows held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether no row is held.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Rows in the view (what the scrollbar spans).
    #[must_use]
    pub const fn total(&self) -> u64 {
        self.total
    }

    /// Tells the timeline how many rows of the id span the filter hides, so
    /// the scrollbar spans what can be shown. Only a seekable view counts
    /// them; the number is exact when it arrives and drifts as rows come.
    pub fn note_hidden(&mut self, hidden: u64) {
        if !self.seekable {
            return;
        }
        self.total = self
            .total
            .saturating_add(self.hidden)
            .saturating_sub(hidden);
        self.hidden = hidden;
    }

    /// Whether `total` is an estimate (a filtered view that is still being
    /// read, or a view with hidden rows).
    #[must_use]
    pub const fn is_estimate(&self) -> bool {
        !self.exact
    }

    /// Whether the island starts at the newest row.
    #[must_use]
    pub const fn at_newest(&self) -> bool {
        self.at_newest
    }

    /// Whether the island ends at the oldest row.
    #[must_use]
    pub const fn at_oldest(&self) -> bool {
        self.at_oldest
    }

    /// Whether a fetch is out: a scan may be running.
    #[must_use]
    pub const fn busy(&self) -> bool {
        self.pending_older.is_some() || self.pending_newer.is_some() || self.pending_seek.is_some()
    }

    /// The newest and oldest id of the log, when it is known.
    #[must_use]
    pub const fn extent(&self) -> Option<Extent> {
        self.extent
    }

    /// Forgets the fetches that are out, because their jobs were cancelled and
    /// will never answer; the view asks again when it needs more.
    pub fn abandon_pending(&mut self) {
        self.pending_older = None;
        self.pending_newer = None;
        self.pending_seek = None;
    }

    /// Rows that arrived while the user was away from the newest rows.
    #[must_use]
    pub const fn unseen(&self) -> u64 {
        self.unseen
    }

    /// Virtual index of the first held row.
    #[must_use]
    pub const fn top(&self) -> u64 {
        self.top
    }

    /// The row at virtual index `index`, if it is held.
    #[must_use]
    pub fn row_at(&self, index: u64) -> Option<&R> {
        let offset = usize::try_from(index.checked_sub(self.top)?).ok()?;
        self.rows.get(offset)
    }

    /// Marks the log as having no row yet: the view is complete and empty,
    /// and the live tail brings the first rows.
    pub fn empty_log(&mut self) {
        self.revision += 1;
        self.rows.clear();
        self.top = 0;
        self.total = 0;
        self.at_newest = true;
        self.at_oldest = true;
    }

    /// The virtual index of the row with `id`, if it is held.
    #[must_use]
    pub fn index_of(&self, id: i64) -> Option<u64> {
        let position = self.rows.iter().position(|row| row.id() == id)?;
        Some(self.top + u64::try_from(position).ok()?)
    }

    /// The held rows, newest first.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &R> + Clone {
        self.rows.iter()
    }

    fn recompute_total(&mut self) {
        if self.seekable {
            return;
        }
        let held = self.top + u64::try_from(self.rows.len()).unwrap_or(u64::MAX);
        self.total = held + if self.at_oldest { 0 } else { SLACK };
    }

    /// The keyset cursor for virtual row `index`.
    fn cursor_for(&self, index: u64) -> Option<i64> {
        if index == 0 {
            return None;
        }
        let extent = self.extent?;
        let span = u128::from(extent.span());
        let total = u128::from(self.total.max(1));
        let down = u128::from(index).saturating_mul(span) / total;
        let down = i64::try_from(down).unwrap_or(i64::MAX);
        let id = extent.newest.saturating_sub(down).max(extent.oldest);
        // Rows with `id < cursor`: the row with `id` is the first one.
        Some(id.saturating_add(1))
    }

    fn seek(&mut self, index: u64) -> Fetch {
        let index = index.min(self.total.saturating_sub(1));
        self.pending_seek = Some(index);
        Fetch::Seek {
            index,
            cursor: self.cursor_for(index),
        }
    }

    /// The fetches the view `visible` needs now (at most two). A fetch that is
    /// already out is not asked for again.
    pub fn plan(&mut self, visible: &Range<u64>) -> Vec<Fetch> {
        let mut out = Vec::new();
        if self.pending_seek.is_some() {
            return out;
        }
        // A view past the end (a log that shrank, a scrollbar dragged too
        // far) means the end.
        let start = visible.start.min(self.total.saturating_sub(1));
        let visible = &(start..visible.end.min(self.total).max(start));
        let Some(front) = self.rows.front().map(Identified::id) else {
            // Nothing held yet (a new view): ask for where the view is.
            let index = if self.seekable { visible.start } else { 0 };
            out.push(self.seek(index));
            return out;
        };
        let held = u64::try_from(self.rows.len()).unwrap_or(u64::MAX);
        let end = self.top + held;
        let far =
            visible.start >= end.saturating_add(FAR) || visible.end.saturating_add(FAR) <= self.top;
        if far && self.seekable {
            out.push(self.seek(visible.start));
            return out;
        }
        if let Some(back) = self.rows.back().map(Identified::id)
            && !self.at_oldest
            && self.pending_older.is_none()
            && visible.end.saturating_add(PREFETCH) > end
        {
            self.pending_older = Some(back);
            out.push(Fetch::Older { cursor: back });
        }
        if !self.at_newest
            && self.pending_newer.is_none()
            && visible.start < self.top.saturating_add(PREFETCH)
        {
            self.pending_newer = Some(front);
            out.push(Fetch::Newer { cursor: front });
        }
        out
    }

    /// The newest id of the view seen so far: where the live tail goes on.
    #[must_use]
    pub fn tail_cursor(&self) -> Option<i64> {
        self.tail_cursor
            .or_else(|| self.rows.front().map(Identified::id))
    }

    /// Goes back to the newest rows: the "N new events" pill.
    pub fn jump_to_newest(&mut self) -> Fetch {
        self.unseen = 0;
        self.pending_older = None;
        self.pending_newer = None;
        self.seek(0)
    }

    /// Goes to the oldest rows: the End key. Only a seekable view knows where
    /// they are.
    pub fn jump_to_oldest(&mut self) -> Option<Fetch> {
        if !self.seekable {
            return None;
        }
        Some(self.seek(self.total.saturating_sub(1)))
    }

    /// Forgets what a failed fetch had marked as pending, so the view asks
    /// again.
    pub fn failed(&mut self, fetch: &Fetch) {
        match fetch {
            Fetch::Older { .. } => self.pending_older = None,
            Fetch::Newer { .. } => self.pending_newer = None,
            Fetch::Seek { .. } => self.pending_seek = None,
        }
    }

    /// Takes the answer to `fetch`. Returns whether the held rows or the
    /// counts changed (the view needs a repaint).
    pub fn apply(&mut self, fetch: &Fetch, fetched: Fetched<R>) -> bool {
        match *fetch {
            Fetch::Older { cursor } => self.apply_older(cursor, fetched),
            Fetch::Newer { cursor } => self.apply_newer(cursor, fetched),
            Fetch::Seek { index, cursor } => self.apply_seek(index, cursor, fetched),
        }
    }

    fn apply_older(&mut self, cursor: i64, fetched: Fetched<R>) -> bool {
        if self.pending_older != Some(cursor) {
            return false;
        }
        self.pending_older = None;
        let Some(back) = self.rows.back().map(Identified::id) else {
            return false;
        };
        if back != cursor {
            return false;
        }
        let mut changed = false;
        let mut last = back;
        for row in fetched.rows {
            if row.id() < last {
                last = row.id();
                self.rows.push_back(row);
                changed = true;
            }
        }
        if !fetched.more || !changed {
            self.at_oldest = true;
            changed = true;
        }
        self.recompute_total();
        self.revision += 1;
        changed
    }

    fn apply_newer(&mut self, cursor: i64, fetched: Fetched<R>) -> bool {
        if self.pending_newer != Some(cursor) {
            return false;
        }
        self.pending_newer = None;
        let Some(front) = self.rows.front().map(Identified::id) else {
            return false;
        };
        if front != cursor {
            return false;
        }
        let mut added = 0_u64;
        let mut floor = front;
        // Newest first: the last one is nearest to the island.
        let mut fresh: Vec<R> = Vec::new();
        for row in fetched.rows.into_iter().rev() {
            if row.id() > floor {
                floor = row.id();
                fresh.push(row);
            }
        }
        for row in fresh {
            self.rows.push_front(row);
            added += 1;
        }
        self.top = self.top.saturating_sub(added);
        if !fetched.more || added == 0 {
            // The estimate put the island too low: it starts at the top.
            let drift = self.top;
            self.top = 0;
            self.at_newest = true;
            if self.seekable {
                self.total = self.total.saturating_sub(drift);
            }
        }
        if let Some(front) = self.rows.front().map(Identified::id) {
            self.tail_cursor = Some(self.tail_cursor.map_or(front, |seen| seen.max(front)));
        }
        self.trim_far_from(0);
        self.recompute_total();
        self.revision += 1;
        true
    }

    /// Takes rows the live tail found: everything newer than the newest id
    /// seen. At the newest rows they join the island; away from them they
    /// are only counted, so the island does not move under the pointer.
    pub fn ingest_tail(&mut self, fetched: Fetched<R>) -> Tail {
        if self.pending_seek.is_some() {
            // The island is being replaced; the new one starts from the
            // database, which has these rows already.
            return Tail::Nothing;
        }
        // An empty view that has looked at the newest rows takes whatever
        // arrives; one that has not looked yet takes nothing.
        let cursor = match self.tail_cursor() {
            Some(cursor) => Some(cursor),
            None if self.at_newest => None,
            None => return Tail::Nothing,
        };
        let mut fresh: Vec<R> = Vec::new();
        let mut floor = cursor.unwrap_or(i64::MIN);
        for row in fetched.rows.into_iter().rev() {
            if row.id() > floor {
                floor = row.id();
                fresh.push(row);
            }
        }
        if fresh.is_empty() {
            return Tail::Nothing;
        }
        let count = u64::try_from(fresh.len()).unwrap_or(u64::MAX);
        if self.exact
            && let Some(cursor) = cursor
            && fresh.first().map(Identified::id) != Some(cursor.saturating_add(1))
        {
            // Ids are contiguous, so the batch has to continue the log. A
            // hole (a dropped frame) is refused; the caller asks again from
            // `tail_cursor`.
            return Tail::Hole;
        }
        self.tail_cursor = Some(floor);
        if self.seekable {
            self.total = self.total.saturating_add(count);
            if let Some(extent) = &mut self.extent {
                extent.newest = floor;
            }
        }
        if self.at_newest {
            self.revision += 1;
            for row in fresh {
                self.rows.push_front(row);
            }
            self.top = 0;
            self.trim_far_from(0);
            self.recompute_total();
            Tail::Merged(count)
        } else {
            self.unseen = self.unseen.saturating_add(count);
            Tail::Counted(count)
        }
    }

    fn apply_seek(&mut self, index: u64, cursor: Option<i64>, fetched: Fetched<R>) -> bool {
        if self.pending_seek != Some(index) {
            return false;
        }
        self.pending_seek = None;
        self.pending_older = None;
        self.pending_newer = None;
        let mut rows = VecDeque::new();
        let mut last = i64::MAX;
        for row in fetched.rows {
            if row.id() < last {
                last = row.id();
                rows.push_back(row);
            }
        }
        self.revision += 1;
        if rows.is_empty() {
            // Nothing there: the view is shorter than the index thought.
            self.rows.clear();
            self.top = 0;
            self.at_newest = cursor.is_none();
            self.at_oldest = true;
            self.unseen = 0;
            self.total = if cursor.is_none() {
                0
            } else {
                self.total.min(index)
            };
            return true;
        }
        self.rows = rows;
        self.top = index;
        self.at_newest = cursor.is_none();
        self.at_oldest = !fetched.more;
        if self.at_newest {
            self.top = 0;
            self.unseen = 0;
        }
        if let Some(front) = self.rows.front().map(Identified::id) {
            self.tail_cursor = Some(self.tail_cursor.map_or(front, |seen| seen.max(front)));
        }
        if self.at_oldest && self.seekable {
            // The island reaches the end: the view ends here.
            self.total = self.top + u64::try_from(self.rows.len()).unwrap_or(u64::MAX);
        }
        self.recompute_total();
        true
    }

    /// Drops rows from the end of the island that is farther from `center`
    /// (a virtual index), until at most [`MAX_ROWS`] are held.
    pub fn trim_far_from(&mut self, center: u64) {
        while self.rows.len() > MAX_ROWS {
            self.revision += 1;
            let held = u64::try_from(self.rows.len()).unwrap_or(u64::MAX);
            let end = self.top + held;
            // Distance of the view's center to each end of the island.
            let to_top = center.saturating_sub(self.top);
            let to_end = end.saturating_sub(center);
            if to_top > to_end {
                // Farther from the newest end: drop the newest rows.
                self.rows.pop_front();
                self.top += 1;
                self.at_newest = false;
            } else {
                self.rows.pop_back();
                self.at_oldest = false;
            }
        }
    }

    /// Keeps the island within [`MAX_ROWS`] around the view.
    pub fn keep_near(&mut self, visible: &Range<u64>) {
        let center = visible.start / 2 + visible.end / 2;
        self.trim_far_from(center);
        self.recompute_total();
    }
}

#[cfg(test)]
#[expect(
    clippy::panic,
    clippy::assert_is_empty,
    reason = "a test panics when the timeline keeps asking, and compares fetch lists to empty ones"
)]
mod tests;
