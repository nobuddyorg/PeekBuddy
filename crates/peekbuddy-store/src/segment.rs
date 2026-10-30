//! Day segments (#54): names, the day an event belongs to, and when to
//! rotate.
//!
//! One file per local day: `events-YYYY-MM-DD.db`, and
//! `events-YYYY-MM-DD.N.db` (`N` from 1) when the size cap forces another
//! rotation within a day. The date is for humans; ordering always uses the
//! event id. Rotation happens when an event of a later day arrives, or when
//! the file is over the cap: never on a schedule (F6).

use std::collections::VecDeque;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rusqlite::{Connection, OpenFlags};

use crate::catalog::{Catalog, Scan, SegmentRow, Skipped};

/// The local calendar day of an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DayKey {
    /// The year.
    pub year: i32,
    /// The month, 1 to 12.
    pub month: u8,
    /// The day of the month, 1 to 31.
    pub day: u8,
}

impl fmt::Display for DayKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

impl DayKey {
    /// The file name of the `extra`-th segment of the day (0 is the first).
    #[must_use]
    pub fn file_name(self, extra: u32) -> String {
        if extra == 0 {
            format!("events-{self}.db")
        } else {
            format!("events-{self}.{extra}.db")
        }
    }
}

/// The day of a timestamp (FILETIME ticks, UTC) in the local time zone.
/// Injected, so tests choose the zone and the clock.
pub type DayFn = Arc<dyn Fn(u64) -> DayKey + Send + Sync>;

/// The day segment a file name stands for: its date and its extra number.
#[must_use]
pub fn parse_name(name: &str) -> Option<(DayKey, u32)> {
    if !crate::catalog::is_segment_name(name) {
        return None;
    }
    let rest = name.strip_prefix("events-")?.strip_suffix(".db")?;
    let (date, extra) = rest.split_once('.').map_or((rest, "0"), |(d, n)| (d, n));
    let year = date.get(0..4)?.parse().ok()?;
    let month = date.get(5..7)?.parse().ok()?;
    let day = date.get(8..10)?.parse().ok()?;
    Some((DayKey { year, month, day }, extra.parse().ok()?))
}

/// What the writer needs to rotate.
#[derive(Clone)]
pub struct Rotation {
    /// Where the segments and `catalog.db` live.
    pub dir: PathBuf,
    /// The local day of a timestamp.
    pub day_of: DayFn,
    /// A segment bigger than this (file and log) is rotated, bytes.
    pub size_cap: u64,
}

impl fmt::Debug for Rotation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Rotation")
            .field("dir", &self.dir)
            .field("size_cap", &self.size_cap)
            .finish_non_exhaustive()
    }
}

/// The default size cap of one segment: 1 GiB.
pub const DEFAULT_SIZE_CAP: u64 = 1024 * 1024 * 1024;

/// The file name that follows `current` when `event_day` is not its day:
/// the first file of the new day.
#[must_use]
pub fn next_file(current: &str, event_day: DayKey, existing: &[SegmentRow]) -> String {
    let current_day = parse_name(current).map(|(day, _)| day);
    if current_day == Some(event_day) {
        // Same day, so this is the size cap: the next free number.
        let used = existing
            .iter()
            .filter_map(|row| parse_name(&row.file))
            .filter(|(day, _)| *day == event_day)
            .map(|(_, n)| n)
            .max()
            .unwrap_or(0);
        return event_day.file_name(used + 1);
    }
    // A new day. If its first file exists (the clock moved back, the zone
    // changed), take the next free number so no sealed file is reopened.
    let used = existing
        .iter()
        .filter_map(|row| parse_name(&row.file))
        .filter(|(day, _)| *day == event_day)
        .map(|(_, n)| n)
        .max();
    used.map_or_else(|| event_day.file_name(0), |n| event_day.file_name(n + 1))
}

/// Whether an event at `ts` belongs in the segment `current` (of day
/// `current_day`) of `size` bytes: it does unless the day moved on or the
/// file is over the cap. A day that is *earlier* than the segment's (the
/// clock moved backwards) stays in the segment: ids and order are what count.
#[must_use]
pub fn needs_rotation(current_day: DayKey, event_day: DayKey, size: u64, cap: u64) -> bool {
    event_day > current_day || size >= cap
}

/// The catalog rows for the files found by a scan, with the id ranges of
/// empty segments filled in from their neighbours, and everything that is not
/// the newest marked sealed.
#[must_use]
pub fn rows_from_scan(scan: &Scan, first_unused_id: i64) -> Vec<SegmentRow> {
    let mut rows = scan.segments.clone();
    let mut next = first_unused_id.max(1);
    let last = rows.len().saturating_sub(1);
    for (index, row) in rows.iter_mut().enumerate() {
        if row.rows == 0 {
            row.first_id = next;
            row.last_id = next - 1;
        } else {
            next = row.last_id + 1;
        }
        row.sealed = index != last;
    }
    rows
}

/// Brings the catalog in line with the files of `dir` and returns the
/// segments, oldest first, and the files that were skipped.
///
/// # Errors
///
/// An I/O error reading the directory, or SQLite's error.
pub fn reconcile_catalog(
    dir: &Path,
    catalog: &Catalog,
) -> Result<(Vec<SegmentRow>, Vec<Skipped>), Box<dyn std::error::Error>> {
    let scan = crate::catalog::scan(dir)?;
    let known = catalog.list()?;
    for row in &known {
        if !dir.join(&row.file).exists() {
            catalog.remove(&row.file)?;
        }
    }
    let rows = rows_from_scan(&scan, 1);
    for row in &rows {
        let mut row = row.clone();
        if let Some(old) = known.iter().find(|k| k.file == row.file) {
            row.purge_pending = old.purge_pending;
            row.sealed |= old.sealed;
        }
        catalog.put(&row)?;
    }
    Ok((catalog.list()?, scan.skipped))
}

/// What the next step of a [`NewestFirst`] walk is.
#[derive(Debug)]
pub enum Step {
    /// A segment, opened read-only.
    Segment(Box<SegmentRow>, Connection),
    /// The segment was retired (deleted) since the catalog was read: the
    /// walk goes on with the next one and the caller shows "older data was
    /// removed".
    Retired(String),
}

/// A newest-first walk over the segments for a reader (#55): one read-only
/// connection at a time, never `ATTACH`. A segment that disappears between
/// the catalog read and the open (retention) is a [`Step::Retired`], not an
/// error and not a panic.
#[derive(Debug)]
pub struct NewestFirst {
    dir: PathBuf,
    rows: VecDeque<SegmentRow>,
}

impl NewestFirst {
    /// Walks the segments of `catalog` from the newest.
    ///
    /// # Errors
    ///
    /// SQLite's error from reading the catalog.
    pub fn new(dir: &Path, catalog: &Catalog) -> rusqlite::Result<Self> {
        Ok(Self {
            dir: dir.to_path_buf(),
            rows: catalog.newest_first()?.into(),
        })
    }

    /// The next segment, newest first; `None` at the end.
    pub fn next_segment(&mut self) -> Option<Step> {
        let row = self.rows.pop_front()?;
        let path = self.dir.join(&row.file);
        match Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY) {
            Ok(conn) if path.exists() => Some(Step::Segment(Box::new(row), conn)),
            _ => Some(Step::Retired(row.file)),
        }
    }
}

#[cfg(test)]
mod tests;
