//! Autostart against the registry (#63): a throw-away key under
//! `HKCU\Software\nobuddy`, and, only on a disposable runner, the real Run key.
#![cfg(windows)]

use peekbuddy_core::autostart::{APPROVED_ENABLED, Autostart, Registry, Status, run_value};
use peekbuddy_win::autostart::RunKeys;

const EXE: &str = "C:/Program Files/PeekBuddy/peekbuddy.exe";

/// A test key removed on drop, so a failing test leaves nothing behind.
struct TestBase(String);

impl TestBase {
    fn new(tag: &str) -> Self {
        let base = format!(
            "Software/nobuddy/PeekBuddy-test-{tag}-{}",
            std::process::id()
        )
        .replace('/', "\\");
        Self(base)
    }

    fn keys(&self) -> RunKeys {
        RunKeys::under(&self.0)
    }
}

impl Drop for TestBase {
    fn drop(&mut self) {
        let _ = RunKeys::delete_tree(&self.0);
    }
}

#[test]
fn the_toggle_writes_and_removes_the_quoted_value() {
    let base = TestBase::new("toggle");
    let mut auto = Autostart::new(base.keys(), EXE);
    assert_eq!(auto.status(), Status::Off);
    assert_eq!(auto.set(true).unwrap(), Status::On);
    assert_eq!(auto.registry().run_value(), Some(run_value(EXE)));
    // A second handle sees the same registry.
    assert_eq!(Autostart::new(base.keys(), EXE).status(), Status::On);
    assert_eq!(auto.set(false).unwrap(), Status::Off);
    assert_eq!(auto.registry().run_value(), None);
    assert_eq!(auto.set(false).unwrap(), Status::Off);
}

#[test]
fn a_moved_exe_is_rewritten_at_start_when_the_intent_is_on() {
    let base = TestBase::new("moved");
    Autostart::new(base.keys(), "D:/old/peekbuddy.exe")
        .set(true)
        .unwrap();
    let mut now = Autostart::new(base.keys(), EXE);
    assert!(matches!(now.status(), Status::OtherPath { .. }));
    assert_eq!(now.on_start(true).unwrap(), Status::On);
    assert_eq!(now.registry().run_value(), Some(run_value(EXE)));
}

#[test]
fn a_task_manager_disabled_entry_is_reported_and_left_alone() {
    let base = TestBase::new("disabled");
    let mut keys = base.keys();
    keys.set_run_value(&run_value(EXE)).unwrap();
    // What Task Manager writes when the user disables the entry.
    let disabled = vec![3, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8];
    keys.set_approved(&disabled).unwrap();

    let mut auto = Autostart::new(base.keys(), EXE);
    assert_eq!(auto.on_start(true).unwrap(), Status::DisabledInTaskManager);
    // The value in the registry is untouched after the engine start.
    assert_eq!(auto.registry().approved(), Some(disabled));
    // Only an explicit toggle brings it back.
    assert_eq!(auto.set(true).unwrap(), Status::On);
    assert_eq!(auto.registry().approved(), Some(APPROVED_ENABLED.to_vec()));
}

#[test]
fn the_value_survives_spaces_and_non_ascii() {
    let base = TestBase::new("unicode");
    let exe = "C:/Users/Jürgen Müller/PeekBuddy/peekbuddy.exe";
    let mut auto = Autostart::new(base.keys(), exe);
    auto.set(true).unwrap();
    assert_eq!(auto.registry().run_value(), Some(run_value(exe)));
}

/// Touches the real Run key; only on a disposable machine (the CI runner),
/// which sets `PEEKBUDDY_TOUCH_REAL_RUN_KEY=1` and removes the value in an
/// `always()` step.
#[test]
fn the_real_run_key_round_trips_on_a_disposable_runner() {
    if std::env::var_os("PEEKBUDDY_TOUCH_REAL_RUN_KEY").is_none() {
        return;
    }
    let mut auto = Autostart::new(RunKeys::current_user(), EXE);
    assert_eq!(auto.set(true).unwrap(), Status::On);
    assert_eq!(auto.registry().run_value(), Some(run_value(EXE)));
    assert_eq!(auto.set(false).unwrap(), Status::Off);
}
