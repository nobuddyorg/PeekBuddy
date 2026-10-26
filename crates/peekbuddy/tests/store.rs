//! The event store inside the real engine (#52): events pushed the way
//! capture will push them reach the segment, the counters and the probe. The
//! synthetic-event flag only exists in builds with the `test-panic` feature,
//! so these tests run in the all-features jobs.
#![cfg(all(windows, feature = "test-panic"))]
#![expect(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns may unwrap"
)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use peekbuddy_testkit::TestDir;
use rusqlite::Connection;

const EXE: &str = env!("CARGO_BIN_EXE_peekbuddy");

fn segment(data: &Path) -> PathBuf {
    std::fs::read_dir(data.join("segments"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|ext| ext == "db"))
        .unwrap_or_else(|| PathBuf::from("no-segment-was-created"))
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |row| row.get(0)).unwrap()
}

fn run_with_events(dir: &TestDir, events: u32) -> serde_json::Value {
    let data = dir.path().join("data");
    let watched = dir.path().join("watched");
    std::fs::create_dir_all(&watched).unwrap();
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(
        data.join("settings.toml"),
        format!(
            "version = 1\n[general]\nfirst_run = false\n[[watch]]\nid = \"w\"\npath = '{}'\n",
            watched.display()
        ),
    )
    .unwrap();
    let out = dir.path().join("probe.json");
    let status = Command::new(EXE)
        .args(["--perf-probe", "0", "--synthetic-events"])
        .arg(events.to_string())
        .arg("--perf-probe-out")
        .arg(&out)
        .arg("--data-dir")
        .arg(&data)
        .stdin(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "{status:?}");
    serde_json::from_str(&std::fs::read_to_string(out).unwrap()).unwrap()
}

#[test]
fn synthetic_events_reach_the_segment_in_order_and_the_counters() {
    let dir = TestDir::new("store-synthetic").unwrap();
    let report = run_with_events(&dir, 500);
    let data = dir.path().join("data");

    let conn = Connection::open(segment(&data)).unwrap();
    assert_eq!(count(&conn, "SELECT count(*) FROM event"), 500);
    assert_eq!(count(&conn, "SELECT min(id) FROM event"), 1);
    assert_eq!(count(&conn, "SELECT max(id) FROM event"), 500);
    assert_eq!(
        count(
            &conn,
            "SELECT count(*) FROM event WHERE id > 1 AND ts <= (SELECT ts FROM event e2 WHERE e2.id = event.id - 1)"
        ),
        0,
        "rows are not in the pushed order"
    );
    assert_eq!(count(&conn, "SELECT count(*) FROM root"), 1);
    assert_eq!(
        count(&conn, "SELECT count(*) FROM node WHERE name LIKE 'file%'"),
        500
    );
    assert_eq!(
        count(
            &conn,
            "SELECT count(*) FROM node_fts WHERE node_fts MATCH '\"file42.txt\"'"
        ),
        1
    );

    // The probe saw the commits and the counters moved with them.
    let counters = &report["counters"];
    assert_eq!(counters["rowsPersisted"], 500);
    assert_eq!(counters["eventsReceived"], 500);
    let commits = report["batchCommits"].as_array().unwrap();
    assert_ne!(commits.len(), 0);
    assert_eq!(
        commits
            .iter()
            .map(|c| c["rows"].as_u64().unwrap())
            .sum::<u64>(),
        500
    );
    assert!(
        report["milestones"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["name"] == "StoreOpened")
    );
}

#[test]
fn quitting_commits_the_queue_and_truncates_the_log() {
    let dir = TestDir::new("store-quit").unwrap();
    run_with_events(&dir, 3_000);
    let data = dir.path().join("data");
    let db = segment(&data);
    let conn = Connection::open(&db).unwrap();
    assert_eq!(count(&conn, "SELECT count(*) FROM event"), 3_000);
    // The final checkpoint ran: the write-ahead log is gone or empty.
    let mut wal = db.into_os_string();
    wal.push("-wal");
    let wal = PathBuf::from(wal);
    assert!(
        !wal.exists() || std::fs::metadata(&wal).unwrap().len() == 0,
        "{wal:?}"
    );
    // A second run continues the id sequence in the same segment.
    drop(conn);
    run_with_events(&dir, 10);
    let conn = Connection::open(segment(&data)).unwrap();
    assert_eq!(count(&conn, "SELECT count(*) FROM event"), 3_010);
    assert_eq!(count(&conn, "SELECT max(id) FROM event"), 3_010);
}
