//! Day segments against real files (#54): rotation, the id sequence across
//! files, sealing, retention and the readers' view.
#![expect(clippy::unwrap_used, reason = "tests unwrap")]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use peekbuddy_core::event::{Event, Flags, Kind, NameKind, NameRef, RootIdx, Source, VolIdx};
use peekbuddy_store::batch::Batch;
use peekbuddy_store::catalog::Catalog;
use peekbuddy_store::retention::{Caps, NoViewer, Retire};
use peekbuddy_store::segment::{DayFn, DayKey, NewestFirst, Rotation, Step};
use peekbuddy_store::writer::{Cursor, RootInfo, SegmentsConfig, VolInfo, Writer};
use proptest::prelude::*;
use rusqlite::Connection;

/// 100 ns ticks per hour.
const HOUR: u64 = 36_000_000_000;
const DAY: u64 = 24 * HOUR;

fn event(ts: u64) -> Event {
    Event {
        ts,
        qpc: ts,
        src_ts: 0,
        source: Source::Rdcw,
        kind: Kind::Created,
        raw_mask: 1,
        source_info: 0,
        is_dir: false,
        size: 1,
        attrs: 0x20,
        reparse_tag: 0,
        file_id: u128::from(ts),
        parent_file_id: 5,
        usn: 0,
        vol: VolIdx(0),
        root: RootIdx(0),
        name: NameRef::EMPTY,
        name_kind: NameKind::Long,
        flags: Flags::NONE,
    }
}

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn batch(timestamps: &[u64]) -> Batch {
    let mut batch = Batch::with_capacity(timestamps.len() + 1, 8192);
    for &ts in timestamps {
        batch
            .push(event(ts), &units(&format!("sub\\file{ts}.txt")))
            .unwrap();
    }
    batch
}

struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("pb-segments-{tag}-{}", std::process::id()));
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

/// Days of 24 hours counted from 2026-10-01.
fn plain_days() -> DayFn {
    Arc::new(|ts| DayKey {
        year: 2026,
        month: 10,
        day: u8::try_from(1 + ts / DAY).unwrap_or(28).min(28),
    })
}

fn config(dir: &Path, day_of: DayFn, cap: u64, caps: Caps) -> SegmentsConfig {
    SegmentsConfig {
        rotation: Rotation {
            dir: dir.to_path_buf(),
            day_of,
            size_cap: cap,
        },
        caps,
        retire: Arc::new(NoViewer),
    }
}

fn open(dir: &Path, day_of: DayFn, cap: u64, caps: Caps, now: u64) -> Writer {
    let (mut writer, skipped) = Writer::open_segments(config(dir, day_of, cap, caps), now).unwrap();
    assert!(skipped.is_empty(), "{skipped:?}");
    writer
        .register_root(&RootInfo {
            idx: RootIdx(0),
            vol: VolInfo {
                guid: String::from("vol-1"),
                serial: 1,
                letter: Some(String::from("C:")),
                journal_id: None,
            },
            path: units(r"\\?\C:\Watched"),
            recursive: true,
        })
        .unwrap();
    writer
}

fn ids(path: &Path) -> Vec<i64> {
    let conn = Connection::open(path).unwrap();
    let mut statement = conn.prepare("SELECT id FROM event ORDER BY id").unwrap();
    statement
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn count(path: &Path, sql: &str) -> i64 {
    Connection::open(path)
        .unwrap()
        .query_row(sql, [], |row| row.get(0))
        .unwrap()
}

fn files(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("events-") && Path::new(n).extension().is_some_and(|e| e == "db"))
        .collect();
    names.sort();
    names
}

