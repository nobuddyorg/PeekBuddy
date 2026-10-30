//! Retention (#54): whole oldest segments go, never rows.
//!
//! Caps come from the settings: a maximum age, a maximum total size and a
//! maximum number of rows. They are enforced after each rotation and at
//! start, never on a timer, by deleting the oldest **sealed** segment files.
//! The newest segment is never deleted, and neither is any unsealed one.
//! Every deletion is reported to the caller, which writes it as a
//! `RetentionPurged` row: nothing disappears silently.
//!
//! Before a file is deleted the viewer is told ([`Retire::announce`]) so it
//! can close its connections; a file that is still open (Windows will not
//! delete it) is *deferred*, remembered in the catalog and tried again at the
//! next rotation or start.

use std::io;
use std::path::Path;

use crate::catalog::{Catalog, SegmentRow};

/// 100 ns ticks per day.
const TICKS_PER_DAY: i64 = 864_000_000_000;

/// The caps; a `None` is no cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Caps {
    /// Segments whose newest event is older than this many days go.
    pub max_age_days: Option<u32>,
    /// Oldest segments go while the total size is over this, bytes.
    pub max_total_bytes: Option<u64>,
    /// Oldest segments go while the total rows are over this.
    pub max_rows: Option<u64>,
}

/// The defaults of a fresh install: 30 days and 2 GB.
pub const DEFAULT_CAPS: Caps = Caps {
    max_age_days: Some(30),
    max_total_bytes: Some(2 * 1024 * 1024 * 1024),
    max_rows: None,
};

/// Which segments must go, oldest first, given the caps and the time `now`
/// (FILETIME ticks). `segments` is oldest first. Only sealed segments other
/// than the newest are candidates; a segment already marked `purge_pending`
/// is always a candidate.
#[must_use]
pub fn plan(segments: &[SegmentRow], caps: &Caps, now: i64) -> Vec<String> {
    let newest = segments.len().saturating_sub(1);
    let mut total_bytes: i64 = segments.iter().map(|s| s.bytes).sum();
    let mut total_rows: i64 = segments.iter().map(|s| s.rows).sum();
    let cutoff = caps
        .max_age_days
        .map(|days| now.saturating_sub(i64::from(days).saturating_mul(TICKS_PER_DAY)));
    let max_bytes = caps.max_total_bytes.and_then(|b| i64::try_from(b).ok());
    let max_rows = caps.max_rows.and_then(|r| i64::try_from(r).ok());
    let mut out = Vec::new();
    for (index, segment) in segments.iter().enumerate() {
        if index >= newest || !segment.sealed {
            // Nothing after an unsealed or the newest segment may go: the
            // oldest-first order is the contract.
            break;
        }
        let too_old = cutoff.is_some_and(|cutoff| segment.rows > 0 && segment.last_ts < cutoff);
        let too_big = max_bytes.is_some_and(|max| total_bytes > max);
        let too_many = max_rows.is_some_and(|max| total_rows > max);
        if segment.purge_pending || too_old || too_big || too_many {
            total_bytes -= segment.bytes;
            total_rows -= segment.rows;
            out.push(segment.file.clone());
        } else {
            break;
        }
    }
    out
}

/// The viewer's side of the retirement handshake.
pub trait Retire {
    /// Tells the viewer that `file` is about to be deleted and waits for its
    /// acknowledgement (it closed its connections). `true` also when no
    /// viewer is connected.
    fn announce(&self, file: &str) -> bool;
}

/// A viewer that is never there.
#[derive(Debug, Clone, Copy)]
pub struct NoViewer;

impl Retire for NoViewer {
    fn announce(&self, _file: &str) -> bool {
        true
    }
}

/// What happened to one segment of a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The files are gone.
    Purged(SegmentRow),
    /// The file is still open somewhere: remembered, tried again later.
    Deferred(SegmentRow),
}

/// Deletes `file` and its `-wal` and `-shm` side files. `NotFound` counts as
/// done.
fn delete_files(dir: &Path, file: &str) -> io::Result<()> {
    for suffix in ["", "-wal", "-shm"] {
        match std::fs::remove_file(dir.join(format!("{file}{suffix}"))) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// Carries out `files` (from [`plan`]) in `dir`: announce, delete, and update
/// the catalog. A delete that fails is retried a few times, then deferred.
///
/// # Errors
///
/// SQLite's error from the catalog.
pub fn purge(
    dir: &Path,
    catalog: &Catalog,
    files: &[String],
    retire: &dyn Retire,
) -> rusqlite::Result<Vec<Outcome>> {
    let mut out = Vec::new();
    for file in files {
        let Some(row) = catalog.get(file)? else {
            continue;
        };
        let acked = retire.announce(file);
        let mut deleted = false;
        if acked {
            for attempt in 0..3 {
                if delete_files(dir, file).is_ok() {
                    deleted = true;
                    break;
                }
                if attempt < 2 {
                    std::thread::sleep(std::time::Duration::from_millis(40));
                }
            }
        }
        if deleted {
            catalog.remove(file)?;
            out.push(Outcome::Purged(row));
        } else {
            catalog.defer_purge(file)?;
            out.push(Outcome::Deferred(row));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
