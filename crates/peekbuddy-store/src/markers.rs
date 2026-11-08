//! Markers (#82): named points in the event stream.
//!
//! A marker is an ordinary event row of kind `marker` (its name is the row's
//! path, so every view shows it where it happened) plus one `marker` row that
//! keeps the current name. The event row never changes; renaming updates the
//! `marker` row only, and only in the open segment: a sealed segment is never
//! rewritten.
//!
//! Markers live in the day segment of their event and disappear with it, so
//! [`Marker::segment`] and [`Marker::expires_with`] let the viewer warn before
//! the segment goes.

use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, params};

use crate::catalog::{Catalog, CatalogError};
use crate::segment::{DayKey, parse_name};

/// The longest marker name, in characters.
pub const MAX_NAME_CHARS: usize = 120;

/// A marker as the viewer lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marker {
    /// The id of the marker's event row: "since this marker" means rows with
    /// a greater id.
    pub event_id: i64,
    /// The current name.
    pub name: String,
    /// When it was made, FILETIME ticks.
    pub created_ts: i64,
    /// The segment file that holds it.
    pub segment: String,
    /// Whether that segment is the one still being written.
    pub open_segment: bool,
}

impl Marker {
    /// Whether the marker disappears when `segment` is deleted.
    #[must_use]
    pub fn expires_with(&self, segment: &str) -> bool {
        self.segment == segment
    }
}

/// A name the way a marker keeps it: trimmed, path separators and control
/// characters replaced by a space, cut to [`MAX_NAME_CHARS`]; `None` when
/// nothing is left. The event row interns the name as a path, so a `\` would
/// split it.
#[must_use]
pub fn clean_name(name: &str) -> Option<String> {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c == '\\' || c == '/' || c.is_control() {
                ' '
            } else {
                c
            }
        })
        .collect();
    let cleaned: String = cleaned.trim().chars().take(MAX_NAME_CHARS).collect();
    let cleaned = cleaned.trim_end().to_owned();
    (!cleaned.is_empty()).then_some(cleaned)
}