#[test]
fn the_first_event_of_a_new_day_rotates_and_the_ids_go_on() {
    let dir = Dir::new("rotate");
    let mut writer = open(dir.path(), plain_days(), u64::MAX, Caps::default(), 10);
    assert_eq!(writer.segment_file(), Some("events-2026-10-01.db"));
    writer.write(&[batch(&[10, 20, 30])], &[]).unwrap();
    let commit = writer.write(&[batch(&[DAY + 5, DAY + 6])], &[]).unwrap();
    // The rotation row is the first row of the new file.
    assert_eq!((commit.first_id, commit.last_id), (4, 6));
    assert_eq!(writer.segment_file(), Some("events-2026-10-02.db"));
    writer.wait_background();

    assert_eq!(ids(&dir.path().join("events-2026-10-01.db")), [1, 2, 3]);
    assert_eq!(ids(&dir.path().join("events-2026-10-02.db")), [4, 5, 6]);

    let new_file = dir.path().join("events-2026-10-02.db");
    // The rotation row: kind SegmentRotated (17 << 8), source System (3).
    assert_eq!(
        count(
            &new_file,
            "SELECT count(*) FROM event WHERE id = 4 AND kind = 4352 AND source = 3"
        ),
        1
    );
    // Events resolve their names through the new file's own dictionary.
    assert_eq!(
        count(
            &new_file,
            "SELECT count(*) FROM node WHERE name LIKE 'file%'"
        ),
        2
    );
    assert_eq!(count(&new_file, "SELECT count(*) FROM root"), 1);

    let catalog = Catalog::open(&dir.path().join("catalog.db")).unwrap();
    let rows = catalog.list().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        (
            rows[0].first_id,
            rows[0].last_id,
            rows[0].rows,
            rows[0].sealed
        ),
        (1, 3, 3, true)
    );
    assert_eq!((rows[1].first_id, rows[1].sealed), (4, false));
    assert_eq!(catalog.next_id().unwrap(), 4);
}

#[test]
fn a_sealed_segment_is_never_written_again() {
    let dir = Dir::new("sealed");
    let mut writer = open(dir.path(), plain_days(), u64::MAX, Caps::default(), 10);
    writer.write(&[batch(&[10])], &[]).unwrap();
    writer.write(&[batch(&[DAY + 1])], &[]).unwrap();
    writer.wait_background();
    let old = dir.path().join("events-2026-10-01.db");
    let before = ids(&old);
    for i in 0..5 {
        // Even events stamped in the first day's time go to the open file.
        writer
            .write(&[batch(&[DAY + 10 + i, 20 + i])], &[])
            .unwrap();
    }
    assert_eq!(ids(&old), before);
    assert_eq!(files(dir.path()).len(), 2);
}

#[test]
fn the_clock_moving_backwards_keeps_writing_where_the_ids_are() {
    let dir = Dir::new("backwards");
    let mut writer = open(dir.path(), plain_days(), u64::MAX, Caps::default(), 10);
    writer.write(&[batch(&[2 * DAY + 1])], &[]).unwrap();
    writer.write(&[batch(&[10])], &[]).unwrap();
    assert_eq!(files(dir.path()).len(), 1, "{:?}", files(dir.path()));
    assert_eq!(writer.next_id(), 3);
}

#[test]
fn the_size_cap_rotates_within_a_day_with_numbered_files() {
    let dir = Dir::new("cap");
    // A cap of 1 byte: every write finds the file full.
    let mut writer = open(dir.path(), plain_days(), 1, Caps::default(), 10);
    for i in 0..4_u64 {
        writer.write(&[batch(&[10 + i])], &[]).unwrap();
    }
    writer.wait_background();
    assert_eq!(
        files(dir.path()),
        [
            "events-2026-10-01.1.db",
            "events-2026-10-01.2.db",
            "events-2026-10-01.3.db",
            "events-2026-10-01.db"
        ]
    );
    let all: Vec<i64> = [
        "events-2026-10-01.db",
        "events-2026-10-01.1.db",
        "events-2026-10-01.2.db",
        "events-2026-10-01.3.db",
    ]
    .iter()
    .flat_map(|f| ids(&dir.path().join(f)))
    .collect();
    // Four events plus three rotation rows, one contiguous sequence.
    assert_eq!(all, (1..=7).collect::<Vec<i64>>());
}

#[test]
fn an_empty_segment_is_dropped_instead_of_sealed() {
    let dir = Dir::new("empty");
    let mut writer = open(dir.path(), plain_days(), u64::MAX, Caps::default(), 10);
    // Nothing written on day one; the first event is on day three.
    writer.write(&[batch(&[2 * DAY + 5])], &[]).unwrap();
    writer.wait_background();
    assert_eq!(files(dir.path()), ["events-2026-10-03.db"]);
    let catalog = Catalog::open(&dir.path().join("catalog.db")).unwrap();
    assert_eq!(catalog.list().unwrap().len(), 1);
    assert_eq!(ids(&dir.path().join("events-2026-10-03.db")), [1]);
}

