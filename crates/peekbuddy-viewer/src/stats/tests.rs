use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use peekbuddy_core::event::{Event, Flags, Kind, NameKind, NameRef, RootIdx, Source, VolIdx};
use peekbuddy_store::batch::Batch;
use peekbuddy_store::read::stats::HourlyCount;
use peekbuddy_store::retention::{Caps, NoViewer};
use peekbuddy_store::schema::TICKS_PER_HOUR;
use peekbuddy_store::segment::{DayFn, DayKey, Rotation};
use peekbuddy_store::writer::{RootInfo, SegmentsConfig, VolInfo, Writer};

use super::{HOURS, RootSeries, StatsPanel, first_hour, hour_label, hour_of, series, spark_points};

const HOUR: u64 = TICKS_PER_HOUR as u64;

fn count(hour: i64, root: &str, events: u64) -> HourlyCount {
    HourlyCount {
        hour_ts: hour,
        root: root.to_owned(),
        events,
    }
}

#[test]
fn the_window_ends_with_the_hour_of_now_and_spans_a_day() {
    let now = 100 * TICKS_PER_HOUR + 12_345;
    assert_eq!(hour_of(now), 100 * TICKS_PER_HOUR);
    assert_eq!(first_hour(now), (100 - 23) * TICKS_PER_HOUR);
    assert_eq!(HOURS, 24);
}

#[test]
fn counts_are_slotted_by_hour_and_summed_per_root() {
    let now = 100 * TICKS_PER_HOUR;
    let h = |back: i64| (100 - back) * TICKS_PER_HOUR;
    let counts = [
        count(h(0), "A", 5),
        count(h(0), "A", 2),
        count(h(23), "A", 9),
        count(h(24), "A", 99),
        count(h(1), "B", 3),
        count(h(3) + 1, "B", 7),
        count(h(-1), "B", 11),
    ];
    let all = series(&counts, now);
    assert_eq!(all.len(), 2);
    let a = &all[0];
    assert_eq!(a.root, "A");
    assert_eq!(a.per_hour.len(), 24);
    assert_eq!((a.per_hour[0], a.per_hour[23]), (9, 7));
    assert_eq!(a.total(), 16);
    let b = &all[1];
    // Off-the-hour, too-old and future counts are left out.
    assert_eq!((b.per_hour[22], b.total()), (3, 3));
}

#[test]
fn the_peak_is_the_first_busiest_hour() {
    let mut per_hour = vec![0; 24];
    per_hour[4] = 8;
    per_hour[9] = 8;
    per_hour[20] = 3;
    let one = RootSeries {
        root: String::from("A"),
        per_hour,
    };
    assert_eq!(one.peak(), (4, 8));
    let none = RootSeries {
        root: String::new(),
        per_hour: vec![0; 24],
    };
    assert_eq!(none.peak(), (0, 0));
}

#[test]
fn a_sparkline_runs_from_the_bottom_to_the_top_of_its_box() {
    let points = spark_points(&[0, 5, 10], 100.0, 20.0);
    assert_eq!(points, [(0.0, 20.0), (50.0, 10.0), (100.0, 0.0)]);
    // All zero: along the bottom, no division by zero.
    assert_eq!(spark_points(&[0, 0], 10.0, 4.0), [(0.0, 4.0), (10.0, 4.0)]);
    // One value: a point at the left.
    assert_eq!(spark_points(&[3], 10.0, 4.0), [(0.0, 0.0)]);
    assert_eq!(spark_points(&[], 10.0, 4.0), []);
}

#[test]
fn an_hour_is_labelled_in_local_time() {
    // 1601-01-01 00:00 UTC is the start of the tick count.
    assert_eq!(hour_label(0, 0), "00:00");
    assert_eq!(hour_label(13 * TICKS_PER_HOUR, 0), "13:00");
    assert_eq!(hour_label(13 * TICKS_PER_HOUR, 2 * 3_600), "15:00");
    assert_eq!(hour_label(TICKS_PER_HOUR, -2 * 3_600), "23:00");
}

// --- the panel over a real log ---

struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("pb-stats-ui-{tag}-{}", std::process::id()));
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

fn event(ts: u64, kind: Kind) -> Event {
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

/// Events in the hour before `now` (HOUR * 100) on one root.
fn log(dir: &Path) -> Writer {
    let day_of: DayFn = Arc::new(|_| DayKey {
        year: 2026,
        month: 10,
        day: 1,
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
    let mut batch = Batch::with_capacity(64, 8192);
    for i in 0..40_u64 {
        batch
            .push(
                event(99 * HOUR + i, Kind::Modified),
                &units(r"logs\app.log"),
            )
            .unwrap();
    }
    for i in 0..3_u64 {
        batch
            .push(
                event(99 * HOUR + 100 + i, Kind::Created),
                &units("note.txt"),
            )
            .unwrap();
    }
    writer.write(&[batch], &[]).unwrap();
    writer.flush_churn().unwrap();
    writer
}

type State = StatsPanel;

fn harness(dir: &Path) -> Harness<'static, State> {
    let panel = StatsPanel::new(dir.to_path_buf(), Arc::new(|| {}), Arc::new(|_| 0), || {
        100 * HOUR + 5
    });
    Harness::new_ui_state(|ui, panel: &mut State| panel.show(ui), panel)
}

fn wait_for(harness: &mut Harness<'static, State>, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        harness.step();
        if !harness.state().lock().counts.is_empty() {
            harness.run_steps(2);
            return;
        }
        assert!(Instant::now() < deadline, "{what} never came");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn the_panel_shows_events_per_hour_and_the_busiest_of_the_latest_hour() {
    let dir = Dir::new("flow");
    let _writer = log(dir.path());
    let mut harness = harness(dir.path());
    harness.run_steps(2);
    wait_for(&mut harness, "the stats");
    harness.get_by_label("Events per hour, last 24 hours");
    harness.get_by_label(
        "Events per hour, C:\\Watched: 43 events, busiest hour 99:00 with 43"
            .replace("99:00", &hour_label(99 * TICKS_PER_HOUR, 0))
            .as_str(),
    );
    harness
        .get_by_label("Counts are approximate (a sketch of the busiest 100 per hour and watch).");
    harness.get_by_label("40  C:\\Watched\\logs\\app.log");
    harness.get_by_label("40  LOG");
    harness.get_by_label("3  TXT");
    crate::a11y::assert_every_control_is_named(&harness);
}

#[test]
fn an_empty_log_says_so() {
    let dir = Dir::new("empty");
    let mut harness = harness(dir.path());
    harness.run_steps(3);
    std::thread::sleep(Duration::from_millis(300));
    harness.run_steps(3);
    harness.get_by_label("No events recorded in the last 24 hours.");
    harness.get_by_label("Nothing to rank yet.");
}
