use std::sync::{Arc, Mutex};

use peekbuddy_core::event::{Kind, WatchState};
use peekbuddy_core::settings::{Loaded, Watch, load, to_toml};

use super::*;

#[derive(Clone, Default)]
struct Rows(Arc<Mutex<Vec<Event>>>);

impl RowSink for Rows {
    fn row(&mut self, event: Event) {
        self.0.lock().unwrap().push(event);
    }
}

impl Rows {
    fn states(&self) -> Vec<(u16, WatchState, u64)> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e.kind {
                Kind::WatchStateChanged(state) => Some((e.root.0, state, e.size)),
                _ => None,
            })
            .collect()
    }
}

fn watch(id: &str, enabled: bool) -> Watch {
    Watch {
        id: id.to_owned(),
        path: format!("C:/watched/{id}"),
        enabled,
        ..Watch::default()
    }
}

fn temp(tag: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("peekbuddy-pause-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&path).unwrap();
    path.join("settings.toml")
}

fn ctl(tag: &str, watches: Vec<Watch>, writable: bool) -> (Arc<PauseCtl>, Rows, PathBuf) {
    let settings = Settings {
        watches,
        ..Settings::default()
    };
    let path = temp(tag);
    std::fs::write(&path, to_toml(&settings).unwrap()).unwrap();
    let rows = Rows::default();
    let ctl = PauseCtl::new(
        settings,
        path.clone(),
        writable,
        Box::new(rows.clone()),
        || (1, 2),
    );
    (ctl, rows, path)
}

#[expect(clippy::panic, reason = "a test helper fails the test with the cause")]
fn reload(path: &PathBuf) -> Settings {
    let text = std::fs::read_to_string(path).unwrap();
    let context = peekbuddy_core::settings::Context {
        data_dir: String::from("C:/data"),
        export_dir: None,
    };
    match load(&text, &context) {
        Loaded::Ok { settings, .. } => *settings,
        other => panic!("settings did not load: {other:?}"),
    }
}

#[test]
fn pause_all_and_resume_all_write_one_row_per_watch_with_the_missed_count() {
    let (ctl, rows, path) = ctl("all", vec![watch("a", true), watch("b", true)], true);
    ctl.pause_all();
    assert_eq!(
        rows.states(),
        [
            (0, WatchState::SuspendedByUser, 0),
            (1, WatchState::SuspendedByUser, 0)
        ]
    );
    for _ in 0..3 {
        ctl.pause().note_missed(RootIdx(1));
    }
    ctl.resume_all();
    let states = rows.states();
    assert_eq!(states.get(2), Some(&(0, WatchState::Live, 0)));
    assert_eq!(states.get(3), Some(&(1, WatchState::Live, 3)));
    // A global pause is not persisted.
    assert!(reload(&path).watches.iter().all(|w| w.enabled));
}

#[test]
fn a_single_watch_pause_is_persisted_and_logged_on_start() {
    let (ctl, rows, path) = ctl("one", vec![watch("a", true), watch("b", true)], true);
    ctl.set_watch("b", true).unwrap();
    assert_eq!(rows.states(), [(1, WatchState::SuspendedByUser, 0)]);
    assert!(!reload(&path).watches.get(1).unwrap().enabled);

    // The next start: the watch comes up paused and says so.
    let settings = reload(&path);
    let rows = Rows::default();
    let restarted = PauseCtl::new(settings, path.clone(), true, Box::new(rows.clone()), || {
        (1, 2)
    });
    assert!(restarted.pause().is_paused(RootIdx(1)));
    assert!(!restarted.pause().is_paused(RootIdx(0)));
    restarted.announce_start();
    assert_eq!(rows.states(), [(1, WatchState::SuspendedByUser, 0)]);
    restarted.set_watch("b", false).unwrap();
    assert!(reload(&path).watches.get(1).unwrap().enabled);
}

#[test]
fn an_unknown_watch_is_an_error_and_a_repeat_is_not_a_second_row() {
    let (ctl, rows, _) = ctl("unknown", vec![watch("a", true)], true);
    assert!(ctl.set_watch("nope", true).unwrap_err().contains("nope"));
    ctl.set_watch("a", true).unwrap();
    ctl.set_watch("a", true).unwrap();
    assert_eq!(rows.states().len(), 1);
    assert!(ctl.set_watch_at(5, true).is_err());
    ctl.set_watch_at(0, false).unwrap();
    assert_eq!(rows.states().len(), 2);
}

#[test]
fn a_newer_settings_file_is_never_written() {
    let (ctl, rows, path) = ctl("newer", vec![watch("a", true)], false);
    let before = std::fs::read(&path).unwrap();
    let error = ctl.set_watch("a", true).unwrap_err();
    assert!(error.contains("newer version"), "{error}");
    assert_eq!(std::fs::read(&path).unwrap(), before);
    // The pause itself took effect, with its row.
    assert!(ctl.pause().is_paused(RootIdx(0)));
    assert_eq!(rows.states().len(), 1);
}

#[test]
fn the_clock_reads_wall_time_in_filetime_ticks_and_a_rising_monotonic_count() {
    use std::time::{SystemTime, UNIX_EPOCH};

    /// 100 ns ticks from 1601-01-01 to 1970-01-01.
    const UNIX_EPOCH_TICKS: u64 = 116_444_736_000_000_000;
    let (wall, mono) = now_ticks();
    let expected = UNIX_EPOCH_TICKS
        + u64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
                / 100,
        )
        .unwrap();
    // Within a second of the system clock.
    assert!(wall.abs_diff(expected) < 10_000_000, "{wall} vs {expected}");
    std::thread::sleep(std::time::Duration::from_millis(5));
    let (later_wall, later_mono) = now_ticks();
    assert!(later_wall > wall);
    assert!(later_mono > mono);
    // 5 ms are at least 50,000 ticks.
    assert!(later_mono - mono >= 50_000, "{mono} -> {later_mono}");
}

#[test]
fn the_pause_control_describes_itself_in_debug_output() {
    let ctl = PauseCtl::new(
        Settings::default(),
        std::env::temp_dir().join("pb-pausectl-debug.toml"),
        false,
        Box::new(Rows::default()),
        now_ticks,
    );
    let text = format!("{ctl:?}");
    assert!(text.contains("PauseCtl"), "{text}");
    assert!(text.contains("global: false"), "{text}");
}
