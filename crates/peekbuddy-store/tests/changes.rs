//! "What changed since" against real segments (#82): the net change across
//! day segments, rename pairs, gaps and cancelling.
#![expect(clippy::unwrap_used, reason = "tests unwrap")]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use peekbuddy_core::changes::NetKind;
use peekbuddy_core::event::{
    Event, Flags, GapReason, Kind, NameKind, NameRef, RootIdx, Source, VolIdx,
};
use peekbuddy_core::query::CancelFlag;
use peekbuddy_store::batch::Batch;
use peekbuddy_store::markers;
use peekbuddy_store::read::Reader;
use peekbuddy_store::retention::{Caps, NoViewer};
use peekbuddy_store::segment::{DayFn, DayKey, Rotation};
use peekbuddy_store::writer::{RootInfo, SegmentsConfig, VolInfo, Writer};

const DAY: u64 = 24 * 36_000_000_000;

fn event(ts: u64, kind: Kind, file_id: u128) -> Event {
    Event {
        ts,
        qpc: ts,
        src_ts: 0,
        source: Source::Rdcw,
        kind,
        raw_mask: 1,
        source_info: 0,
        is_dir: false,
        size: 1,
        attrs: 0x20,
        reparse_tag: 0,
        file_id,
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

struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("pb-changes-{tag}-{}", std::process::id()));
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
    let mut writer = Writer::open_segments(config, 10).unwrap().0;
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

/// A batch of `(ts, kind, relative path, file id)`.
fn batch(rows: &[(u64, Kind, &str, u128)]) -> Batch {
    let mut batch = Batch::with_capacity(rows.len() + 1, 16_384);
    for &(ts, kind, name, file_id) in rows {
        batch.push(event(ts, kind, file_id), &units(name)).unwrap();
    }
    batch
}

fn changes(dir: &Path, since: i64) -> peekbuddy_store::read::changes::ChangeSet {
    Reader::new(dir)
        .changes_since(since, &CancelFlag::new(), &mut |_| {})
        .unwrap()
}

fn rows(set: &peekbuddy_store::read::changes::ChangeSet) -> Vec<(NetKind, String, Option<String>)> {
    set.summary
        .rows
        .iter()
        .map(|r| {
            (
                r.kind,
                set.path(r.key).to_owned(),
                r.from.map(|f| set.path(f).to_owned()),
            )
        })
        .collect()
}

#[test]
fn the_net_change_spans_day_segments_and_names_the_paths() {
    let dir = Dir::new("span");
    let mut writer = open(dir.path());
    writer
        .write(
            &[batch(&[
                (10, Kind::Created, "a.txt", 1),
                (11, Kind::Modified, "a.txt", 1),
                (12, Kind::Created, "tmp.part", 2),
            ])],
            &[],
        )
        .unwrap();
    writer
        .write(
            &[batch(&[
                (DAY + 1, Kind::Deleted, "tmp.part", 2),
                (DAY + 2, Kind::Modified, "old\\b.txt", 3),
                (DAY + 3, Kind::Deleted, "gone.txt", 4),
            ])],
            &[],
        )
        .unwrap();
    writer.wait_background();
    let set = changes(dir.path(), 0);
    assert!(!set.cancelled);
    assert_eq!(set.summary.transient, 1);
    let kinds: Vec<_> = rows(&set)
        .into_iter()
        .map(|(kind, path, _)| (kind, path))
        .collect();
    assert_eq!(
        kinds,
        [
            (NetKind::Created, String::from(r"C:\Watched\a.txt")),
            (NetKind::Deleted, String::from(r"C:\Watched\gone.txt")),
            (NetKind::Modified, String::from(r"C:\Watched\old\b.txt")),
        ]
    );
    // The created file folds its create and its modify.
    let created = set
        .summary
        .rows
        .iter()
        .find(|r| r.kind == NetKind::Created)
        .unwrap();
    assert_eq!(created.raw_events, 2);
    assert!(!set.summary.incomplete());
}

#[test]
fn only_rows_after_the_marker_count() {
    let dir = Dir::new("since");
    let mut writer = open(dir.path());
    writer
        .write(&[batch(&[(10, Kind::Created, "before.txt", 1)])], &[])
        .unwrap();
    let mut mark = Batch::with_capacity(2, 256);
    let mut marker = event(20, Kind::Marker, 0);
    marker.source = Source::System;
    marker.root = RootIdx::NONE;
    mark.push(marker, &units("install")).unwrap();
    writer.write(&[mark], &[]).unwrap();
    writer
        .write(
            &[batch(&[
                (30, Kind::Created, "after.txt", 2),
                (31, Kind::Modified, "before.txt", 1),
            ])],
            &[],
        )
        .unwrap();
    let marker = markers::list(dir.path()).unwrap().remove(0);
    let set = changes(dir.path(), marker.event_id);
    assert_eq!(
        rows(&set)
            .into_iter()
            .map(|(k, p, _)| (k, p))
            .collect::<Vec<_>>(),
        [
            (NetKind::Created, String::from(r"C:\Watched\after.txt")),
            (NetKind::Modified, String::from(r"C:\Watched\before.txt")),
        ]
    );
    assert_eq!(set.since_id, marker.event_id);
    assert!(set.last_id > marker.event_id);
}

#[test]
fn a_rename_pair_is_one_row_and_a_move_out_is_a_delete() {
    let dir = Dir::new("rename");
    let mut writer = open(dir.path());
    writer
        .write(
            &[batch(&[
                (10, Kind::RenamedFrom, "report.doc", 7),
                (11, Kind::RenamedTo, "final.doc", 7),
                (12, Kind::RenamedFrom, "moved-away.doc", 8),
                (13, Kind::Modified, "other.txt", 9),
            ])],
            &[],
        )
        .unwrap();
    let set = changes(dir.path(), 0);
    assert_eq!(
        rows(&set),
        [
            (
                NetKind::Deleted,
                String::from(r"C:\Watched\moved-away.doc"),
                None
            ),
            (
                NetKind::Modified,
                String::from(r"C:\Watched\other.txt"),
                None
            ),
            (
                NetKind::Renamed,
                String::from(r"C:\Watched\final.doc"),
                Some(String::from(r"C:\Watched\report.doc"))
            ),
        ]
    );
}

#[test]
fn paths_differ_by_case_only_are_one_path() {
    let dir = Dir::new("case");
    let mut writer = open(dir.path());
    writer
        .write(
            &[batch(&[
                (10, Kind::Created, "Report.TXT", 1),
                (11, Kind::Modified, "report.txt", 1),
            ])],
            &[],
        )
        .unwrap();
    let set = changes(dir.path(), 0);
    assert_eq!(set.summary.rows.len(), 1);
    assert_eq!(set.summary.rows[0].raw_events, 2);
}

#[test]
fn an_overflow_makes_the_diff_incomplete_until_a_reconcile_covers_it() {
    let dir = Dir::new("gap");
    let mut writer = open(dir.path());
    writer
        .write(
            &[batch(&[
                (10, Kind::Created, "a.txt", 1),
                (11, Kind::Overflow(GapReason::BufferOverflow), "", 0),
            ])],
            &[],
        )
        .unwrap();
    let set = changes(dir.path(), 0);
    assert!(set.summary.incomplete());
    assert_eq!(set.summary.gaps.len(), 1);
    writer
        .write(&[batch(&[(12, Kind::ReconcileFinished, "", 0)])], &[])
        .unwrap();
    let set = changes(dir.path(), 0);
    assert!(!set.summary.incomplete());
    assert!(set.summary.gaps.iter().all(|g| g.reconciled));
}

#[test]
fn a_cancelled_scan_says_so_and_a_finished_one_reports_progress() {
    let dir = Dir::new("cancel");
    let mut writer = open(dir.path());
    let many: Vec<(u64, Kind, String, u128)> = (0..20_000_u64)
        .map(|i| (10 + i, Kind::Created, format!("f{i}.txt"), u128::from(i)))
        .collect();
    for chunk in many.chunks(200) {
        let rows: Vec<(u64, Kind, &str, u128)> = chunk
            .iter()
            .map(|(ts, kind, name, id)| (*ts, *kind, name.as_str(), *id))
            .collect();
        writer.write(&[batch(&rows)], &[]).unwrap();
    }
    let mut seen = Vec::new();
    let done = Reader::new(dir.path())
        .changes_since(0, &CancelFlag::new(), &mut |p| seen.push(p.scanned))
        .unwrap();
    assert!(!done.cancelled);
    assert_eq!(done.summary.rows.len(), 20_000);
    assert!(seen.len() >= 2, "{seen:?}");
    assert_eq!(seen.last().copied(), Some(20_000));

    let cancel = CancelFlag::new();
    let stopped = Reader::new(dir.path())
        .changes_since(0, &cancel, &mut |_| cancel.cancel())
        .unwrap();
    assert!(stopped.cancelled);
    assert!(stopped.summary.rows.len() < 20_000);
}

#[test]
fn a_time_is_found_as_the_id_before_it_across_segments() {
    let dir = Dir::new("before");
    let mut writer = open(dir.path());
    let first = writer
        .write(
            &[batch(&[
                (100, Kind::Created, "a", 1),
                (200, Kind::Created, "b", 2),
                (300, Kind::Created, "c", 3),
            ])],
            &[],
        )
        .unwrap()
        .first_id;
    let second = writer
        .write(
            &[batch(&[
                (DAY + 100, Kind::Created, "d", 4),
                (DAY + 200, Kind::Created, "e", 5),
            ])],
            &[],
        )
        .unwrap();
    writer.wait_background();
    let reader = Reader::new(dir.path());
    assert_eq!(reader.id_before(50).unwrap(), 0);
    assert_eq!(reader.id_before(100).unwrap(), 0);
    assert_eq!(reader.id_before(101).unwrap(), first);
    assert_eq!(reader.id_before(250).unwrap(), first + 1);
    assert_eq!(reader.id_before(DAY).unwrap(), first + 2);
    // The rotation row of the second day sits between the two days.
    assert_eq!(reader.id_before(DAY + 150).unwrap(), second.last_id - 1);
    assert_eq!(reader.id_before(u64::MAX).unwrap(), second.last_id);
}

#[test]
fn a_gap_names_its_root() {
    let dir = Dir::new("roots");
    let mut writer = open(dir.path());
    writer
        .write(
            &[batch(&[(10, Kind::Overflow(GapReason::RingFull), "", 0)])],
            &[],
        )
        .unwrap();
    let set = changes(dir.path(), 0);
    let gap = set.summary.gaps[0];
    assert_eq!(
        set.roots.get(&gap.root).map(String::as_str),
        Some(r"\\?\C:\Watched")
    );
}

/// Times one pass over a kept store (`PEEKBUDDY_QUERY_DIR`, built by the query
/// bench): `cargo test --release -p peekbuddy-store --test changes -- --ignored --nocapture`.
#[test]
#[ignore = "needs the kept query-bench store"]
#[expect(clippy::print_stdout, reason = "the point is the printed timing")]
fn timing_of_a_pass_over_a_kept_store() {
    let Some(dir) = std::env::var_os("PEEKBUDDY_QUERY_DIR") else {
        return;
    };
    let started = std::time::Instant::now();
    let mut scanned_in = None;
    let set = Reader::new(Path::new(&dir))
        .changes_since(0, &CancelFlag::new(), &mut |p| {
            if p.segments_done == p.segments {
                scanned_in = Some(started.elapsed());
            }
        })
        .unwrap();
    println!("scan done after {scanned_in:?}");
    println!(
        "changes_since: {} rows folded to {} lines in {:?}",
        set.summary.raw_events,
        set.summary.rows.len(),
        started.elapsed()
    );
}
