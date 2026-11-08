use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use peekbuddy_core::changes::{GapInfo, NetKind};
use peekbuddy_core::event::{
    Event, Flags, GapReason, Kind, NameKind, NameRef, RootIdx, Source, VolIdx,
};
use peekbuddy_store::batch::Batch;
use peekbuddy_store::retention::{Caps, NoViewer};
use peekbuddy_store::segment::{DayFn, DayKey, Rotation};
use peekbuddy_store::writer::{RootInfo, SegmentsConfig, VolInfo, Writer};

use super::{ChangesAction, ChangesPanel, Line, Since, gap_lines, lines, split_path, totals};

const DAY: u64 = 24 * 36_000_000_000;

fn event(ts: u64, kind: Kind, file_id: u128) -> Event {
    Event {
        ts,
        qpc: ts,
        src_ts: 0,
        source: if kind == Kind::Marker {
            Source::System
        } else {
            Source::Rdcw
        },
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
        root: if kind == Kind::Marker {
            RootIdx::NONE
        } else {
            RootIdx(0)
        },
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
        let dir = std::env::temp_dir().join(format!("pb-changes-ui-{tag}-{}", std::process::id()));
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

fn writer(dir: &Path) -> Writer {
    let day_of: DayFn = Arc::new(|ts| DayKey {
        year: 2026,
        month: 10,
        day: u8::try_from(1 + ts / DAY).unwrap_or(28).min(28),
    });
    let mut writer = Writer::open_segments(
        SegmentsConfig {
            rotation: Rotation {
                dir: dir.to_path_buf(),
                day_of,
                size_cap: u64::MAX,
            },
            caps: Caps::default(),
            retire: Arc::new(NoViewer),
        },
        10,
    )
    .unwrap()
    .0;
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

fn batch(rows: &[(u64, Kind, &str, u128)]) -> Batch {
    let mut batch = Batch::with_capacity(rows.len() + 1, 8192);
    for &(ts, kind, name, file_id) in rows {
        batch.push(event(ts, kind, file_id), &units(name)).unwrap();
    }
    batch
}

/// A log with one marker between an old file and the changes after it.
fn log(dir: &Path) -> Writer {
    let mut writer = writer(dir);
    writer
        .write(&[batch(&[(100, Kind::Created, "before.txt", 1)])], &[])
        .unwrap();
    writer
        .write(&[batch(&[(200, Kind::Marker, "before install", 0)])], &[])
        .unwrap();
    writer
        .write(
            &[batch(&[
                (300, Kind::Created, "app\\setup.exe", 2),
                (310, Kind::Created, "tmp.part", 3),
                (320, Kind::Deleted, "tmp.part", 3),
                (330, Kind::Modified, "before.txt", 1),
                (340, Kind::RenamedFrom, "a.cfg", 4),
                (341, Kind::RenamedTo, "b.cfg", 4),
            ])],
            &[],
        )
        .unwrap();
    writer
}

type State = (ChangesPanel, Vec<ChangesAction>);

fn harness(dir: &Path) -> Harness<'static, State> {
    let mut panel = ChangesPanel::new(
        dir.to_path_buf(),
        Arc::new(|| {}),
        Arc::new(|_| 0),
        (
            || DayKey {
                year: 2026,
                month: 10,
                day: 1,
            },
            || 2_000,
        ),
    );
    panel.set_max_age_days(Some(30));
    Harness::new_ui_state(
        |ui, (panel, taken): &mut State| {
            taken.extend(panel.show(ui));
        },
        (panel, Vec::new()),
    )
}

fn wait_for(harness: &mut Harness<'static, State>, what: &str, done: impl Fn(&mut State) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        harness.step();
        if done(harness.state_mut()) {
            harness.run_steps(2);
            return;
        }
        assert!(Instant::now() < deadline, "{what} never came");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_path_splits_into_folder_and_name() {
    assert_eq!(
        split_path(r"C:\Watched\app\setup.exe"),
        (String::from(r"C:\Watched\app"), String::from("setup.exe"))
    );
    assert_eq!(split_path("bare"), (String::new(), String::from("bare")));
}

#[test]
fn a_line_is_one_row_of_tsv_with_its_raw_count() {
    let line = Line {
        glyph: ">",
        label: "Renamed",
        path: String::from("b"),
        from: Some(String::from("a")),
        modified: false,
        raw_events: 2,
    };
    assert_eq!(line.tsv(), "Renamed\tb\ta\t2");
}

#[test]
fn since_says_what_it_is_measured_from() {
    assert_eq!(
        Since::Marker(3, String::from("x")).describe(),
        "since marker \u{201c}x\u{201d}"
    );
    assert_eq!(Since::Minutes(60).describe(), "in the last hour");
    assert_eq!(Since::Minutes(15).describe(), "in the last 15 minutes");
}

#[test]
fn the_panel_lists_markers_and_folds_the_changes_since_one() {
    let dir = Dir::new("flow");
    let _writer = log(dir.path());
    let mut harness = harness(dir.path());
    harness.run();
    wait_for(&mut harness, "the marker list", |(panel, _)| {
        !panel.lock().markers.is_empty()
    });
    harness.get_by_label("before install");
    harness
        .get_by_label("What changed since marker before install")
        .click();
    harness.run_steps(3);
    wait_for(&mut harness, "the result", |(panel, _)| {
        panel.result().is_some()
    });
    let (since, set) = harness.state().0.result().unwrap();
    assert_eq!(
        since,
        Since::Marker(set.since_id, String::from("before install"))
    );
    let shown = lines(&set);
    let kinds: Vec<_> = shown.iter().map(|l| (l.label, l.path.as_str())).collect();
    assert_eq!(
        kinds,
        [
            ("Created", r"C:\Watched\app\setup.exe"),
            ("Modified", r"C:\Watched\before.txt"),
            ("Renamed", r"C:\Watched\b.cfg"),
        ]
    );
    assert_eq!(
        totals(&set),
        "1 created, 0 deleted, 1 modified, 1 renamed; 1 created and removed again \
         (transient, hidden)"
    );
    harness.get_by_label("What changed since marker \u{201c}before install\u{201d}");
    harness.get_by_label("Copy list");
    // The raw rows of a path are one click away.
    harness
        .get_by_label(r"Show the 1 raw rows of C:\Watched\app\setup.exe")
        .click();
    harness.run();
    assert_eq!(
        harness.state().1,
        [ChangesAction::ShowRows {
            dir: String::from(r"C:\Watched\app"),
            name: String::from("setup.exe")
        }]
    );
    crate::a11y::assert_every_control_is_named(&harness);
}

#[test]
fn mark_now_and_rename_are_actions_for_the_app_to_send() {
    let dir = Dir::new("actions");
    let _writer = log(dir.path());
    let mut harness = harness(dir.path());
    harness.run();
    wait_for(&mut harness, "the marker list", |(panel, _)| {
        !panel.lock().markers.is_empty()
    });
    harness
        .get_by_label("Mark now: name this moment in the log")
        .click();
    harness.run();
    assert_eq!(harness.state().1, [ChangesAction::MarkNow]);
    harness.state_mut().1.clear();

    harness.get_by_label("Rename marker before install").click();
    harness.run();
    harness.get_by_label("New marker name").focus();
    harness.run();
    harness.get_by_label("New marker name").type_text(" 2");
    harness.run();
    harness.get_by_label("Save name").click();
    harness.run();
    let taken = harness.state().1.clone();
    assert!(
        matches!(
            taken.as_slice(),
            [ChangesAction::Rename { name, .. }] if name.starts_with("before install")
        ),
        "{taken:?}"
    );
    crate::a11y::assert_every_control_is_named(&harness);
}

#[test]
fn a_marker_close_to_its_deletion_is_flagged() {
    let dir = Dir::new("expiry");
    let _writer = log(dir.path());
    let mut panel = ChangesPanel::new(
        dir.path().to_path_buf(),
        Arc::new(|| {}),
        Arc::new(|_| 0),
        (
            || DayKey {
                year: 2026,
                month: 10,
                day: 29,
            },
            || 2_000,
        ),
    );
    panel.set_max_age_days(Some(30));
    let mut harness = Harness::new_ui_state(
        |ui, (panel, taken): &mut State| {
            taken.extend(panel.show(ui));
        },
        (panel, Vec::new()),
    );
    harness.run();
    wait_for(&mut harness, "the marker list", |(panel, _)| {
        !panel.lock().markers.is_empty()
    });
    harness.get_by_label("! goes with its day file soon");
}

#[test]
fn an_unreconciled_gap_is_called_incomplete_and_named() {
    let dir = Dir::new("gap");
    let mut writer = writer(dir.path());
    writer
        .write(
            &[batch(&[
                (100, Kind::Marker, "m", 0),
                (200, Kind::Created, "a.txt", 1),
                (210, Kind::Overflow(GapReason::BufferOverflow), "", 0),
            ])],
            &[],
        )
        .unwrap();
    let mut harness = harness(dir.path());
    harness.run();
    wait_for(&mut harness, "the marker list", |(panel, _)| {
        !panel.lock().markers.is_empty()
    });
    harness.get_by_label("What changed since marker m").click();
    harness.run_steps(3);
    wait_for(&mut harness, "the result", |(panel, _)| {
        panel.result().is_some()
    });
    let (_, set) = harness.state().0.result().unwrap();
    assert!(set.summary.incomplete());
    assert_eq!(
        set.summary.gaps,
        vec![GapInfo {
            root: set.summary.gaps[0].root,
            reconciled: false
        }]
    );
    assert_eq!(
        gap_lines(&set),
        [r"C:\Watched: changes were missed and no scan covered them"]
    );
    harness.get_by_label("! Incomplete: the log has holes in this range.");
    harness.get_by_label(r"C:\Watched: changes were missed and no scan covered them");
    assert_eq!(
        set.summary.rows.first().map(|r| r.kind),
        Some(NetKind::Created)
    );
}

#[test]
fn a_time_span_is_measured_back_from_now() {
    let dir = Dir::new("minutes");
    let _writer = log(dir.path());
    let mut harness = harness(dir.path());
    harness.run();
    // "now" is 2_000 ticks and a span is minutes of 600 million ticks: the
    // whole log is inside the last 15 minutes.
    harness
        .get_by_label("What changed in the last 15 minutes")
        .click();
    harness.run_steps(3);
    wait_for(&mut harness, "the result", |(panel, _)| {
        panel.result().is_some()
    });
    let (since, set) = harness.state().0.result().unwrap();
    assert_eq!(since, Since::Minutes(15));
    assert_eq!(set.summary.rows.len(), 3);
    assert_eq!(set.since_id, 0);
}

#[test]
fn an_empty_log_says_how_to_start() {
    let dir = Dir::new("empty");
    let _writer = writer(dir.path());
    let mut harness = harness(dir.path());
    harness.run();
    wait_for(&mut harness, "the empty list", |(panel, _)| {
        !panel.lock().loading
    });
    harness.get_by_label("No markers yet. Mark a moment, do something, then ask what changed.");
    harness.get_by_label("Pick a marker, or a time span, to see what changed.");
}

#[test]
fn a_newer_request_replaces_an_older_one_whatever_order_they_finish_in() {
    let dir = Dir::new("replace");
    let _writer = log(dir.path());
    let mut harness = harness(dir.path());
    harness.run();
    for minutes in [15, 60, 1_440] {
        harness.state_mut().0.compute(Since::Minutes(minutes));
    }
    wait_for(&mut harness, "the last result", |(panel, _)| {
        panel
            .result()
            .is_some_and(|(since, _)| since == Since::Minutes(1_440))
            && !panel.lock().running
    });
    // Nothing older arrives later and takes its place.
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(
        harness.state().0.result().map(|(s, _)| s),
        Some(Since::Minutes(1_440))
    );
}
