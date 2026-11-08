//! The numbers behind the stats panel (#82): events per hour and root from
//! the `stats` table, and the busiest paths and extensions of an hour from
//! the `churn` sketches. Neither counts rows of the log: both are tables the
//! writer keeps as it commits, so the cost does not grow with the log.

use std::collections::{BTreeMap, HashMap};

use peekbuddy_core::name::fold_upper;
use rusqlite::{Connection, OpenFlags};

use super::{PathCache, Paths, Reader};
use crate::catalog::{Catalog, CatalogError};
use crate::churn::{DIM_EXT, DIM_PATH};
use crate::names::from_columns;
use crate::schema::TICKS_PER_HOUR;

/// Events recorded in one hour of one watch root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HourlyCount {
    /// The hour, FILETIME ticks at its start.
    pub hour_ts: i64,
    /// The watch root, as stored.
    pub root: String,
    /// Rows of the log in that hour and root (system rows excluded).
    pub events: u64,
}

/// One busy path or extension, counts approximate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// The path, or the extension upper-cased.
    pub label: String,
    /// How often it changed, at most `error` too high.
    pub count: u64,
    /// How much of the count may belong to others.
    pub error: u64,
}

/// The busiest paths and extensions of one hour and root.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HourChurn {
    /// The hour.
    pub hour_ts: i64,
    /// The watch root, as stored.
    pub root: String,
    /// Paths, busiest first.
    pub paths: Vec<Hit>,
    /// Extensions, busiest first.
    pub exts: Vec<Hit>,
}

/// (root, folded label) to (label, count, error), for one dimension.
type Merged = HashMap<(String, String), (String, u64, u64)>;

/// The upper-cased form of a path, to merge the same path across segments.
fn folded(path: &str) -> String {
    path.encode_utf16()
        .map(fold_upper)
        .map(|unit| char::from_u32(u32::from(unit)).unwrap_or('\u{FFFD}'))
        .collect()
}

fn root_text(conn: &Connection, id: i64) -> String {
    conn.query_row("SELECT path, path_w FROM root WHERE id = ?1", [id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Option<Vec<u8>>>(1)?))
    })
    .ok()
    .and_then(|(text, wide)| from_columns(&text, wide.as_deref()))
    .map(|name| String::from_utf16_lossy(&name.into_units()))
    .unwrap_or_default()
}

impl Reader {
    /// Events per hour and root since `from_hour` (FILETIME ticks), oldest
    /// first. A segment from before the table existed, or one that cannot be
    /// opened, adds nothing.
    ///
    /// # Errors
    ///
    /// The catalog's error.
    pub fn hourly_counts(&self, from_hour: i64) -> Result<Vec<HourlyCount>, CatalogError> {
        let catalog = Catalog::open_for_reading(&self.dir.join("catalog.db"))?;
        let mut sums: BTreeMap<(i64, String), u64> = BTreeMap::new();
        for segment in catalog.list()? {
            if segment.last_ts != 0 && segment.last_ts < from_hour {
                continue;
            }
            let Ok(conn) = Connection::open_with_flags(
                self.dir.join(&segment.file),
                OpenFlags::SQLITE_OPEN_READ_ONLY,
            ) else {
                continue;
            };
            let Ok(mut statement) = conn.prepare(
                "SELECT hour_ts, root_id, sum(n) FROM stats \
                 WHERE hour_ts >= ?1 AND root_id <> 0 GROUP BY hour_ts, root_id",
            ) else {
                continue;
            };
            let found: Vec<(i64, i64, i64)> = statement
                .query_map([from_hour], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })
                .map(|rows| rows.flatten().collect())
                .unwrap_or_default();
            for (hour, root_id, n) in found {
                let root = root_text(&conn, root_id);
                *sums.entry((hour, root)).or_insert(0) += u64::try_from(n).unwrap_or(0);
            }
        }
        Ok(sums
            .into_iter()
            .map(|((hour_ts, root), events)| HourlyCount {
                hour_ts,
                root,
                events,
            })
            .collect())
    }

    /// The busiest paths and extensions of the hour starting at `hour_ts`,
    /// per root, at most `limit` of each. The same hour in two day files
    /// (a time zone that is not a whole hour from UTC) is merged.
    ///
    /// # Errors
    ///
    /// The catalog's error.
    pub fn hour_churn(&self, hour_ts: i64, limit: usize) -> Result<Vec<HourChurn>, CatalogError> {
        let catalog = Catalog::open_for_reading(&self.dir.join("catalog.db"))?;
        let mut paths: Merged = HashMap::new();
        let mut exts: Merged = HashMap::new();
        for segment in catalog.list()? {
            if segment.last_ts != 0 && segment.last_ts < hour_ts {
                continue;
            }
            if segment.first_ts != 0 && segment.first_ts >= hour_ts + TICKS_PER_HOUR {
                continue;
            }
            let Ok(conn) = Connection::open_with_flags(
                self.dir.join(&segment.file),
                OpenFlags::SQLITE_OPEN_READ_ONLY,
            ) else {
                continue;
            };
            let Ok(mut statement) = conn
                .prepare("SELECT root_id, dim, key, label, n, err FROM churn WHERE hour_ts = ?1")
            else {
                continue;
            };
            let found: Vec<(i64, i64, i64, Option<String>, i64, i64)> = statement
                .query_map([hour_ts], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                })
                .map(|rows| rows.flatten().collect())
                .unwrap_or_default();
            let mut resolver = Paths::with_cache(&conn, PathCache::default());
            for (root_id, dim, key, label, n, err) in found {
                let root = root_text(&conn, root_id);
                let (target, text) = match dim {
                    DIM_PATH => (
                        &mut paths,
                        String::from_utf16_lossy(&resolver.path_of(key).1),
                    ),
                    DIM_EXT => (&mut exts, label.unwrap_or_default()),
                    _ => continue,
                };
                let entry = target.entry((root, folded(&text))).or_insert((text, 0, 0));
                entry.1 += u64::try_from(n).unwrap_or(0);
                entry.2 += u64::try_from(err).unwrap_or(0);
            }
        }
        let mut out: BTreeMap<String, HourChurn> = BTreeMap::new();
        for (target, merged) in [(true, paths), (false, exts)] {
            for ((root, _), (label, count, error)) in merged {
                let churn = out.entry(root.clone()).or_insert_with(|| HourChurn {
                    hour_ts,
                    root,
                    ..HourChurn::default()
                });
                let hit = Hit {
                    label,
                    count,
                    error,
                };
                if target {
                    churn.paths.push(hit);
                } else {
                    churn.exts.push(hit);
                }
            }
        }
        let mut out: Vec<HourChurn> = out.into_values().collect();
        for churn in &mut out {
            for list in [&mut churn.paths, &mut churn.exts] {
                list.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.label.cmp(&b.label)));
                list.truncate(limit);
            }
        }
        Ok(out)
    }
}
