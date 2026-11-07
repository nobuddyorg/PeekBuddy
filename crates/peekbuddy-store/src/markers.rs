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

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};

use crate::catalog::{Catalog, CatalogError};
use crate::writer::{WriteError, Writer};

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

impl Writer {
    /// Renames the marker whose event is `event_id` in the open segment.
    /// `false` when there is no such marker here (it is in a sealed segment,
    /// or the name is empty after cleaning).
    ///
    /// # Errors
    ///
    /// SQLite's error.
    pub fn rename_marker(&self, event_id: i64, name: &str) -> Result<bool, WriteError> {
        let Some(name) = clean_name(name) else {
            return Ok(false);
        };
        let changed = self.connection().execute(
            "UPDATE marker SET name = ?2 WHERE event_id = ?1",
            params![event_id, name],
        )?;
        Ok(changed > 0)
    }

    /// The marker with this event id in the open segment.
    ///
    /// # Errors
    ///
    /// SQLite's error.
    pub fn marker_name(&self, event_id: i64) -> Result<Option<String>, WriteError> {
        Ok(self
            .connection()
            .query_row(
                "SELECT name FROM marker WHERE event_id = ?1",
                [event_id],
                |row| row.get(0),
            )
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use super::clean_name;

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
