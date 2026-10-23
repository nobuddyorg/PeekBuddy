//! The executor against the real file system (Windows only, #25).
#![cfg(windows)]

use peekbuddy_churn::exec::windows::{WinClock, WinTarget, defender_realtime, header, os_build};
use peekbuddy_churn::exec::{Clock, Config, run};
use peekbuddy_churn::ledger::{Record, from_jsonl, to_jsonl};
use peekbuddy_churn::ops::OpKind;
use peekbuddy_churn::plan::Shape;
use peekbuddy_churn::profile::{Names, Profile, Rate, profile};
use peekbuddy_testkit::TestDir;

/// The real clock with a cap on total sleep: a pacing bug fails the test
/// instead of hanging it.
struct Bounded {
    clock: WinClock,
    slept: u64,
}

impl Bounded {
    fn new() -> Self {
        Self {
            clock: WinClock::new(),
            slept: 0,
        }
    }
}

impl Clock for Bounded {
    fn now(&mut self) -> u64 {
        self.clock.now()
    }

    fn sleep(&mut self, ticks: u64) {
        self.slept = self.slept.saturating_add(ticks);
        // 20 s in total is far more than any test here needs.
        assert!(
            self.slept <= 200_000_000,
            "pacing asked for {} ticks",
            self.slept
        );
        self.clock.sleep(ticks);
    }
}

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
    let outcome = run(&config, |_| (WinTarget::new(&root), Bounded::new()));
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
    let parts: Vec<&str> = header.os_build.split('.').collect();
    assert_eq!(parts.len(), 4, "{}", header.os_build);
    assert!(
        parts.iter().all(|p| p.parse::<u32>().is_ok()),
        "{}",
        header.os_build
    );
    assert_eq!(parts[..2], ["10", "0"]);
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
fn machine_facts_match_independent_queries() {
    // The build number from the registry must be the third part.
    let output = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "(Get-ItemProperty 'HKLM:\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion').CurrentBuildNumber",
        ])
        .output()
        .unwrap();
    let build = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let os = os_build().unwrap();
    assert_eq!(os.split('.').nth(2), Some(build.as_str()), "{os}");
    // Defender: the same property read through JSON.
    let output = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Get-MpComputerStatus | Select-Object -ExpandProperty RealTimeProtectionEnabled | ConvertTo-Json",
        ])
        .output()
        .unwrap();
    let json = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let expected = match json.as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    };
    assert_eq!(defender_realtime(), expected, "{json}");
}

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
    let outcome = run(&config, |_| (WinTarget::new(&root), Bounded::new()));
    assert_eq!(outcome.failures(), 0);
    // 2,000 events at 2,000 ev/s cannot finish much before 1 s; the 10 ms
    // bucket and the 1 ms minimum sleep allow a little slack.
    assert!(outcome.elapsed >= 9_500_000, "{} ticks", outcome.elapsed);
    dir.close().unwrap();
}
