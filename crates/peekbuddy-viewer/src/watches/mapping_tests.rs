//! The state-to-screen mapping of the watches (#68): which watches need a
//! look, which wait for a restart, what a row says about its state, and the
//! boundaries of each rule. Written against the survivors of cargo-mutants.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use peekbuddy_core::event::WatchState;
use peekbuddy_core::ipc::WatchInfo;

use super::{Tab, WatchAction, WatchPanel, tabs};

fn watch(id: &str, state: WatchState, armed: bool) -> WatchInfo {
    WatchInfo {
        id: id.to_owned(),
        path: format!(r"\\?\C:\{id}"),
        recursive: true,
        state,
        armed,
        missed_while_paused: 0,
    }
}

type State = (WatchPanel, Vec<WatchAction>);

fn panel_harness(watches: Vec<WatchInfo>) -> Harness<'static, State> {
    let mut panel = WatchPanel::default();
    panel.set_watches(watches);
    let mut harness = Harness::new_ui_state(
        |ui, (panel, taken): &mut State| {
            taken.extend(panel.show(ui));
        },
        (panel, Vec::new()),
    );
    harness.run();
    harness
}

#[test]
fn only_armed_watches_that_lost_a_change_or_wait_for_a_drive_need_attention() {
    let mut panel = WatchPanel::default();
    assert_eq!(panel.attention(), 0);
    panel.set_watches(vec![
        watch("live", WatchState::Live, true),
        watch("paused", WatchState::SuspendedByUser, true),
        watch("scanning", WatchState::Reconciling, true),
        watch("restored", WatchState::Restored, true),
        // Not armed: its state is a placeholder, whatever it says.
        watch("waiting", WatchState::Lost, false),
    ]);
    assert_eq!(panel.attention(), 0);
    panel.set_watches(vec![
        watch("lost", WatchState::Lost, true),
        watch("overflowed", WatchState::Overflowed, true),
        watch("locked", WatchState::SuspendedLocked, true),
        watch("live", WatchState::Live, true),
        watch("waiting", WatchState::Lost, false),
    ]);
    assert_eq!(panel.attention(), 3);
}

#[test]
fn only_watches_that_are_not_armed_wait_for_a_restart() {
    let mut panel = WatchPanel::default();
    assert_eq!(panel.waiting(), 0);
    panel.set_watches(vec![
        watch("a", WatchState::Live, true),
        watch("b", WatchState::Lost, true),
    ]);
    assert_eq!(panel.waiting(), 0);
    panel.set_watches(vec![
        watch("a", WatchState::Live, true),
        watch("b", WatchState::Arming, false),
        watch("c", WatchState::Arming, false),
    ]);
    assert_eq!(panel.waiting(), 2);
}

#[test]
fn the_restart_offer_appears_only_while_a_saved_watch_waits() {
    let harness = panel_harness(vec![watch("a", WatchState::Live, true)]);
    assert!(harness.query_by_label("Restart engine now").is_none());
    assert!(
        harness
            .query_by_label_contains("start recording when PeekBuddy's engine restarts")
            .is_none()
    );
    let harness = panel_harness(vec![
        watch("a", WatchState::Live, true),
        watch("b", WatchState::Arming, false),
        watch("c", WatchState::Arming, false),
    ]);
    harness.get_by_label("Restart engine now");
    harness
        .get_by_label_contains("2 saved folders start recording when PeekBuddy's engine restarts");
}

#[test]
fn a_watch_of_one_folder_only_says_so() {
    let mut flat = watch("flat", WatchState::Live, true);
    flat.recursive = false;
    let harness = panel_harness(vec![flat]);
    harness.get_by_label("(this folder only)");
    let harness = panel_harness(vec![watch("deep", WatchState::Live, true)]);
    assert!(harness.query_by_label("(this folder only)").is_none());
}

#[test]
fn missed_changes_are_shown_only_for_an_armed_paused_watch_that_missed_some() {
    let with = |state, armed, missed| {
        let mut info = watch("w", state, armed);
        info.missed_while_paused = missed;
        info
    };
    let missed = |info: WatchInfo| {
        let harness = panel_harness(vec![info]);
        harness
            .query_by_label_contains("changes missed while paused")
            .is_some()
    };
    assert!(missed(with(WatchState::SuspendedByUser, true, 7)));
    // Paused, but nothing passed yet.
    assert!(!missed(with(WatchState::SuspendedByUser, true, 0)));
    // Missed changes of a watch that is not paused are not a thing to show.
    assert!(!missed(with(WatchState::Live, true, 7)));
    // A watch the engine has not armed has no pause to speak of.
    assert!(!missed(with(WatchState::SuspendedByUser, false, 7)));
}

#[test]
fn a_new_result_clears_the_field_only_for_a_successful_add() {
    let mut harness = panel_harness(Vec::new());
    harness.get_by_label("Watch a folder").focus();
    harness.run();
    harness.get_by_label("Watch a folder").type_text(r"C:\keep");
    harness.run();
    let typed = |harness: &Harness<'static, State>| harness.state().0.input.clone();
    assert_eq!(typed(&harness), r"C:\keep");
    // Not an add that worked: the path stays for editing.
    harness.state_mut().0.on_result(true, "removed w0");
    assert_eq!(typed(&harness), r"C:\keep");
    harness.state_mut().0.on_result(false, "added nothing");
    assert_eq!(typed(&harness), r"C:\keep");
    harness.state_mut().0.on_result(true, "added w1");
    assert_eq!(typed(&harness), "");
}

#[test]
fn the_first_run_adder_sends_what_the_user_types_and_picks() {
    let mut harness = Harness::new_ui_state(
        |ui, (panel, taken): &mut State| {
            taken.extend(panel.show_adder(ui));
        },
        (WatchPanel::default(), Vec::new()),
    );
    harness.run();
    harness.get_by_label("Watch a folder").focus();
    harness.run();
    harness
        .get_by_label("Watch a folder")
        .type_text(r"D:\Photos");
    harness.run();
    harness.get_by_label("Add").click();
    harness.get_by_label("Browse...").click();
    harness.run();
    assert_eq!(
        harness.state().1,
        [
            WatchAction::Add {
                path: String::from(r"D:\Photos"),
                confirmed: false
            },
            WatchAction::Browse
        ]
    );
}

#[test]
fn the_tab_strip_counts_attention_only_when_there_is_some() {
    let mut harness = Harness::new_ui_state(
        |ui, tab: &mut Tab| {
            tabs(ui, tab, 3, 0);
        },
        Tab::Log,
    );
    harness.run();
    harness.get_by_label("Watches (3)");
    assert!(harness.query_by_label_contains("need attention").is_none());
}
