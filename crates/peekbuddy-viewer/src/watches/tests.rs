use std::collections::HashSet;
use std::fmt::Write as _;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use peekbuddy_core::event::WatchState;
use peekbuddy_core::ipc::WatchInfo;

use super::{Tab, WatchAction, WatchPanel, display_path, present, present_state, tabs};

fn watch(id: &str, path: &str, state: WatchState, armed: bool) -> WatchInfo {
    WatchInfo {
        id: id.to_owned(),
        path: path.to_owned(),
        recursive: true,
        state,
        armed,
        missed_while_paused: 0,
    }
}

type State = (WatchPanel, Vec<WatchAction>);

/// Types into the path field the way a user does: focus first.
fn type_path(harness: &mut Harness<'static, State>, text: &str) {
    harness.get_by_label("Watch a folder").focus();
    harness.run();
    harness.get_by_label("Watch a folder").type_text(text);
    harness.run();
}

fn harness(watches: Vec<WatchInfo>) -> Harness<'static, State> {
    let mut panel = WatchPanel::default();
    panel.set_watches(watches);
    Harness::new_ui_state(
        |ui, (panel, taken): &mut State| {
            taken.extend(panel.show(ui));
        },
        (panel, Vec::new()),
    )
}

#[test]
fn every_state_has_a_label_an_icon_and_a_tooltip_of_its_own() {
    let mut labels = HashSet::new();
    let mut tooltips = HashSet::new();
    let mut table = String::new();
    for state in WatchState::ALL {
        let shown = present_state(state);
        assert_ne!(shown.label.len(), 0, "{state:?}");
        assert!(shown.tooltip.len() > 30, "{state:?} explains itself");
        assert!(labels.insert(shown.label), "{state:?} label is unique");
        assert!(
            tooltips.insert(shown.tooltip),
            "{state:?} tooltip is unique"
        );
        writeln!(
            table,
            "{state:?}\n  label:   {}\n  icon:    {:?}\n  tooltip: {}",
            shown.label, shown.icon, shown.tooltip
        )
        .unwrap();
    }
    let waiting = present(&watch("w", "C:\\x", WatchState::Live, false));
    writeln!(
        table,
        "not armed yet\n  label:   {}\n  icon:    {:?}\n  tooltip: {}",
        waiting.label, waiting.icon, waiting.tooltip
    )
    .unwrap();
    assert!(labels.insert(waiting.label));
    // Colour is never the only signal: two states share a shape (a lock) only
    // because the system holds the drive in both, and their words differ.
    let icons: HashSet<_> = WatchState::ALL
        .iter()
        .map(|s| present_state(*s).icon)
        .collect();
    assert_eq!(icons.len(), WatchState::ALL.len() - 1);
    insta::assert_snapshot!(table);
}

#[test]
fn a_watch_is_shown_with_its_state_and_a_readable_path() {
    let mut harness = harness(vec![
        watch("docs", r"\\?\C:\Users\me\docs", WatchState::Live, true),
        watch("nas", r"\\?\UNC\nas\team", WatchState::Lost, true),
        watch("new", r"\\?\D:\new", WatchState::Arming, false),
    ]);
    harness.run();
    harness.get_by_label("Live");
    harness.get_by_label(r"C:\Users\me\docs");
    harness.get_by_label("Lost");
    harness.get_by_label(r"\\nas\team");
    harness.get_by_label("Starts after restart");
    harness
        .get_by_label_contains("1 saved folder starts recording when PeekBuddy's engine restarts");
    assert_eq!(harness.state().0.attention(), 1);
    assert_eq!(harness.state().0.waiting(), 1);
}

#[test]
fn an_empty_list_says_how_to_start() {
    let mut harness = harness(Vec::new());
    harness.run();
    harness.get_by_label_contains("Nothing is watched yet");
}

#[test]
fn verbatim_prefixes_are_not_shown() {
    assert_eq!(display_path(r"\\?\C:\a"), r"C:\a");
    assert_eq!(display_path(r"\\?\UNC\srv\share\x"), r"\\srv\share\x");
    assert_eq!(display_path(r"C:\plain"), r"C:\plain");
}

#[test]
fn typing_a_path_and_adding_asks_the_engine_and_a_good_answer_clears_the_field() {
    let mut harness = harness(Vec::new());
    harness.run();
    type_path(&mut harness, r"C:\Work");
    harness.run();
    harness.get_by_label("Add").click();
    harness.run();
    assert_eq!(
        harness.state().1,
        [WatchAction::Add {
            path: String::from(r"C:\Work"),
            confirmed: false
        }]
    );
    harness.state_mut().0.on_result(
        true,
        "added `work-1`; it starts capturing with the next engine start",
    );
    harness.run();
    harness.get_by_label_contains("added `work-1`");
    harness.get_by_label("Done:");
    assert_eq!(
        harness.get_by_label("Watch a folder").value().as_deref(),
        Some("")
    );
}

#[test]
fn a_refusal_is_shown_and_the_path_stays_for_editing() {
    let mut harness = harness(Vec::new());
    harness.run();
    type_path(&mut harness, r"C:\Data");
    harness.run();
    harness.get_by_label("Add").click();
    harness.run();
    harness.state_mut().0.on_result(
        false,
        "That folder is PeekBuddy's own data folder (or inside it).",
    );
    harness.run();
    harness.get_by_label("Not done:");
    harness.get_by_label_contains("own data folder");
    assert_eq!(
        harness.get_by_label("Watch a folder").value().as_deref(),
        Some(r"C:\Data")
    );
}

