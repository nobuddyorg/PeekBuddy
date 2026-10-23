//! The executor against the real file system (Windows only, #25).
#![cfg(windows)]

use peekbuddy_churn::exec::windows::{WinClock, WinTarget, header};
use peekbuddy_churn::exec::{Config, run};
use peekbuddy_churn::ledger::{Record, from_jsonl, to_jsonl};
use peekbuddy_churn::ops::OpKind;
use peekbuddy_churn::plan::Shape;
use peekbuddy_churn::profile::{Names, Profile, Rate, profile};
use peekbuddy_testkit::TestDir;

#[test]
fn edge_mix_runs_every_class_without_failures() {
    let dir = TestDir::new("churn-edge").unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let config = Config {
        profile: profile("edge-mix").unwrap(),
        seed: 1,
        threads: 2,
        shape: Shape::default(),
    };
    let outcome = run(&config, |_| (WinTarget::new(&root), WinClock::new()));
    let failed: Vec<&Record> = outcome
        .records
        .iter()
        .filter(|record| record.result != 0)
        .take(5)
        .collect();
    assert_eq!(failed, Vec::<&Record>::new());
    assert_eq!(outcome.missing_classes(config.profile), []);
    for kind in OpKind::ALL {
        assert!(
            outcome.classes.get(&kind).is_some_and(|c| c.ok > 0),
            "{kind:?}"
        );
    }
    // The ledger round-trips, header facts included.
    let header = header("edge-mix", 1, 2, &root, "test");
    assert_ne!(header.os_build, "unknown");
    assert!(header.cpus > 0);
    assert!(["NTFS", "ReFS"].contains(&header.file_system.as_str()));
    assert!(header.short_names.is_some());
    let text = to_jsonl(&header, &outcome.records).unwrap();
    assert_eq!(from_jsonl(&text).unwrap().1, outcome.records);
    dir.close().unwrap();
}

const PACED: Profile = Profile {
    name: "paced-test",
    mix: &[(OpKind::CreateFile, 1), (OpKind::Write, 1)],
    rate: Rate::Steady {
        events_per_s: 2_000,
        seconds: 1,
    },
    names: Names::Ascii,
};

#[test]
fn pacing_holds_a_steady_run_to_its_rate() {
    let dir = TestDir::new("churn-paced").unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let config = Config {
        profile: &PACED,
        seed: 3,
        threads: 1,
        shape: Shape::default(),
    };
    let outcome = run(&config, |_| (WinTarget::new(&root), WinClock::new()));
    assert_eq!(outcome.failures(), 0);
    // 2,000 events at 2,000 ev/s cannot finish much before 1 s; the 10 ms
    // bucket and the 1 ms minimum sleep allow a little slack.
    assert!(outcome.elapsed >= 9_500_000, "{} ticks", outcome.elapsed);
    dir.close().unwrap();
}
