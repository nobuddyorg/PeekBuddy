//! Zero undetected loss, end to end (#50, PR tier): the real engine watches a
//! tree, the churn generator (#25) works on it, and the comparator checks that
//! every operation is either a logged row or covered by a gap followed by a
//! finished reconcile. Runs in the `fs-serial` test group on real NTFS.
#![cfg(windows)]
#![expect(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns may unwrap"
)]

use std::time::Duration;

use peekbuddy_churn::exec::windows::{WinClock, WinTarget};
use peekbuddy_churn::exec::{Config, run};
use peekbuddy_churn::plan::Shape;
use peekbuddy_churn::profile::profile;
use peekbuddy_testkit::TestDir;
use peekbuddy_win::suspend::suspend_process;
#[path = "support/engine.rs"]
mod engine;
#[path = "support/judge.rs"]
mod judge;

use engine::{quit, spawn, wait_for_window, wait_until_settled};
use judge::{Run, judge};

/// Starts the engine on a tree, runs the churn, lets the log settle and stops
/// the engine; returns what the comparator needs.
fn churn_under_the_engine(
    dir: &TestDir,
    profile_name: &str,
    seed: u64,
    threads: u32,
    buffer_kib: Option<u32>,
    freeze: bool,
) -> (Run, peekbuddy_churn::exec::Outcome) {
    let data = dir.path().join("data");
    let root = dir.path().join("tree");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&root).unwrap();
    let root = std::fs::canonicalize(&root).unwrap();
    let plain = root
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .to_owned();
    let buffer = buffer_kib.map_or_else(String::new, |kib| format!("buffer_kib = {kib}\n"));
    std::fs::write(
        data.join("settings.toml"),
        format!(
            "version = 1\n[general]\nfirst_run = false\n[[watch]]\nid = \"w0\"\npath = '{plain}'\n{buffer}"
        ),
    )
    .unwrap();

    let mut engine = spawn(&data);
    let window = wait_for_window(&data, &mut engine.0);
    // Let the watch arm and the baseline reconcile finish before the churn.
    wait_until_settled(&data, Duration::from_secs(3));

    let config = Config {
        profile: profile(profile_name).unwrap(),
        seed,
        threads,
        shape: Shape::default(),
    };
    // A frozen engine reads nothing while the churn runs, so the 64 KiB buffer
    // overflows however fast the machine is.
    let frozen = freeze.then(|| suspend_process(engine.0.id()).unwrap());
    let outcome = run(&config, |_| (WinTarget::new(&root), WinClock::new()));
    drop(frozen);
    wait_until_settled(&data, Duration::from_secs(5));
    quit(window, &mut engine.0);
    (Run { data, root }, outcome)
}

#[test]
fn every_churn_operation_is_logged_or_covered_by_a_gap() {
    let dir = TestDir::new("zero-loss").unwrap();
    let (run, outcome) = churn_under_the_engine(&dir, "edge-mix", 7, 2, None, false);
    assert_eq!(outcome.failures(), 0, "the churn itself failed");
    let verdict = judge(&run, &outcome);
    assert_eq!(verdict.report.malformed, Vec::<u64>::new());
    assert_eq!(
        verdict.report.undetected,
        Vec::<u64>::new(),
        "undetected loss: {}",
        verdict.summary
    );
    assert_eq!(verdict.unfinished_gaps, 0, "{}", verdict.summary);
    assert!(
        verdict.report.matched + verdict.report.covered_by_gap > 0,
        "{}",
        verdict.summary
    );
}

/// A 64 KiB buffer and a burst of 50,000 operations: the watch overflows. Each
/// overflow must be a gap row followed by a finished reconcile, and nothing may
/// be lost outside a gap.
#[test]
fn a_forced_overflow_is_a_gap_followed_by_a_reconcile() {
    let dir = TestDir::new("zero-loss-overflow").unwrap();
    let (run, outcome) = churn_under_the_engine(&dir, "burst-50k", 11, 4, Some(64), true);
    assert_eq!(outcome.failures(), 0, "the churn itself failed");
    let verdict = judge(&run, &outcome);
    assert_eq!(verdict.report.malformed, Vec::<u64>::new());
    assert!(
        verdict.gaps >= 1,
        "the burst did not overflow the buffer: {}",
        verdict.summary
    );
    assert_eq!(verdict.unfinished_gaps, 0, "{}", verdict.summary);
    assert_eq!(
        verdict.report.undetected,
        Vec::<u64>::new(),
        "undetected loss: {}",
        verdict.summary
    );
}