#[test]
fn the_replay_position_and_the_cursors_follow_the_rotation() {
    let dir = Dir::new("carry");
    let mut writer = open(dir.path(), plain_days(), u64::MAX, Caps::default(), 10);
    writer
        .write_with_spill(
            &[batch(&[10])],
            &[Cursor {
                root: RootIdx(0),
                usn: Some(77),
            }],
            Some((3, 4096)),
        )
        .unwrap();
    writer.write(&[batch(&[DAY + 1])], &[]).unwrap();
    assert_eq!(writer.spill_position(), (3, 4096));
    let new_file = dir.path().join("events-2026-10-02.db");
    assert_eq!(count(&new_file, "SELECT count(*) FROM cursor"), 1);
    assert_eq!(count(&new_file, "SELECT usn FROM cursor"), 77);
}

#[test]
fn a_restart_continues_in_the_newest_segment_and_the_id_sequence() {
    let dir = Dir::new("restart");
    {
        let mut writer = open(dir.path(), plain_days(), u64::MAX, Caps::default(), 10);
        writer.write(&[batch(&[10, 11])], &[]).unwrap();
        writer.write(&[batch(&[DAY + 1])], &[]).unwrap();
        writer.wait_background();
        writer.checkpoint_truncate().unwrap();
    }
    let mut writer = open(
        dir.path(),
        plain_days(),
        u64::MAX,
        Caps::default(),
        DAY + 100,
    );
    assert_eq!(writer.segment_file(), Some("events-2026-10-02.db"));
    let commit = writer.write(&[batch(&[DAY + 200])], &[]).unwrap();
    assert_eq!(commit.first_id, 5);
    // A restart on the next day starts the next file at the right id.
    drop(writer);
    let mut writer = open(dir.path(), plain_days(), u64::MAX, Caps::default(), 3 * DAY);
    let commit = writer.write(&[batch(&[3 * DAY + 1])], &[]).unwrap();
    assert_eq!(commit.first_id, 6, "the rotation row is first");
    assert_eq!(writer.segment_file(), Some("events-2026-10-04.db"));
}

#[test]
fn junk_and_foreign_files_are_skipped_and_never_touched() {
    let dir = Dir::new("junk");
    std::fs::write(dir.path().join("events-2026-09-30.db"), b"not a database").unwrap();
    let foreign = Connection::open(dir.path().join("events-2026-09-29.db")).unwrap();
    foreign
        .execute_batch("CREATE TABLE t (a); PRAGMA application_id = 5;")
        .unwrap();
    drop(foreign);
    std::fs::write(dir.path().join("readme.txt"), b"keep").unwrap();
    let (writer, skipped) = Writer::open_segments(
        config(dir.path(), plain_days(), u64::MAX, Caps::default()),
        10,
    )
    .unwrap();
    let names: Vec<&str> = skipped.iter().map(|s| s.file.as_str()).collect();
    assert_eq!(names, ["events-2026-09-29.db", "events-2026-09-30.db"]);
    assert_eq!(writer.segment_file(), Some("events-2026-10-01.db"));
    assert_eq!(
        std::fs::read(dir.path().join("events-2026-09-30.db")).unwrap(),
        b"not a database"
    );
    assert_eq!(
        std::fs::read(dir.path().join("readme.txt")).unwrap(),
        b"keep"
    );
}

