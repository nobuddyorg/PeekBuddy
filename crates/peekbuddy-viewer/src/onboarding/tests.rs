use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use peekbuddy_core::event::WatchState;
use peekbuddy_core::ipc::WatchInfo;

use super::{Onboarding, OnboardingAction, PRIVACY_HEADLINE, PRIVACY_STATEMENT, Step};
use crate::watches::WatchAction;

type State = (Onboarding, Vec<WatchInfo>, Vec<OnboardingAction>);

fn harness() -> Harness<'static, State> {
    Harness::builder()
        .with_size(egui::vec2(800.0, 900.0))
        .build_ui_state(
            |ui, (screens, watches, taken): &mut State| {
                taken.extend(screens.show(ui, watches));
            },
            (Onboarding::default(), Vec::new(), Vec::new()),
        )
}

fn words(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn the_privacy_statement_is_the_one_of_the_readme_and_privacy_md() {
    let readme = words(include_str!("../../../../README.md"));
    let privacy = words(include_str!("../../../../PRIVACY.md"));
    assert!(PRIVACY_STATEMENT.starts_with(PRIVACY_HEADLINE));
    assert!(
        readme.contains(PRIVACY_HEADLINE),
        "README says it differently"
    );
    assert!(
        privacy.contains(PRIVACY_HEADLINE),
        "PRIVACY.md says it differently"
    );
    assert!(readme.contains("PeekBuddy makes no network connections"));
    assert!(privacy.contains(
        "PeekBuddy makes no network connections: no telemetry, no update checks, no accounts."
    ));
    assert!(
        PRIVACY_STATEMENT
            .contains("makes no network connections: no telemetry, no update checks, no accounts")
    );
}

#[test]
fn the_steps_run_from_welcome_to_done_and_back() {
    let mut harness = harness();
    harness.run();
    harness.get_by_label("Welcome to PeekBuddy");
    harness.get_by_label("Step 1 of 5");
    harness.get_by_label_contains("never leaves your disk");
    for (step, heading) in [
        (2, "Which folders should PeekBuddy watch?"),
        (3, "Where is PeekBuddy when this window is closed?"),
        (4, "Start with Windows?"),
        (5, "You are all set"),
    ] {
        harness.get_by_label("Next").click();
        harness.run();
        harness.get_by_label(&format!("Step {step} of 5"));
        harness.get_by_label(heading);
    }
    assert_eq!(harness.state().0.step, Step::Done);
    harness.get_by_label("Back").click();
    harness.run();
    assert_eq!(harness.state().0.step, Step::Autostart);
}

#[test]
fn the_tray_step_explains_the_overflow_flyout() {
    let mut harness = harness();
    harness.state_mut().0.step = Step::Tray;
    harness.run();
    harness.get_by_label_contains("overflow flyout");
    harness.get_by_label_contains("onto the taskbar");
}

#[test]
fn skipping_or_finishing_with_the_defaults_turns_nothing_on() {
    let mut harness = harness();
    harness.run();
    harness.get_by_label("Skip").click();
    harness.run();
    assert_eq!(
        harness.state().2,
        [OnboardingAction::Finish {
            autostart: false,
            presets: Vec::new()
        }]
    );
    // Walking through every step and finishing gives the same.
    harness.state_mut().2.clear();
    harness.state_mut().0.step = Step::Done;
    harness.run();
    harness.get_by_label("Open the log").click();
    harness.run();
    assert_eq!(
        harness.state().2,
        [OnboardingAction::Finish {
            autostart: false,
            presets: Vec::new()
        }]
    );
}

#[test]
fn autostart_and_presets_are_remembered_only_when_ticked() {
    let mut harness = harness();
    harness.state_mut().0.step = Step::Folders;
    harness.run();
    harness
        .get_by_label_contains("Git internals - hidden in view (recorded)")
        .click();
    harness.get_by_label("Next").click();
    harness.run();
    harness.get_by_label("Next").click();
    harness.run();
    harness
        .get_by_label("Start PeekBuddy when I sign in to Windows")
        .click();
    harness.run();
    harness.get_by_label("Skip").click();
    harness.run();
    assert_eq!(
        harness.state().2,
        [OnboardingAction::Finish {
            autostart: true,
            presets: vec![String::from("git-internals")]
        }]
    );
}

#[test]
fn folders_are_added_through_the_same_flow_as_the_watches_tab() {
    let mut harness = harness();
    harness.state_mut().0.step = Step::Folders;
    harness.run();
    harness.get_by_label("Browse...").click();
    harness.run();
    assert_eq!(
        harness.state().2,
        [OnboardingAction::Watch(WatchAction::Browse)]
    );
    harness.state_mut().1 = vec![WatchInfo {
        id: String::from("docs-1"),
        path: String::from(r"\\?\C:\docs"),
        recursive: true,
        state: WatchState::Arming,
        armed: false,
        missed_while_paused: 0,
    }];
    harness.run();
    harness.get_by_label(r"Watching C:\docs");
}

#[test]
fn the_screens_work_from_the_keyboard() {
    let mut harness = harness();
    harness.run();
    harness.get_by_label("Next").focus();
    harness.run();
    harness.key_press(egui::Key::Enter);
    harness.run();
    assert_eq!(harness.state().0.step, Step::Folders);
}

#[test]
fn every_control_of_every_step_has_a_name() {
    let mut harness = harness();
    for step in [
        Step::Welcome,
        Step::Folders,
        Step::Tray,
        Step::Autostart,
        Step::Done,
    ] {
        harness.state_mut().0.step = step;
        harness.run();
        crate::a11y::assert_every_control_is_named(&harness);
    }
}