/// Days since 1970-01-01 of a civil date (Howard Hinnant's algorithm).
fn day_number(day: DayKey) -> i64 {
    let year = i64::from(day.year) - i64::from(day.month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let month = i64::from(day.month);
    let day_of_year =
        (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + i64::from(day.day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// How many days are left before retention removes `segment` (a file name),
/// given today and the age limit; `None` when there is no age limit or the
/// name is not a segment. Zero or less means it is due.
#[must_use]
pub fn days_left(segment: &str, today: DayKey, max_age_days: Option<u32>) -> Option<i64> {
    let limit = i64::from(max_age_days?);
    let (day, _) = parse_name(segment)?;
    Some(limit - (day_number(today) - day_number(day)))
}

/// Whether the viewer should warn that a marker is about to go: its segment
/// has at most [`WARN_DAYS`] days left.
#[must_use]
pub fn expires_soon(segment: &str, today: DayKey, max_age_days: Option<u32>) -> bool {
    days_left(segment, today, max_age_days).is_some_and(|left| left <= WARN_DAYS)
}

/// The warning starts this many days before a segment is due.
pub const WARN_DAYS: i64 = 2;

/// Every marker in the segments of `dir`, oldest first.
///
/// A segment written before schema 2 has no markers; one that cannot be
/// opened is skipped (retention may just have removed it).
///
/// # Errors
///
/// The catalog's error.
pub fn list(dir: &Path) -> Result<Vec<Marker>, CatalogError> {
    let catalog = Catalog::open_for_reading(&dir.join("catalog.db"))?;
    let mut out = Vec::new();
    for segment in catalog.list()? {
        let path = dir.join(&segment.file);
        let Ok(conn) = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY) else {
            continue;
        };
        let Ok(mut statement) =
            conn.prepare("SELECT event_id, name, created_ts FROM marker ORDER BY event_id")
        else {
            continue;
        };
        let rows = statement.query_map([], |row| {
            Ok(Marker {
                event_id: row.get(0)?,
                name: row.get(1)?,
                created_ts: row.get(2)?,
                segment: segment.file.clone(),
                open_segment: !segment.sealed,
            })
        });
        if let Ok(rows) = rows {
            out.extend(rows.flatten());
        }
    }
    Ok(out)
}

/// Renames the marker whose event is `event_id`, in the segment still being
/// written. `false` when there is no such marker there (it is in a sealed
/// segment, or the name is empty after cleaning).
///
/// This opens its own connection for one short statement and waits briefly
/// for the writer's lock; it never touches event rows.
///
/// # Errors
///
/// The catalog's error, or SQLite's.
pub fn rename(dir: &Path, event_id: i64, name: &str) -> Result<bool, CatalogError> {
    let Some(name) = clean_name(name) else {
        return Ok(false);
    };
    let catalog = Catalog::open_for_reading(&dir.join("catalog.db"))?;
    for segment in catalog.list()?.into_iter().filter(|s| !s.sealed) {
        let Ok(conn) = Connection::open(dir.join(&segment.file)) else {
            continue;
        };
        conn.busy_timeout(Duration::from_secs(5))?;
        let changed = conn.execute(
            "UPDATE marker SET name = ?2 WHERE event_id = ?1",
            params![event_id, name],
        )?;
        if changed > 0 {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::{clean_name, days_left, expires_soon};
    use crate::segment::DayKey;

    fn day(year: i32, month: u8, day: u8) -> DayKey {
        DayKey { year, month, day }
    }

    #[test]
    fn days_left_counts_calendar_days_across_months_and_leap_years() {
        let file = "events-2026-02-27.db";
        assert_eq!(days_left(file, day(2026, 2, 27), Some(30)), Some(30));
        assert_eq!(days_left(file, day(2026, 3, 1), Some(30)), Some(28));
        assert_eq!(days_left(file, day(2026, 3, 29), Some(30)), Some(0));
        // 2028 is a leap year: February has 29 days.
        assert_eq!(
            days_left("events-2028-02-27.db", day(2028, 3, 1), Some(30)),
            Some(27)
        );
        assert_eq!(days_left(file, day(2025, 12, 31), Some(30)), Some(30 + 58));
        assert_eq!(days_left(file, day(2026, 3, 1), None), None);
        assert_eq!(days_left("catalog.db", day(2026, 3, 1), Some(30)), None);
    }

    #[test]
    fn the_warning_starts_two_days_before_the_segment_is_due() {
        let file = "events-2026-10-01.db";
        assert!(!expires_soon(file, day(2026, 10, 28), Some(30)));
        assert!(expires_soon(file, day(2026, 10, 29), Some(30)));
        assert!(expires_soon(file, day(2026, 11, 5), Some(30)));
        assert!(!expires_soon(file, day(2026, 11, 5), None));
    }

    #[test]
    fn names_are_trimmed_and_cannot_split_a_path() {
        assert_eq!(
            clean_name("  before install  ").as_deref(),
            Some("before install")
        );
        assert_eq!(clean_name("a\\b/c\nd").as_deref(), Some("a b c d"));
        assert_eq!(clean_name("   "), None);
        assert_eq!(clean_name("\\\t"), None);
    }

    #[test]
    fn names_are_cut_at_the_limit_without_trailing_space() {
        let long = "x".repeat(300);
        assert_eq!(
            clean_name(&long).map(|n| n.chars().count()),
            Some(super::MAX_NAME_CHARS)
        );
        let spaced = format!("{} tail", "y".repeat(super::MAX_NAME_CHARS - 1));
        let cut = clean_name(&spaced).unwrap_or_default();
        assert!(!cut.ends_with(' '));
        assert_eq!(cut.chars().count(), super::MAX_NAME_CHARS - 1);
    }
}
