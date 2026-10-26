use super::*;

const EXE: &str = r"C:\Program Files\PeekBuddy\peekbuddy.exe";
const OLD: &str = r#""D:\old\peekbuddy.exe""#;

fn disabled_by_task_manager() -> Vec<u8> {
    vec![3, 0, 0, 0, 0xAA, 0xBB, 0xCC, 0xDD, 0, 0, 0, 0]
}

fn with(run: Option<&str>, approved: Option<Vec<u8>>) -> Autostart<MemoryRegistry> {
    Autostart::new(
        MemoryRegistry {
            run: run.map(str::to_owned),
            approved,
            writes: 0,
        },
        EXE,
    )
}

#[test]
fn the_value_is_the_quoted_path() {
    assert_eq!(run_value(EXE), format!("\"{EXE}\""));
}

#[test]
fn the_flag_byte_decides() {
    assert_eq!(Approval::parse(None), Approval::Absent);
    assert_eq!(Approval::parse(Some(&[])), Approval::Absent);
    assert_eq!(Approval::parse(Some(&[2, 0, 0])), Approval::Enabled);
    assert_eq!(Approval::parse(Some(&[6])), Approval::Enabled);
    assert_eq!(Approval::parse(Some(&[3])), Approval::Disabled);
    assert_eq!(Approval::parse(Some(&[7, 1, 2])), Approval::Disabled);
    assert_eq!(Approval::parse(Some(&APPROVED_ENABLED)), Approval::Enabled);
}

#[test]
fn off_by_default_and_a_start_with_the_intent_off_touches_nothing() {
    let mut a = with(None, None);
    assert_eq!(a.status(), Status::Off);
    assert_eq!(a.on_start(false), Ok(Status::Off));
    assert_eq!(a.registry().writes, 0);
    // A value from an earlier run stays: only a toggle removes it.
    let mut b = with(Some(OLD), None);
    assert!(matches!(b.on_start(false), Ok(Status::OtherPath { .. })));
    assert_eq!(b.registry().writes, 0);
}

#[test]
fn a_start_with_the_intent_on_writes_a_missing_or_stale_value() {
    let mut missing = with(None, None);
    assert_eq!(missing.on_start(true), Ok(Status::On));
    assert_eq!(
        missing.registry().run.as_deref(),
        Some(run_value(EXE).as_str())
    );

    let mut moved = with(Some(OLD), None);
    assert_eq!(moved.on_start(true), Ok(Status::On));
    assert_eq!(
        moved.registry().run.as_deref(),
        Some(run_value(EXE).as_str())
    );
    assert_eq!(moved.registry().writes, 1);
}

#[test]
fn a_start_never_re_enables_what_task_manager_disabled() {
    let value = run_value(EXE);
    let mut a = with(Some(&value), Some(disabled_by_task_manager()));
    assert_eq!(a.on_start(true), Ok(Status::DisabledInTaskManager));
    assert_eq!(a.registry().approved, Some(disabled_by_task_manager()));
    assert_eq!(a.registry().writes, 0);

    // Also when the value is stale and gets rewritten: the switch stays.
    let mut b = with(Some(OLD), Some(disabled_by_task_manager()));
    assert_eq!(b.on_start(true), Ok(Status::DisabledInTaskManager));
    assert_eq!(b.registry().approved, Some(disabled_by_task_manager()));
}

#[test]
fn an_explicit_toggle_on_enables_again_and_off_removes_the_value() {
    let value = run_value(EXE);
    let mut a = with(Some(&value), Some(disabled_by_task_manager()));
    assert_eq!(a.set(true), Ok(Status::On));
    assert_eq!(a.registry().approved, Some(APPROVED_ENABLED.to_vec()));

    assert_eq!(a.set(false), Ok(Status::Off));
    assert_eq!(a.registry().run, None);
    assert_eq!(a.set(false), Ok(Status::Off), "twice is fine");
}

#[test]
fn an_explicit_toggle_on_leaves_an_untouched_approval_alone() {
    let mut a = with(None, None);
    assert_eq!(a.set(true), Ok(Status::On));
    assert_eq!(a.registry().approved, None);
}
