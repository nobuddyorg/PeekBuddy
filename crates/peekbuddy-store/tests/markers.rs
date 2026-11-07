//! Markers against real segments (#82): written with their event, listed
//! across day segments, renamed only while their segment is open.
#![expect(clippy::unwrap_used, reason = "tests unwrap")]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use peekbuddy_core::event::{Event, Flags, Kind, NameKind, NameRef, RootIdx, Source, VolIdx};
use peekbuddy_store::batch::Batch;
use peekbuddy_store::markers;
use peekbuddy_store::retention::{Caps, NoViewer};
use peekbuddy_store::segment::{DayFn, DayKey, Rotation};
use peekbuddy_store::writer::{SegmentsConfig, Writer};
use rusqlite::Connection;

const DAY: u64 = 24 * 36_000_000_000;

fn event(ts: u64, kind: Kind, root: RootIdx) -> Event {
    Event {
        ts,
        qpc: ts,
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
        root,
        name: NameRef::EMPTY,
        name_kind: NameKind::Unknown,
        flags: Flags::NONE,
    }
}

fn marker_batch(marks: &[(u64, &str)]) -> Batch {
    let mut batch = Batch::with_capacity(marks.len() + 1, 4096);
    for &(ts, name) in marks {
        let units: Vec<u16> = name.encode_utf16().collect();
        batch
            .push(event(ts, Kind::Marker, RootIdx::NONE), &units)
            .unwrap();
    }
    batch
}

struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("pb-markers-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn open(dir: &Path) -> Writer {
    let day_of: DayFn = Arc::new(|ts| DayKey {
        year: 2026,
        month: 10,
        day: u8::try_from(1 + ts / DAY).unwrap_or(28).min(28),
    });
    let config = SegmentsConfig {
        rotation: Rotation {
            dir: dir.to_path_buf(),
            day_of,
            size_cap: u64::MAX,
        },
        caps: Caps::default(),
        retire: Arc::new(NoViewer),
    };
    Writer::open_segments(config, 10).unwrap().0
}

#[test]
fn a_marker_is_an_event_row_and_a_marker_row() {
    let dir = Dir::new("row");
    let mut writer = open(dir.path());
    let commit = writer
        .write(&[marker_batch(&[(10, "before install")])], &[])
        .unwrap();
    let found = markers::list(dir.path()).unwrap();
    assert_eq!(found.len(), 1);
    let marker = found.first().unwrap();
    assert_eq!(marker.event_id, commit.first_id);
    assert_eq!(marker.name, "before install");
    assert_eq!(marker.created_ts, 10);
    assert!(marker.open_segment);
    assert!(marker.expires_with("events-2026-10-01.db"));
    assert!(!marker.expires_with("events-2026-10-02.db"));
    // The event row shows the name where it happened.
    let conn = Connection::open(dir.path().join("events-2026-10-01.db")).unwrap();
    let name: String = conn
        .query_row(
            "SELECT n.name FROM event e JOIN node n ON n.id = e.node_id WHERE e.id = ?1",
            [commit.first_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(name, "before install");
    // Kind 28 << 8.
    let kind: i64 = conn
        .query_row(
            "SELECT kind FROM event WHERE id = ?1",
            [commit.first_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(kind, 7168);
}

#[test]
fn markers_are_listed_across_segments_in_order() {
    let dir = Dir::new("list");
    let mut writer = open(dir.path());
    writer.write(&[marker_batch(&[(10, "one")])], &[]).unwrap();
    writer
        .write(
            &[marker_batch(&[(DAY + 1, "two"), (DAY + 2, "three")])],
            &[],
        )
        .unwrap();
    writer.wait_background();
    let found = markers::list(dir.path()).unwrap();
    let names: Vec<_> = found.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, ["one", "two", "three"]);
    let ids: Vec<_> = found.iter().map(|m| m.event_id).collect();
    assert!(ids.windows(2).all(|w| w[0] < w[1]), "{ids:?}");
    let open: Vec<_> = found.iter().map(|m| m.open_segment).collect();
    assert_eq!(open, [false, true, true]);
    assert_eq!(found.first().unwrap().segment, "events-2026-10-01.db");
}

#[test]
fn only_a_marker_of_the_open_segment_can_be_renamed() {
    let dir = Dir::new("rename");
    let mut writer = open(dir.path());
    writer
        .write(&[marker_batch(&[(10, "old day")])], &[])
        .unwrap();
    writer
        .write(&[marker_batch(&[(DAY + 1, "today")])], &[])
        .unwrap();
    writer.wait_background();
    let found = markers::list(dir.path()).unwrap();
    let (old, new) = (found[0].event_id, found[1].event_id);
    assert!(!writer.rename_marker(old, "renamed").unwrap());
    assert!(writer.rename_marker(new, "  after the build ").unwrap());
    assert!(!writer.rename_marker(new, "  ").unwrap());
    assert_eq!(
        writer.marker_name(new).unwrap().as_deref(),
        Some("after the build")
    );
    assert_eq!(writer.marker_name(old).unwrap(), None);
    let names: Vec<_> = markers::list(dir.path())
        .unwrap()
        .into_iter()
        .map(|m| m.name)
        .collect();
    assert_eq!(names, ["old day", "after the build"]);
    assert!(!writer.rename_marker(9999, "nobody").unwrap());
}

#[test]
fn a_segment_from_before_schema_two_has_no_markers() {
    let dir = Dir::new("v1");
    let mut writer = open(dir.path());
    writer.write(&[marker_batch(&[(10, "x")])], &[]).unwrap();
    drop(writer);
    // Make the open segment look like schema 1.
    let conn = Connection::open(dir.path().join("events-2026-10-01.db")).unwrap();
    conn.execute_batch("DROP TABLE marker;").unwrap();
    drop(conn);
    assert_eq!(markers::list(dir.path()).unwrap(), []);
}
