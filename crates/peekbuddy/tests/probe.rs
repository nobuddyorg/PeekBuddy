//! `--perf-probe` and `--diagnostics` of the real exe (#24, #161).
#![cfg(windows)]
#![expect(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns may unwrap"
)]

use std::fmt::Write as _;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use peekbuddy_testkit::TestDir;

const EXE: &str = env!("CARGO_BIN_EXE_peekbuddy");

fn schema() -> jsonschema::Validator {
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../../../perf/probe.schema.json")).unwrap();
    jsonschema::validator_for(&schema).unwrap()
}

fn assert_valid(text: &str) -> serde_json::Value {
    let document: serde_json::Value = serde_json::from_str(text).unwrap();
    let errors: Vec<_> = schema()
        .iter_errors(&document)
        .map(|error| error.to_string())
        .collect();
    assert!(errors.is_empty(), "{errors:#?}\n{text}");
    document
}

fn milestone<'a>(document: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    document["milestones"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == name)
        .unwrap()
}

#[test]
fn the_probe_prints_a_valid_document_to_a_redirected_stdout() {
    let dir = TestDir::new("probe-stdout").unwrap();
    let started = Instant::now();
    let output = Command::new(EXE)
        .args(["--perf-probe", "0", "--data-dir"])
        .arg(dir.path())
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output.status);
    // Generous: the first start of a fresh exe on a busy runner can take
    // seconds (virus scan, parallel tests); this only catches a hang.
    assert!(started.elapsed() < Duration::from_secs(30));
    let document = assert_valid(&String::from_utf8(output.stdout).unwrap());

    assert_eq!(document["requestedSeconds"], 0);
    let ready = milestone(&document, "EngineReady")["firstUs"]
        .as_u64()
        .unwrap();
    let start = milestone(&document, "ProcessStart")["firstUs"]
        .as_u64()
        .unwrap();
    assert!(
        start <= ready,
        "ProcessStart {start} after EngineReady {ready}"
    );
    assert!(ready > 0 && ready < 5_000_000, "ready after {ready} us");
    // Every milestone appears exactly once per occurrence: the engine marks
    // each of these once per run.
    for name in ["ProcessStart", "EngineReady"] {
        assert_eq!(milestone(&document, name)["count"], 1, "{name}");
    }
    let times: Vec<u64> = document["milestones"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["firstUs"].as_u64().unwrap())
        .collect();
    assert!(times.windows(2).all(|pair| pair[0] <= pair[1]), "{times:?}");

    let (first, last) = (&document["samples"]["start"], &document["samples"]["end"]);
    assert!(first["privateBytes"].as_u64().unwrap() > 0);
    assert!(first["workingSet"].as_u64().unwrap() > 0);
    assert!(first["handles"].as_u64().unwrap() > 0);
    assert!(last["atUs"].as_u64().unwrap() >= first["atUs"].as_u64().unwrap());
}

#[test]
fn the_probe_stays_up_and_writes_the_requested_file() {
    let dir = TestDir::new("probe-file").unwrap();
    let out = dir.path().join("probe.json");
    let started = Instant::now();
    let output = Command::new(EXE)
        .args(["--perf-probe", "1", "--perf-probe-out"])
        .arg(&out)
        .arg("--data-dir")
        .arg(dir.path().join("data"))
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output.status);
    assert!(started.elapsed() >= Duration::from_millis(900));
    assert!(output.stdout.is_empty(), "the file replaces stdout");
    let document = assert_valid(&std::fs::read_to_string(&out).unwrap());
    assert_eq!(document["requestedSeconds"], 1);
    let span = document["samples"]["end"]["atUs"].as_u64().unwrap()
        - document["samples"]["start"]["atUs"].as_u64().unwrap();
    assert!(
        (900_000..5_000_000).contains(&span),
        "sample span {span} us"
    );
}

#[test]
fn diagnostics_prints_to_a_redirected_stdout_without_user_paths() {
    let output = Command::new(EXE)
        .arg("--diagnostics")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output.status);
    let text = String::from_utf8(output.stdout).unwrap();
    for needle in [
        "PeekBuddy: ",
        "rustc: rustc ",
        "dependency windows-sys: 0.61.2",
        "Windows: 10.0.",
        "architecture: ",
        "data mode: ",
        "Defender real-time protection: ",
    ] {
        assert!(text.contains(needle), "{needle} missing in\n{text}");
    }
    let profile = std::env::var("USERPROFILE").unwrap();
    assert!(!text.contains(&profile), "user path in\n{text}");
}

#[test]
fn diagnostics_full_shows_the_data_directory() {
    let output = Command::new(EXE)
        .args(["--diagnostics", "--full"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        text.to_lowercase().contains(r"\nobuddy\peekbuddy"),
        "{text}"
    );
}

fn run_probe_with_watches(dir: &TestDir, count: usize) -> serde_json::Value {
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    let mut settings = String::from("version = 1\n[general]\nfirst_run = false\n");
    for index in 0..count {
        let folder = dir.path().join(format!("watched{index}"));
        std::fs::create_dir_all(&folder).unwrap();
        let _ = write!(
            settings,
            "[[watch]]\nid = \"w{index}\"\npath = '{}'\n",
            folder.display()
        );
    }
    std::fs::write(data.join("settings.toml"), settings).unwrap();
    let output = Command::new(EXE)
        .args(["--perf-probe", "1", "--data-dir"])
        .arg(&data)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output.status);
    assert_valid(&String::from_utf8(output.stdout).unwrap())
}

/// F7: ready is "every watch armed and the tray attempt done", so
/// `EngineReady` comes once, after `WatcherArmed` of every root (#162).
#[test]
fn engine_ready_waits_for_every_watch_to_arm_and_is_recorded_once() {
    let dir = TestDir::new("probe-ready").unwrap();
    let document = run_probe_with_watches(&dir, 3);
    let armed = milestone(&document, "WatcherArmed");
    assert_eq!(armed["count"], 3);
    let ready = milestone(&document, "EngineReady");
    assert_eq!(ready["count"], 1);
    assert!(
        ready["firstUs"].as_u64().unwrap() >= armed["lastUs"].as_u64().unwrap(),
        "ready before the last watch was armed: {document}"
    );
    let names: Vec<&str> = document["milestones"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["name"].as_str().unwrap())
        .collect();
    // The start reconcile of each new root is recorded too.
    assert!(names.contains(&"ReconcileStart"), "{names:?}");
}

#[test]
fn a_run_without_watches_is_ready_without_any_watcher_milestone() {
    let dir = TestDir::new("probe-ready-none").unwrap();
    let document = run_probe_with_watches(&dir, 0);
    assert_eq!(milestone(&document, "EngineReady")["count"], 1);
    assert!(
        document["milestones"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["name"] != "WatcherArmed")
    );
}

#[test]
fn the_engine_log_records_the_watch_states() {
    let dir = TestDir::new("probe-log").unwrap();
    let _ = run_probe_with_watches(&dir, 1);
    let log = std::fs::read_to_string(dir.path().join("data").join("logs").join("engine.log"))
        .unwrap_or_default();
    assert!(log.contains("watch 0: Arming -> Live"), "{log}");
}
