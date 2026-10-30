//! The running side of day segments (#54): rotation state, the system rows
//! rotation and retention produce, sealing, and the retention job.
//!
//! The writer owns a [`SegmentState`]. When an event of a later day arrives,
//! or the file is over its size cap, it closes the segment (final checkpoint),
//! opens the next file with the same roots registered, continues the id
//! sequence, writes a `SegmentRotated` row as the first row of the new file
//! and hands the old file to a background thread that seals it
//! ([`seal_and_enforce`]): `PRAGMA optimize`, mark it sealed in the catalog,
//! then apply the retention caps. All of it happens on rotation or at start,
//! never on a timer.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use peekbuddy_core::event::{Event, Flags, Kind, NameKind, NameRef, RootIdx, Source, VolIdx};
use rusqlite::Connection;

use crate::catalog::{Catalog, SegmentRow};
use crate::retention::{self, Caps, Outcome, Retire};
use crate::segment::{DayKey, Rotation};

/// A row the store writes about itself, with its name.
#[derive(Debug, Clone)]
pub struct SystemRow {
    /// The event.
    pub event: Event,
    /// Its name (a segment file name).
    pub name: Vec<u16>,
}

/// System rows waiting for the next commit; shared with the sealing thread.
pub type SystemRows = Arc<Mutex<Vec<SystemRow>>>;

/// What retention needs; cloned into the sealing thread.
#[derive(Clone)]
pub struct SealContext {
    /// The data directory of the segments.
    pub dir: PathBuf,
    /// The catalog.
    pub catalog: Arc<Mutex<Catalog>>,
    /// The caps.
    pub caps: Caps,
    /// The viewer's side of the handshake.
    pub retire: Arc<dyn Retire + Send + Sync>,
    /// Where the purge rows go.
    pub system: SystemRows,
}

impl std::fmt::Debug for SealContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SealContext")
            .field("dir", &self.dir)
            .field("caps", &self.caps)
            .finish_non_exhaustive()
    }
}

/// Everything rotation needs, owned by the writer.
pub struct SegmentState {
    /// Directory, day function and size cap.
    pub rotation: Rotation,
    /// Catalog, caps, handshake and the system row queue.
    pub context: SealContext,
    /// The file being written, by name.
    pub file: String,
    /// Its day.
    pub day: DayKey,
    /// The first id it holds.
    pub first_id: i64,
    /// Timestamp of its first event, 0 while empty.
    pub first_ts: i64,
    /// Timestamp of its last event.
    pub last_ts: i64,
    /// Sealing threads still running.
    pub sealers: Vec<JoinHandle<()>>,
}

impl std::fmt::Debug for SegmentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SegmentState")
            .field("file", &self.file)
            .field("day", &self.day)
            .field("first_id", &self.first_id)
            .finish_non_exhaustive()
    }
}

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn system_event(kind: Kind, ts: u64) -> Event {
    Event {
        ts,
        qpc: 0,
        src_ts: 0,
        source: Source::System,
        kind,
        raw_mask: 0,
        source_info: 0,
        is_dir: false,
        size: 0,
        attrs: 0,
        reparse_tag: 0,
        file_id: 0,
        parent_file_id: 0,
        usn: 0,
        vol: VolIdx(0),
        root: RootIdx::NONE,
        name: NameRef::EMPTY,
        name_kind: NameKind::Unknown,
        flags: Flags::NONE,
    }
}

/// The row that starts a new segment.
///
/// `usn` is the last id of the segment before, `size` its rows and
/// `file_id` its size in bytes; the name is the new file.
#[must_use]
pub fn rotated_row(ts: u64, previous: &SegmentRow, new_file: &str) -> SystemRow {
    let mut event = system_event(Kind::SegmentRotated, ts);
    event.usn = previous.last_id;
    event.size = previous.rows.cast_unsigned();
    event.file_id = u128::from(previous.bytes.cast_unsigned());
    SystemRow {
        event,
        name: units(new_file),
    }
}

/// The row for one purged segment.
///
/// `usn` is its first id, `size` its rows (so its last id is
/// `usn + size - 1`, segments being contiguous) and `file_id` its bytes; the
/// name is the file.
#[must_use]
pub fn purged_row(ts: u64, purged: &SegmentRow) -> SystemRow {
    let mut event = system_event(Kind::RetentionPurged, ts);
    event.usn = purged.first_id;
    event.size = purged.rows.cast_unsigned();
    event.file_id = u128::from(purged.bytes.cast_unsigned());
    SystemRow {
        event,
        name: units(&purged.file),
    }
}

impl SealContext {
    /// Applies the retention caps at `now` (FILETIME ticks): plans, purges
    /// through the handshake and queues one `RetentionPurged` row per deleted
    /// segment. Returns the outcomes.
    ///
    /// # Errors
    ///
    /// A poisoned lock, or SQLite's error from the catalog.
    pub fn enforce(&self, now: i64) -> Result<Vec<Outcome>, Box<dyn std::error::Error>> {
        let catalog = self
            .catalog
            .lock()
            .map_err(|_| "the catalog lock is poisoned")?;
        let segments = catalog.list()?;
        let files = retention::plan(&segments, &self.caps, now);
        let outcomes = retention::purge(&self.dir, &catalog, &files, self.retire.as_ref())?;
        drop(catalog);
        let mut rows = self
            .system
            .lock()
            .map_err(|_| "the system rows lock is poisoned")?;
        for outcome in &outcomes {
            if let Outcome::Purged(row) = outcome {
                rows.push(purged_row(now.cast_unsigned(), row));
            }
        }
        Ok(outcomes)
    }
}

/// The background half of a rotation: optimise the old file, mark it sealed,
/// enforce the retention caps.
pub fn seal_and_enforce(context: &SealContext, file: &str, now: i64) {
    let path = context.dir.join(file);
    if let Ok(conn) = Connection::open(&path) {
        let _ = conn.execute_batch("PRAGMA optimize;");
    }
    let bytes = std::fs::metadata(&path).map_or(0, |meta| i64::try_from(meta.len()).unwrap_or(0));
    if let Ok(catalog) = context.catalog.lock() {
        let _ = catalog.seal(file, bytes);
    }
    let _ = context.enforce(now);
}

#[cfg(test)]
mod tests;