#[test]
fn retention_deletes_the_oldest_and_writes_a_purge_row_with_the_range() {
    let dir = Dir::new("retention");
    let caps = Caps {
        max_age_days: Some(2),
        ..Caps::default()
    };
    let mut writer = open(dir.path(), plain_days(), u64::MAX, caps, 10);
    for day in 0..6_u64 {
        writer
            .write(&[batch(&[day * DAY + 5, day * DAY + 6])], &[])
            .unwrap();
        writer.wait_background();
    }
    writer.flush_system_rows().unwrap();
    // Only the segments within two days of the last rotation stay.
    let left = files(dir.path());
    assert!(left.len() <= 4 && left.len() >= 2, "{left:?}");
    assert!(!left.contains(&String::from("events-2026-10-01.db")));
    // Every purge is a RetentionPurged row (18 << 8) in the open segment,
    // with the id range of the segment it removed.
    let current = dir.path().join(writer.segment_file().unwrap());
    let purged = count(
        &current,
        "SELECT count(*) FROM event WHERE kind = 4608 AND source = 3",
    );
    assert!(purged >= 1, "no purge rows");
    // The row carries the id range of the segment it removed: before the
    // open segment, and in order.
    let conn = Connection::open(&current).unwrap();
    let first_here: i64 = conn
        .query_row("SELECT min(id) FROM event", [], |row| row.get(0))
        .unwrap();
    let mut statement = conn
        .prepare("SELECT usn, usn + size - 1 FROM event WHERE kind = 4608 ORDER BY id")
        .unwrap();
    let ranges: Vec<(i64, i64)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    for (start, end) in &ranges {
        assert!(
            start <= end && *end < first_here,
            "{start}..{end} vs {first_here}"
        );
    }
    assert!(ranges.windows(2).all(|w| w[0].1 < w[1].0), "{ranges:?}");
    // The catalog and the disk agree.
    let catalog = Catalog::open(&dir.path().join("catalog.db")).unwrap();
    let in_catalog: Vec<String> = catalog
        .list()
        .unwrap()
        .into_iter()
        .map(|r| r.file)
        .collect();
    assert_eq!(in_catalog, left);
}

#[expect(clippy::panic, reason = "a test helper fails the test with the cause")]
fn expect_segment(step: Option<Step>) -> (Box<peekbuddy_store::catalog::SegmentRow>, Connection) {
    match step {
        Some(Step::Segment(row, conn)) => (row, conn),
        other => panic!("expected a segment, got {other:?}"),
    }
}

#[expect(clippy::panic, reason = "a test helper fails the test with the cause")]
fn retired_file(step: Option<Step>) -> String {
    match step {
        Some(Step::Retired(file)) => file,
        other => panic!("expected the retired signal, got {other:?}"),
    }
}

/// A viewer that closes its reader when told, like the real handshake.
struct ClosingViewer(std::sync::Mutex<Vec<Connection>>);

impl Retire for ClosingViewer {
    fn announce(&self, _file: &str) -> bool {
        self.0.lock().unwrap().clear();
        true
    }
}

#[cfg(windows)]
#[test]
fn retention_while_a_reader_holds_the_file_waits_for_the_handshake() {
    use peekbuddy_store::retention::{Outcome, plan, purge};

    let dir = Dir::new("reader");
    let mut writer = open(dir.path(), plain_days(), u64::MAX, Caps::default(), 10);
    for day in 0..3_u64 {
        writer.write(&[batch(&[day * DAY + 5])], &[]).unwrap();
        writer.wait_background();
    }
    let catalog = writer.catalog().unwrap();
    // A viewer reading the oldest segment.
    let oldest = dir.path().join("events-2026-10-01.db");
    let reader = Connection::open(&oldest).unwrap();
    let _: i64 = reader
        .query_row("SELECT count(*) FROM event", [], |row| row.get(0))
        .unwrap();
    let viewer = ClosingViewer(std::sync::Mutex::new(vec![reader]));

    let caps = Caps {
        max_total_bytes: Some(1),
        ..Caps::default()
    };
    let catalog = catalog.lock().unwrap();
    let wanted = plan(&catalog.list().unwrap(), &caps, 10);
    assert_eq!(
        wanted.first().map(String::as_str),
        Some("events-2026-10-01.db")
    );
    // The handshake makes the viewer close its connection first, so the
    // delete goes through.
    let out = purge(dir.path(), &catalog, &wanted[..1], &viewer).unwrap();
    assert!(matches!(out.as_slice(), [Outcome::Purged(_)]), "{out:?}");
    assert!(!oldest.exists());
}

