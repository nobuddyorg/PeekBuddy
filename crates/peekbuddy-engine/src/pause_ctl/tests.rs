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
    let context = Context {
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

fn context() -> Context {
    Context {
        data_dir: String::from("C:/data"),
        export_dir: None,
    }
}

#[test]
fn an_added_watch_is_saved_listed_unarmed_and_removing_it_leaves_no_row() {
    let (ctl, rows, path) = ctl("add", vec![watch("a", true)], true);
    let (added, armed) = ctl.add_watch("C:/added", false, &context()).unwrap();
    assert!(!armed, "nothing armed it");
    let listed = ctl.list_with(|_| Some(WatchState::Live));
    assert_eq!(listed.len(), 2);
    assert!(listed[0].armed && !listed[1].armed);
    assert_eq!(listed[0].state, WatchState::Live);
    assert_eq!(listed[1].state, WatchState::Arming);
    assert_eq!(reload(&path).watches.len(), 2);
    ctl.remove_watch(&added.watch.id).unwrap();
    assert_eq!(rows.states(), []);
    assert_eq!(ctl.list_with(|_| None).len(), 1);
    assert_eq!(reload(&path).watches.len(), 1);
}

#[test]
fn removing_an_armed_watch_pauses_it_at_once_and_keeps_the_other_positions() {
    let (ctl, rows, path) = ctl("remove", vec![watch("a", true), watch("b", true)], true);
    ctl.remove_watch("a").unwrap();
    assert_eq!(rows.states(), [(0, WatchState::SuspendedByUser, 0)]);
    let ids: Vec<String> = ctl
        .list_with(|_| Some(WatchState::Live))
        .into_iter()
        .map(|watch| watch.id)
        .collect();
    assert_eq!(ids, ["b"]);
    // `b` is still root 1 of the running host.
    ctl.set_watch("b", true).unwrap();
    assert_eq!(
        rows.states().last(),
        Some(&(1, WatchState::SuspendedByUser, 0))
    );
    let saved = reload(&path);
    assert_eq!(saved.watches.len(), 1);
    assert_eq!(saved.watches[0].id, "b");
    assert!(!saved.watches[0].enabled);
    assert!(ctl.set_watch("a", false).is_err(), "a is gone");
    // The same folder can be added again while the old root is still armed.
    let (again, _) = ctl.add_watch("C:/watched/a", false, &context()).unwrap();
    assert!(again.watch.id.starts_with("a-"), "{}", again.watch.id);
    assert_eq!(ctl.list_with(|_| None).len(), 2);
}

/// The arming the engine binds, as a fake: records what it was asked and
/// answers `take`.
fn bind_fake(ctl: &PauseCtl, take: bool) -> Arc<Mutex<Vec<(u16, String)>>> {
    let asked: Arc<Mutex<Vec<(u16, String)>>> = Arc::default();
    let record = Arc::clone(&asked);
    ctl.bind_arming(Box::new(move |root, watch| {
        record.lock().unwrap().push((root.0, watch.path.clone()));
        take
    }));
    asked
}

#[test]
fn an_added_watch_the_engine_takes_is_armed_at_the_next_position() {
    let (ctl, rows, _path) = ctl("arm", vec![watch("a", true)], true);
    let asked = bind_fake(&ctl, true);
    let (added, armed) = ctl.add_watch("C:/added", false, &context()).unwrap();
    assert!(armed);
    assert_eq!(
        *asked.lock().unwrap(),
        [(1, added.watch.path.clone())],
        "the new watch is root 1"
    );
    let listed = ctl.list_with(|_| Some(WatchState::Live));
    assert!(listed.iter().all(|watch| watch.armed), "{listed:?}");
    assert_eq!(listed[1].state, WatchState::Live);
    // Arming writes no row of its own: the lifecycle machine does.
    assert_eq!(rows.states(), []);
    // It pauses and resumes like a watch armed at start.
    ctl.set_watch(&added.watch.id, true).unwrap();
    assert_eq!(rows.states(), [(1, WatchState::SuspendedByUser, 0)]);
    assert_eq!(
        ctl.list_with(|_| Some(WatchState::Live))[1].state,
        WatchState::SuspendedByUser
    );
}

#[test]
fn an_added_watch_the_engine_cannot_take_waits_for_a_restart() {
    let (ctl, _rows, _path) = ctl("arm-refused", vec![watch("a", true)], true);
    let asked = bind_fake(&ctl, false);
    let (_, armed) = ctl.add_watch("C:/added", false, &context()).unwrap();
    assert!(!armed);
    assert_eq!(asked.lock().unwrap().len(), 1);
    let listed = ctl.list_with(|_| Some(WatchState::Live));
    assert!(!listed[1].armed);
    assert_eq!(listed[1].state, WatchState::Arming);
}

#[test]
fn a_watch_added_during_a_global_pause_is_paused_with_its_row() {
    let (ctl, rows, _path) = ctl("arm-paused", vec![watch("a", true)], true);
    let _asked = bind_fake(&ctl, true);
    ctl.pause_all();
    let (_, armed) = ctl.add_watch("C:/added", false, &context()).unwrap();
    assert!(armed);
    assert!(ctl.pause().is_paused(RootIdx(1)));
    assert_eq!(
        rows.states(),
        [
            (0, WatchState::SuspendedByUser, 0),
            (1, WatchState::SuspendedByUser, 0)
        ]
    );
    ctl.pause().note_missed(RootIdx(1));
    ctl.resume_all();
    assert_eq!(rows.states().last(), Some(&(1, WatchState::Live, 1)));
}

#[test]
fn removing_a_watch_armed_while_running_pauses_it_and_keeps_its_position() {
    let (ctl, rows, path) = ctl("arm-remove", vec![watch("a", true)], true);
    let _asked = bind_fake(&ctl, true);
    let (added, _) = ctl.add_watch("C:/added", false, &context()).unwrap();
    let (later, _) = ctl.add_watch("C:/later", false, &context()).unwrap();
    ctl.remove_watch(&added.watch.id).unwrap();
    // It stops recording at once, with a row, like a watch armed at start.
    assert_eq!(rows.states(), [(1, WatchState::SuspendedByUser, 0)]);
    assert!(ctl.pause().is_paused(RootIdx(1)));
    let ids: Vec<String> = ctl
        .list_with(|_| Some(WatchState::Live))
        .into_iter()
        .map(|watch| watch.id)
        .collect();
    assert_eq!(ids, ["a".to_owned(), later.watch.id.clone()]);
    assert_eq!(reload(&path).watches.len(), 2);
    // The later watch is still root 2 of the running host.
    ctl.set_watch(&later.watch.id, true).unwrap();
    assert_eq!(
        rows.states().last(),
        Some(&(2, WatchState::SuspendedByUser, 0))
    );
}

#[test]
fn removing_a_watch_that_waits_for_a_restart_keeps_the_positions_after_it() {
    use std::sync::atomic::{AtomicBool, Ordering};

    let (ctl, rows, path) = ctl("arm-shift", vec![watch("a", true)], true);
    let take = Arc::new(AtomicBool::new(false));
    let answer = Arc::clone(&take);
    ctl.bind_arming(Box::new(move |_, _| answer.load(Ordering::Acquire)));
    let (waiting, armed) = ctl.add_watch("C:/waiting", false, &context()).unwrap();
    assert!(!armed);
    take.store(true, Ordering::Release);
    let (live, armed) = ctl.add_watch("C:/live", false, &context()).unwrap();
    assert!(armed);
    ctl.remove_watch(&waiting.watch.id).unwrap();
    assert_eq!(rows.states(), [], "a watch that was not armed has no row");
    let listed = ctl.list_with(|_| Some(WatchState::Live));
    assert_eq!(listed.len(), 2);
    assert!(listed[1].armed && listed[1].id == live.watch.id);
    assert_eq!(reload(&path).watches.len(), 2);
    // The armed watch is still root 2 of the running host.
    ctl.set_watch(&live.watch.id, true).unwrap();
    assert_eq!(rows.states(), [(2, WatchState::SuspendedByUser, 0)]);
}

#[test]
fn a_paused_watch_is_listed_as_suspended_with_what_it_missed() {
    let (ctl, _rows, _path) = ctl("listpaused", vec![watch("a", true), watch("b", true)], true);
    ctl.set_watch("a", true).unwrap();
    ctl.pause().note_missed(RootIdx(0));
    ctl.pause().note_missed(RootIdx(0));
    let listed = ctl.list_with(|_| Some(WatchState::Live));
    assert_eq!(listed[0].state, WatchState::SuspendedByUser);
    assert_eq!(listed[0].missed_while_paused, 2);
    assert_eq!(listed[1].state, WatchState::Live);
    assert_eq!(listed[1].missed_while_paused, 0);
}

#[test]
fn a_file_from_a_newer_version_is_not_edited() {
    let (ctl, _rows, path) = ctl("newer", vec![watch("a", true)], false);
    let before = std::fs::read_to_string(&path).unwrap();
    assert!(matches!(
        ctl.add_watch("C:/x", false, &context()),
        Err(EditError::Failed(_))
    ));
    assert!(matches!(ctl.remove_watch("a"), Err(EditError::Failed(_))));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
}

#[test]
fn applied_settings_keep_the_watches_and_a_later_pause_does_not_undo_them() {
    use peekbuddy_core::settings::Theme;

    let (ctl, _rows, path) = ctl("apply", vec![watch("a", true)], true);
    let mut new = Settings::default();
    new.general.theme = Theme::Dark;
    new.store.max_age_days = Some(7);
    // The viewer's text never changes the watches.
    new.watches = vec![watch("elsewhere", true)];
    ctl.apply_settings(&new).unwrap();
    let saved = reload(&path);
    assert_eq!(saved.general.theme, Theme::Dark);
    assert_eq!(saved.store.max_age_days, Some(7));
    let ids: Vec<&str> = saved.watches.iter().map(|w| w.id.as_str()).collect();
    assert_eq!(ids, ["a"]);
    // Pausing writes the file from the control's own state, which holds the
    // new values too.
    ctl.set_watch("a", true).unwrap();
    let again = reload(&path);
    assert_eq!(again.store.max_age_days, Some(7));
    assert_eq!(again.general.theme, Theme::Dark);
    assert!(!again.watches[0].enabled);
}

#[test]
fn settings_from_a_newer_version_are_not_applied() {
    let (ctl, _rows, path) = ctl("apply-newer", vec![watch("a", true)], false);
    let before = std::fs::read_to_string(&path).unwrap();
    assert!(ctl.apply_settings(&Settings::default()).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
}
