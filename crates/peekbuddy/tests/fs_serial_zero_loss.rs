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
#[path = "support/spill.rs"]
mod spill;
#[path = "support/tree.rs"]
mod tree;

use engine::{quit, spawn, wait_for_window, wait_until_settled};
use judge::{Run, judge};
use spill::drain_spill;
use tree::tree_diff;

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
    drain_spill(&data);
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
    // After the gap and its reconcile the log, replayed, is the tree on disk.
    assert_eq!(
        tree_diff(&run, 20),
        Vec::<String>::new(),
        "the replayed log differs from the disk: {}",
        verdict.summary
    );
}

// ---------------------------------------------------------------------------
// The nightly tier (#277, #17): the same checks at a scale the PR tier cannot
// afford. `#[ignore]`d; the nightly job runs them with `--run-ignored only`
// and `PEEKBUDDY_ZERO_LOSS_REPORT` naming the file for their JSON results.
// ---------------------------------------------------------------------------

/// Appends one JSON line with the numbers of a nightly run to the file
/// `PEEKBUDDY_ZERO_LOSS_REPORT` names (nothing when it is not set).
fn report(name: &str, ops: usize, verdict: &judge::Verdict, tree_differences: usize) {
    use std::io::Write as _;

    let Some(path) = std::env::var_os("PEEKBUDDY_ZERO_LOSS_REPORT") else {
        return;
    };
    let line = format!(
        "{{\"name\":\"{name}\",\"ops\":{ops},\"live\":{},\"reconciled\":{},\"gapRows\":{},\"undetected\":{},\"treeDifferences\":{tree_differences}}}",
        verdict.report.matched,
        verdict.report.covered_by_gap,
        verdict.gaps,
        verdict.report.undetected.len(),
    );
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    writeln!(file, "{line}").unwrap();
}

/// Asserts the things every nightly run owes and reports the numbers.
fn nightly_verdict(name: &str, run: &Run, outcome: &peekbuddy_churn::exec::Outcome) {
    verdict_with(name, run, outcome, false);
}

/// `kill`: a run that lost the engine. The snapshot of the reconcile is kept
/// by the walks only (docs/explanation/reconcile.md), so a path the live
/// stream logged and the gap then deleted is not reported as deleted: the log
/// may hold such paths, but must hold every path that is on the disk.
fn verdict_with(name: &str, run: &Run, outcome: &peekbuddy_churn::exec::Outcome, kill: bool) {
    let verdict = judge(run, outcome);
    let mut diff = tree_diff(run, if kill { usize::MAX } else { 20 });
    if kill {
        diff.retain(|line| line.starts_with("+ "));
        diff.truncate(20);
    }
    report(name, outcome.records.len(), &verdict, diff.len());
    assert_eq!(verdict.report.malformed, Vec::<u64>::new(), "{name}");
    assert_eq!(
        verdict.report.undetected,
        Vec::<u64>::new(),
        "{name}: undetected loss: {}",
        verdict.summary
    );
    assert_eq!(verdict.unfinished_gaps, 0, "{name}: {}", verdict.summary);
    assert_eq!(
        diff,
        Vec::<String>::new(),
        "{name}: the replayed log differs from the disk: {}",
        verdict.summary
    );
}

/// A million raw events from eight writers, as a burst: the buffer overflows,
/// the reconcile closes the gaps, nothing is lost outside them.
#[test]
#[ignore = "nightly zero-loss tier (#277)"]
fn a_million_events_from_eight_writers_lose_nothing_undetected() {
    let dir = TestDir::new("zero-loss-million").unwrap();
    // The hosted arm runner needs more than 75 minutes for the full burst:
    // its job asks for the half-size profile.
    let profile =
        std::env::var("PEEKBUDDY_ZERO_LOSS_PROFILE").unwrap_or_else(|_| "burst-100k".to_owned());
    let (run, outcome) = churn_under_the_engine(&dir, &profile, 31, 8, None, false);
    assert_eq!(outcome.failures(), 0, "the churn itself failed");
    nightly_verdict(&format!("burst-eight-writers-{profile}"), &run, &outcome);
}