#[test]
fn a_whole_drive_needs_a_second_click_and_cancel_changes_nothing() {
    let mut harness = harness(Vec::new());
    harness.run();
    type_path(&mut harness, r"C:\");
    harness.run();
    harness.get_by_label("Add").click();
    harness.run();
    harness.state_mut().1.clear();
    harness
        .state_mut()
        .0
        .on_result(false, "confirm: This is a whole drive. It is noisy.");
    harness.run();
    harness.get_by_label("Watch this folder?");
    harness.get_by_label_contains("whole drive");
    harness.get_by_label("Cancel").click();
    harness.run();
    assert!(harness.query_by_label("Watch this folder?").is_none());
    assert_eq!(harness.state().1, [], "cancel sends nothing");

    // Again, and this time confirm.
    harness.get_by_label("Add").click();
    harness.run();
    harness.state_mut().1.clear();
    harness
        .state_mut()
        .0
        .on_result(false, "confirm: This is a whole drive. It is noisy.");
    harness.run();
    harness.get_by_label("Add anyway").click();
    harness.run();
    assert_eq!(
        harness.state().1,
        [WatchAction::Add {
            path: String::from(r"C:\"),
            confirmed: true
        }]
    );
}

#[test]
fn removing_asks_first_and_says_the_rows_stay() {
    let mut harness = harness(vec![watch("docs", r"\\?\C:\docs", WatchState::Live, true)]);
    harness.run();
    harness.get_by_label(r"Remove the watch on C:\docs").click();
    harness.run();
    harness.get_by_label("Remove this watch?");
    harness.get_by_label_contains("stay in the log");
    assert_eq!(harness.state().1, [], "nothing is sent before the answer");
    harness.get_all_by_label("Remove").last().unwrap().click();
    harness.run();
    assert_eq!(
        harness.state().1,
        [WatchAction::Remove(String::from("docs"))]
    );
}

#[test]
fn pause_and_resume_follow_the_state_and_a_paused_watch_shows_what_it_missed() {
    let mut paused = watch("a", r"\\?\C:\a", WatchState::SuspendedByUser, true);
    paused.missed_while_paused = 1234;
    let live = watch("b", r"\\?\C:\b", WatchState::Live, true);
    let mut harness = harness(vec![paused, live]);
    harness.run();
    harness.get_by_label("Paused");
    harness.get_by_label(&format!(
        "{} changes missed while paused",
        crate::view::group_digits(1234)
    ));
    harness.get_by_label(r"Resume the watch on C:\a").click();
    harness.get_by_label(r"Pause the watch on C:\b").click();
    harness.run();
    assert_eq!(
        harness.state().1,
        [
            WatchAction::SetPaused {
                id: String::from("a"),
                paused: false
            },
            WatchAction::SetPaused {
                id: String::from("b"),
                paused: true
            }
        ]
    );
}

#[test]
fn restart_and_browse_are_offered() {
    let mut harness = harness(vec![watch("n", r"\\?\C:\n", WatchState::Arming, false)]);
    harness.run();
    harness.get_by_label("Restart engine now").click();
    harness.get_by_label("Browse...").click();
    harness.run();
    assert_eq!(
        harness.state().1,
        [WatchAction::RestartEngine, WatchAction::Browse]
    );
    harness.state_mut().0.restarting = true;
    harness.run();
    harness.get_by_label("Restarting the engine...");
    assert!(harness.query_by_label("Restart engine now").is_none());
}

#[test]
fn a_picked_folder_is_added_unconfirmed() {
    let mut panel = WatchPanel::default();
    assert_eq!(
        panel.picked(String::from(r"D:\Photos")),
        WatchAction::Add {
            path: String::from(r"D:\Photos"),
            confirmed: false
        }
    );
    // A whole-drive answer to a picked folder asks about that folder.
    panel.on_result(false, "confirm: whole drive");
    assert!(panel.ask.is_some());
}

#[test]
fn the_tabs_switch_and_count_what_needs_attention() {
    let mut harness = Harness::new_ui_state(
        |ui, tab: &mut Tab| {
            tabs(ui, tab, 3, 1);
        },
        Tab::Log,
    );
    harness.run();
    harness.get_by_label("Log");
    harness
        .get_by_label("Watches (3, 1 need attention)")
        .click();
    harness.run();
    assert_eq!(*harness.state(), Tab::Watches);
}

#[test]
fn every_control_of_the_watches_tab_and_its_questions_has_a_name() {
    let mut harness = harness(vec![
        watch("a", r"\\?\C:\a", WatchState::Live, true),
        watch("b", r"\\?\C:\b", WatchState::SuspendedByUser, true),
        watch("c", r"\\?\C:\c", WatchState::Lost, true),
        watch("d", r"\\?\C:\d", WatchState::Arming, false),
    ]);
    harness.run();
    crate::a11y::assert_every_control_is_named(&harness);
    // The question about a whole drive is a window of its own.
    let _ = harness.state_mut().0.picked(String::from(r"D:\"));
    harness
        .state_mut()
        .0
        .on_result(false, "confirm: whole drive");
    harness.run();
    crate::a11y::assert_every_control_is_named(&harness);
    harness.get_by_label("Cancel").click();
    harness.run();
    harness.get_by_label(r"Remove the watch on C:\a").click();
    harness.run();
    crate::a11y::assert_every_control_is_named(&harness);
}
