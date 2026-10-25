//! `--perf-probe` and `--diagnostics` of the real exe (#24, #161).
#![cfg(windows)]
#![expect(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns may unwrap"
)]

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
    assert!(started.elapsed() < Duration::from_secs(10));
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
    assert_eq!(milestone(&document, "EngineReady")["count"], 1);

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