/// The same kind of burst against a small buffer and a frozen engine: forced
/// overflows, five times over.
#[test]
#[ignore = "nightly zero-loss tier (#277)"]
fn a_forced_overflow_burst_is_closed_every_time() {
    for round in 0..5_u64 {
        let dir = TestDir::new(&format!("zero-loss-forced-{round}")).unwrap();
        let (run, outcome) =
            churn_under_the_engine(&dir, "burst-50k", 40 + round, 4, Some(64), true);
        assert_eq!(outcome.failures(), 0, "the churn itself failed");
        let verdict = judge(&run, &outcome);
        assert!(
            verdict.gaps >= 1,
            "round {round}: no overflow: {}",
            verdict.summary
        );
        nightly_verdict(&format!("forced-overflow-{round}"), &run, &outcome);
    }
}

/// A data directory with one watch on a new tree; returns (data, root).
fn watched_tree(dir: &TestDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let data = dir.path().join("data");
    let root = dir.path().join("tree");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&root).unwrap();
    let root = std::fs::canonicalize(&root).unwrap();
    let plain = root
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .to_owned();
    std::fs::write(
        data.join("settings.toml"),
        format!(
            "version = 1\n[general]\nfirst_run = false\n[[watch]]\nid = \"w0\"\npath = '{plain}'\n"
        ),
    )
    .unwrap();
    (data, root)
}

/// The engine is killed with `TerminateProcess` in the middle of a burst and
/// started again; the reconcile of the restart closes what the kill cost.
#[test]
#[ignore = "nightly zero-loss tier (#277)"]
fn a_kill_in_the_middle_of_a_burst_is_closed_by_the_restart() {
    let dir = TestDir::new("zero-loss-kill").unwrap();
    let (data, root) = watched_tree(&dir);
    let mut engine = spawn(&data);
    let _window = wait_for_window(&data, &mut engine.0);
    wait_until_settled(&data, Duration::from_secs(3));

    let churn_root = root.clone();
    let churn = std::thread::spawn(move || {
        let config = Config {
            profile: profile("burst-50k").unwrap(),
            seed: 77,
            threads: 4,
            shape: Shape::default(),
        };
        run(&config, |_| (WinTarget::new(&churn_root), WinClock::new()))
    });
    std::thread::sleep(Duration::from_millis(1_500));
    engine.0.kill().unwrap();
    engine.0.wait().unwrap();

    let mut engine = spawn(&data);
    let window = wait_for_window(&data, &mut engine.0);
    let outcome = churn.join().unwrap();
    assert_eq!(outcome.failures(), 0, "the churn itself failed");
    wait_until_settled(&data, Duration::from_secs(8));
    quit(window, &mut engine.0);
    drain_spill(&data);
    verdict_with("kill-in-a-burst", &Run { data, root }, &outcome, true);
}

/// The watched root is deleted and made again: the watch is lost, then
/// restored, and the replayed log is the tree on disk.
#[test]
#[ignore = "nightly zero-loss tier (#277)"]
fn a_watched_root_deleted_and_recreated_is_logged_again() {
    use peekbuddy_core::event::{Kind, WatchState};

    let dir = TestDir::new("zero-loss-root").unwrap();
    let (data, root) = watched_tree(&dir);
    std::fs::create_dir_all(root.join("old")).unwrap();
    std::fs::write(root.join("old").join("a.txt"), b"x").unwrap();
    let mut engine = spawn(&data);
    let window = wait_for_window(&data, &mut engine.0);
    wait_until_settled(&data, Duration::from_secs(3));

    std::fs::remove_dir_all(&root).unwrap();
    std::thread::sleep(Duration::from_secs(2));
    std::fs::create_dir_all(root.join("new")).unwrap();
    std::thread::sleep(Duration::from_secs(3));
    for index in 0..50 {
        std::fs::write(root.join("new").join(format!("f{index}.txt")), b"x").unwrap();
    }
    wait_until_settled(&data, Duration::from_secs(5));
    quit(window, &mut engine.0);

    let states: Vec<WatchState> = engine::all_rows(&data)
        .iter()
        .filter_map(|row| match row.kind {
            Kind::WatchStateChanged(state) => Some(state),
            _ => None,
        })
        .collect();
    assert!(states.contains(&WatchState::Lost), "{states:?}");
    assert!(states.contains(&WatchState::Restored), "{states:?}");
    let diff = tree_diff(&Run { data, root }, 20);
    assert_eq!(
        diff,
        Vec::<String>::new(),
        "the replayed log differs from the disk"
    );
}