#[test]
fn a_reader_walking_newest_first_survives_a_segment_being_retired() {
    let dir = Dir::new("iterator");
    let mut writer = open(dir.path(), plain_days(), u64::MAX, Caps::default(), 10);
    for day in 0..3_u64 {
        writer.write(&[batch(&[day * DAY + 5])], &[]).unwrap();
        writer.wait_background();
    }
    let catalog = Catalog::open(&dir.path().join("catalog.db")).unwrap();
    let mut walk = NewestFirst::new(dir.path(), &catalog).unwrap();
    let (first, conn) = expect_segment(walk.next_segment());
    assert_eq!(first.file, "events-2026-10-03.db");
    drop(conn);
    // Retention removes the oldest while the reader is mid-scroll.
    std::fs::remove_file(dir.path().join("events-2026-10-01.db")).unwrap();
    for suffix in ["-wal", "-shm"] {
        let _ = std::fs::remove_file(dir.path().join(format!("events-2026-10-01.db{suffix}")));
    }
    let (second, _conn) = expect_segment(walk.next_segment());
    assert_eq!(second.file, "events-2026-10-02.db");
    assert_eq!(retired_file(walk.next_segment()), "events-2026-10-01.db");
    assert!(walk.next_segment().is_none());
}

/// Days of 23, 24 and 25 hours (daylight saving), from a table of lengths.
fn dst_days(lengths: &[u64]) -> DayFn {
    let mut edges = vec![0_u64];
    for hours in lengths {
        edges.push(edges.last().unwrap() + hours * HOUR);
    }
    Arc::new(move |ts| {
        let index = edges.partition_point(|&edge| edge <= ts).saturating_sub(1);
        DayKey {
            year: 2026,
            month: 10,
            day: u8::try_from(1 + index.min(27)).unwrap(),
        }
    })
}

// Four cases: each rotates real files, which is slow on a hosted runner; the
// nightly proptest-long job runs many more.
proptest! {
    #![proptest_config(ProptestConfig { cases: 4, ..ProptestConfig::default() })]

    #[test]
    fn rotation_keeps_the_invariants_with_dst_days_and_a_wandering_clock(
        lengths in proptest::collection::vec(prop_oneof![Just(23_u64), Just(24), Just(25)], 28),
        steps in proptest::collection::vec((0_u64..30, any::<bool>(), 1_usize..4), 5..40),
        cap in prop_oneof![Just(u64::MAX), Just(1_u64), Just(30_000_u64)],
    ) {
        let dir = Dir::new(&format!("prop{}", rand_tag()));
        let day_of = dst_days(&lengths);
        let mut writer = open(dir.path(), Arc::clone(&day_of), cap, Caps::default(), 1);
        let mut ts = 1_u64;
        let mut pushed = 0_u64;
        for (hours, backwards, count) in steps {
            // Mostly forward in hours; sometimes the clock steps back.
            ts = if backwards { ts.saturating_sub(hours * HOUR / 2) } else { ts + hours * HOUR };
            let stamps: Vec<u64> = (0..count as u64).map(|i| ts + i).collect();
            writer.write(&[batch(&stamps)], &[]).unwrap();
            pushed += count as u64;
        }
        writer.wait_background();
        writer.flush_system_rows().unwrap();

        // Every segment, oldest first, with its rows.
        let catalog = Catalog::open(&dir.path().join("catalog.db")).unwrap();
        let rows = catalog.list().unwrap();
        let mut all: Vec<i64> = Vec::new();
        let mut events = 0_i64;
        for (index, row) in rows.iter().enumerate() {
            let path = dir.path().join(&row.file);
            let in_file = ids(&path);
            // The catalog row describes the file as it is when it is sealed.
            if row.sealed {
                prop_assert_eq!(in_file.first().copied(), Some(row.first_id));
                prop_assert_eq!(in_file.last().copied(), Some(row.last_id));
                prop_assert!(index + 1 < rows.len(), "the newest segment is sealed");
            }
            events += count(&path, "SELECT count(*) FROM event WHERE source = 0");
            all.extend(in_file);
        }
        // One contiguous, strictly increasing sequence across the files;
        // every event is in exactly one file.
        let expected: Vec<i64> = (1..=i64::try_from(all.len()).unwrap()).collect();
        prop_assert_eq!(&all, &expected);
        prop_assert_eq!(events, i64::try_from(pushed).unwrap());
        // File names are unique and the zone-edge cases never reopened a
        // sealed file: a sealed file's range is final.
        let mut names: Vec<&String> = rows.iter().map(|r| &r.file).collect();
        names.sort();
        names.dedup();
        prop_assert_eq!(names.len(), rows.len());
    }
}

fn rand_tag() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}
