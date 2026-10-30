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
        .filter(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("events-"))
        })
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
    // The watch of the settings is live, so its lifecycle rows (state and
    // baseline, source System and Reconcile) are in the segment too; the
    // synthetic events are the Rdcw rows.
    assert_eq!(
        count(&conn, "SELECT count(*) FROM event WHERE source = 0"),
        500
    );
    let total = count(&conn, "SELECT count(*) FROM event");
    assert!(total >= 500, "{total}");
    assert_eq!(count(&conn, "SELECT min(id) FROM event"), 1);
    assert_eq!(count(&conn, "SELECT max(id) FROM event"), total);
    assert_eq!(
        count(
            &conn,
            "SELECT count(*) FROM event WHERE source = 0 AND id > 1 AND ts <= (SELECT max(ts) FROM event e2 WHERE e2.source = 0 AND e2.id < event.id)"
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
    assert_eq!(counters["rowsPersisted"], total);
    assert_eq!(counters["eventsReceived"], 500);
    let commits = report["batchCommits"].as_array().unwrap();
    assert_ne!(commits.len(), 0);
    assert_eq!(
        commits
            .iter()
            .map(|c| c["rows"].as_i64().unwrap())
            .sum::<i64>(),
        total
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
    assert_eq!(
        count(&conn, "SELECT count(*) FROM event WHERE source = 0"),
        3_000
    );
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
    assert_eq!(
        count(&conn, "SELECT count(*) FROM event WHERE source = 0"),
        3_010
    );
    assert_eq!(
        count(&conn, "SELECT max(id) FROM event"),
        count(&conn, "SELECT count(*) FROM event"),
        "ids stay contiguous"
    );
}

/// `WM_ENDSESSION` with 10,000 events pending (#48, #171): the engine commits
/// them and checkpoints inside 2 s, well within the 5 s Windows allows
/// (`shutdown.endsession_flush`).
#[test]
fn a_session_end_flushes_ten_thousand_pending_events_in_time() {
    use std::time::{Duration, Instant};

    use peekbuddy_engine::Names;
    use peekbuddy_win::window::WindowHandle;

    let dir = TestDir::new("store-endsession").unwrap();
    let data = dir.path().join("data");
    let watched = dir.path().join("watched");
    std::fs::create_dir_all(&watched).unwrap();
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(
        data.join("settings.toml"),
        format!(
            "version = 1
[general]
first_run = false
[[watch]]
id = \"w\"
path = '{}'
",
            watched.display()
        ),
    )
    .unwrap();
    let mut engine = Command::new(EXE)
        .args(["--headless", "--synthetic-events", "10000", "--data-dir"])
        .arg(&data)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let class = Names::new(Some(&data)).window_class;
    let deadline = Instant::now() + Duration::from_secs(20);
    let window = loop {
        if let Some(window) = WindowHandle::find(&class) {
            break window;
        }
        assert!(Instant::now() < deadline, "no engine window");
        std::thread::sleep(Duration::from_millis(20));
    };
    let started = Instant::now();
    assert!(window.query_end_session(2_000).unwrap());
    window.end_session(5_000).unwrap();
    let flushed = started.elapsed();
    assert!(flushed < Duration::from_secs(2), "{flushed:?}");
    window.post_close().ok();
    let _ = engine.wait();

    let conn = Connection::open(segment(&data)).unwrap();
    assert_eq!(
        count(&conn, "SELECT count(*) FROM event WHERE source = 0"),
        10_000
    );
}

/// A crash marker at start gives exactly one "terminated abnormally" row,
/// without any watch configured (#171); a normal start after it adds none.
#[test]
fn a_crash_marker_gives_exactly_one_abnormal_exit_row() {
    let dir = TestDir::new("store-abnormal").unwrap();
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(
        data.join(peekbuddy_engine::crash::MARKER_FILE),
        "crash-20261026T120000Z.log",
    )
    .unwrap();
    for _ in 0..2 {
        let status = Command::new(EXE)
            .args(["--perf-probe", "0", "--data-dir"])
            .arg(&data)
            .stdin(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "{status:?}");
    }
    assert!(!data.join(peekbuddy_engine::crash::MARKER_FILE).exists());
    let conn = Connection::open(segment(&data)).unwrap();
    // Kind 25 is stored as tag << 8.
    assert_eq!(
        count(&conn, "SELECT count(*) FROM event WHERE kind = 6400"),
        1
    );
    let name: String = conn
        .query_row(
            "SELECT node.name FROM event JOIN node ON node.id = event.node_id WHERE event.kind = 6400",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(name, "crash-20261026T120000Z.log");
}

/// The first event of today closes an older segment: the id sequence goes on
/// in a new file that starts with the rotation row, and the old one is
/// sealed in the catalog (#54).
#[test]
fn the_first_event_of_today_rotates_an_older_segment() {
    use peekbuddy_core::event::{Event, Flags, Kind, NameKind, NameRef, RootIdx, Source, VolIdx};
    use peekbuddy_store::batch::Batch;
    use peekbuddy_store::catalog::Catalog;
    use peekbuddy_store::writer::Writer;

    let dir = TestDir::new("store-rotate").unwrap();
    let data = dir.path().join("data");
    let segments = data.join("segments");
    std::fs::create_dir_all(&segments).unwrap();
    // A segment from long ago, with two rows.
    let old = segments.join("events-2020-01-01.db");
    {
        let mut writer = Writer::open(&old).unwrap();
        let mut batch = Batch::with_capacity(2, 256);
        for ts in [132_000_000_000_000_000_u64, 132_000_000_000_000_001] {
            let event = Event {
                ts,
                qpc: 0,
                src_ts: 0,
                source: Source::System,
                kind: Kind::EngineStarted,
                raw_mask: 0,
                source_info: 0,
                is_dir: false,
                size: 0,
                attrs: 0,
                reparse_tag: 0,
                file_id: 0,
                parent_file_id: 0,
                usn: 0,
                vol: VolIdx(0),
                root: RootIdx::NONE,
                name: NameRef::EMPTY,
                name_kind: NameKind::Unknown,
                flags: Flags::NONE,
            };
            batch.push(event, &[]).unwrap();
        }
        writer.write(&[batch], &[]).unwrap();
        writer.checkpoint_truncate().unwrap();
    }
    let _ = run_with_events(&dir, 20);

    // The synthetic events are stamped in 2022 and the engine's own rows with
    // today's time, so the day changes more than once: every file after the
    // first starts with a rotation row and the ids never skip.
    let catalog = Catalog::open(&segments.join("catalog.db")).unwrap();
    let rows = catalog.list().unwrap();
    assert!(rows.len() >= 2, "{rows:?}");
    assert_eq!(
        (rows[0].file.as_str(), rows[0].first_id, rows[0].last_id),
        ("events-2020-01-01.db", 1, 2)
    );
    assert!(rows[0].sealed, "the old segment was sealed");
    let mut next = 1;
    for (index, row) in rows.iter().enumerate() {
        let conn = Connection::open(segments.join(&row.file)).unwrap();
        assert_eq!(
            count(&conn, "SELECT min(id) FROM event"),
            next,
            "{}",
            row.file
        );
        if index > 0 {
            assert_eq!(
                count(&conn, &format!("SELECT kind FROM event WHERE id = {next}")),
                4352,
                "{} does not start with a rotation row",
                row.file
            );
        }
        next = count(&conn, "SELECT max(id) FROM event") + 1;
    }
    let events: i64 = rows
        .iter()
        .map(|row| {
            let conn = Connection::open(segments.join(&row.file)).unwrap();
            count(&conn, "SELECT count(*) FROM event WHERE source = 0")
        })
        .sum();
    assert_eq!(events, 20);
}
