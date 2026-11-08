//! The reports of `scripts/owner/perf-baremetal.ps1` that are committed
//! under `perf/owner-runs/` validate against `perf/owner-report.schema.json`
//! (#84), and the schema refuses a report that lost its preflight.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns may unwrap"
)]

use std::path::Path;

fn schema() -> jsonschema::Validator {
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../../../perf/owner-report.schema.json")).unwrap();
    jsonschema::validator_for(&schema).unwrap()
}

#[test]
fn every_committed_owner_report_is_valid() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../perf/owner-runs");
    let validator = schema();
    let mut seen = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        // PowerShell writes a byte order mark.
        let document: serde_json::Value =
            serde_json::from_str(text.trim_start_matches('\u{feff}')).unwrap();
        let errors: Vec<_> = validator
            .iter_errors(&document)
            .map(|error| error.to_string())
            .collect();
        assert!(errors.is_empty(), "{}: {errors:#?}", path.display());
        seen += 1;
    }
    assert!(seen > 0, "no report under perf/owner-runs");
}

#[test]
fn a_report_without_a_preflight_is_refused() {
    let document = serde_json::json!({
        "script": "perf-baremetal.ps1",
        "schemaVersion": 1,
        "startedUtc": "2026-11-08T12:00:00Z",
        "exeVersion": "0.1.0",
        "exeSha256": "0".repeat(64),
        "runs": 3,
        "harness": { "exitCode": 0, "scenarios": {}, "log": "" },
        "viewer": {
            "runs": [],
            "firstFrameMedianUs": null,
            "firstFrameMinUs": null,
            "firstFrameMaxUs": null
        },
        "notMeasured": [],
        "durationSec": 1.0
    });
    assert!(!schema().is_valid(&document));
}
