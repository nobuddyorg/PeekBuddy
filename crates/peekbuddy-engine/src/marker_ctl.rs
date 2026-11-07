//! Markers as the engine writes them (#82).
//!
//! A marker is a system row of kind [`Kind::Marker`] whose name is the
//! marker's name; the writer adds the `marker` row in the same commit. The
//! row goes through the same ring as every other row, so it is ordered with
//! the captured events and survives a spill like them. Renaming goes straight
//! to the open segment (see [`peekbuddy_store::markers::rename`]).

use std::path::PathBuf;

use peekbuddy_core::event::{Event, Flags, Kind, NameKind, NameRef, RootIdx, Source, VolIdx};
use peekbuddy_store::markers::{self, clean_name};
use peekbuddy_store::pipeline::Producer;

/// The name used when none is given and the time is not known.
pub const DEFAULT_NAME: &str = "Marker";

/// FILETIME ticks per second.
const SECOND: u64 = 10_000_000;

/// `Marker HH:MM:SS` for an instant shifted by `offset` seconds from UTC.
#[must_use]
pub fn timed_name(ts: u64, offset: i32) -> String {
    let seconds = (ts / SECOND).saturating_add_signed(i64::from(offset));
    let day = seconds % 86_400;
    format!(
        "{DEFAULT_NAME} {:02}:{:02}:{:02}",
        day / 3_600,
        day / 60 % 60,
        day % 60
    )
}

/// The marker row for `(ts, qpc)`; its name travels beside it.
#[must_use]
pub fn marker_event((ts, qpc): (u64, u64)) -> Event {
    Event {
        ts,
        qpc,
        src_ts: 0,
        source: Source::System,
        kind: Kind::Marker,
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

/// The name a marker gets for `asked`: cleaned, or [`DEFAULT_NAME`].
#[must_use]
pub fn name_for(asked: &str) -> String {
    clean_name(asked).unwrap_or_else(|| DEFAULT_NAME.to_owned())
}

/// The time now as `(FILETIME ticks, monotonic ticks)`, and the offset of the
/// local time from UTC in seconds at an instant.
pub type Clocks = (fn() -> (u64, u64), fn(u64) -> i32);

/// Creates and renames markers.
pub struct MarkerCtl {
    producer: Producer,
    segments: PathBuf,
    now: fn() -> (u64, u64),
    offset: fn(u64) -> i32,
}

impl MarkerCtl {
    /// A controller that writes through `producer` and renames in `segments`.
    #[must_use]
    pub const fn new(producer: Producer, segments: PathBuf, clocks: Clocks) -> Self {
        Self {
            producer,
            segments,
            now: clocks.0,
            offset: clocks.1,
        }
    }

    /// Writes a marker row now; returns the name it got.
    ///
    /// # Errors
    ///
    /// A message when the row does not fit a batch.
    pub fn mark(&self, asked: &str) -> Result<String, String> {
        let (ts, qpc) = (self.now)();
        let name = clean_name(asked).unwrap_or_else(|| timed_name(ts, (self.offset)(ts)));
        let units: Vec<u16> = name.encode_utf16().collect();
        let mut batch = self.producer.new_batch();
        batch
            .push(marker_event((ts, qpc)), &units)
            .map_err(|_| String::from("the marker does not fit a batch"))?;
        let _ = self.producer.push(batch);
        Ok(name)
    }

    /// Renames the marker whose event is `event_id`.
    ///
    /// # Errors
    ///
    /// A message when the id is out of range, the marker is not in the open
    /// segment, or the store fails.
    pub fn rename(&self, event_id: u64, asked: &str) -> Result<String, String> {
        let id = i64::try_from(event_id).map_err(|_| String::from("no such marker"))?;
        let name = clean_name(asked).ok_or_else(|| String::from("the name is empty"))?;
        match markers::rename(&self.segments, id, &name) {
            Ok(true) => Ok(name),
            Ok(false) => Err(String::from(
                "no such marker in today's segment; older markers keep their name",
            )),
            Err(error) => Err(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_NAME, marker_event, name_for, timed_name};
    use peekbuddy_core::event::{Kind, RootIdx, Source};

    #[test]
    fn a_marker_row_is_a_system_row_without_a_root() {
        let event = marker_event((5, 6));
        assert_eq!((event.ts, event.qpc), (5, 6));
        assert_eq!(event.kind, Kind::Marker);
        assert_eq!(event.source, Source::System);
        assert_eq!(event.root, RootIdx::NONE);
    }

    #[test]
    fn the_default_name_carries_the_local_time() {
        // 1601-01-01 00:00:00 UTC plus 13:05:09.
        let ticks = (13 * 3_600 + 5 * 60 + 9) * 10_000_000;
        assert_eq!(timed_name(ticks, 0), "Marker 13:05:09");
        assert_eq!(timed_name(ticks, 2 * 3_600), "Marker 15:05:09");
    }

    #[test]
    fn an_empty_or_blank_name_becomes_the_default() {
        assert_eq!(name_for(""), DEFAULT_NAME);
        assert_eq!(name_for("  \\ "), DEFAULT_NAME);
        assert_eq!(name_for(" build done "), "build done");
    }
}
