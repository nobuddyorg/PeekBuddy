//! Kill soak, PR smoke variant (#57): the real engine is killed with
//! `TerminateProcess` at seeded random points while the churn generator works
//! on its tree, and restarted on the same data directory. After every restart
//! the store must be sound; at the end the comparator must find no undetected
//! loss. Runs in the `fs-serial` group on real NTFS.
#![cfg(windows)]
#![expect(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns may unwrap"
)]

mod support;

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use peekbuddy_churn::exec::windows::{WinClock, WinTarget};
use peekbuddy_churn::exec::{Config, run};
use peekbuddy_churn::plan::Shape;
use peekbuddy_churn::profile::profile;
use peekbuddy_testkit::TestDir;
use rusqlite::{Connection, OpenFlags};
use support::{Run, all_rows, judge, quit, spawn, wait_for_window, wait_until_settled};

/// Kills in the smoke variant; the nightly job runs many more.
const KILLS: usize = 5;

/// A seeded generator for the kill points (xorshift64).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// A value in `low..high`.
    fn between(&mut self, low: u64, high: u64) -> u64 {
        low + self.next() % (high - low)
    }
}

fn seed() -> u64 {
    std::env::var("PEEKBUDDY_KILL_SEED")
        .ok()
        .and_then(|text| text.parse().ok())
        .unwrap_or(0x9E37_79B9_7F4A_7C15)
}

/// What a row looked like when it was committed.
fn snapshot(data: &Path) -> BTreeMap<i64, (u64, String, String)> {
    all_rows(data)
        .into_iter()
        .map(|row| {
            (
                row.id,
                (
                    row.ts,
                    row.kind.name().to_owned(),
                    String::from_utf16_lossy(&row.path),
                ),
            )
        })
        .collect()
}

/// Every segment and the catalog pass `integrity_check`, and no capture
/// cursor is ahead of the newest committed event.
fn assert_sound(data: &Path, context: &str) {
    let dir = data.join("segments");
    let open =
        |path: &Path| Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let check = |path: &Path| {
        let result: String = open(path)
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .unwrap();
        assert_eq!(result, "ok", "{context}: {}", path.display());
    };
    check(&dir.join("catalog.db"));
    let mut newest_event = 0_i64;
    let mut newest_cursor = 0_i64;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if !name.starts_with("events-")
            || path
                .extension()
                .is_none_or(|ext| !ext.eq_ignore_ascii_case("db"))
        {
            continue;
        }
        check(&path);
        let conn = open(&path);
        newest_event = newest_event.max(
            conn.query_row("SELECT coalesce(max(id), 0) FROM event", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        );
        newest_cursor = newest_cursor.max(
            conn.query_row(
                "SELECT coalesce(max(last_event_id), 0) FROM cursor",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        );
    }
    assert!(
        newest_cursor <= newest_event,
        "{context}: a cursor at {newest_cursor} is ahead of the newest event {newest_event}"
    );
}

#[test]
fn killing_the_engine_at_random_points_loses_nothing_it_does_not_cover() {
    let dir = TestDir::new("kill-soak").unwrap();
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

    let mut engine = spawn(&data);
    let mut window = wait_for_window(&data, &mut engine.0);
    // Let the watch arm and the baseline reconcile finish before the churn.
    wait_until_settled(&data, Duration::from_secs(3));

    let churn_root = root.clone();
    let churn = std::thread::spawn(move || {
        let config = Config {
            profile: profile("edge-mix").unwrap(),
            seed: 21,
            threads: 2,
            shape: Shape::default(),
        };
        run(&config, |_| (WinTarget::new(&churn_root), WinClock::new()))
    });

    let mut rng = Rng(seed());
    for kill in 0..KILLS {
        std::thread::sleep(Duration::from_millis(rng.between(300, 1_800)));
        let committed = snapshot(&data);
        engine.0.kill().unwrap();
        engine.0.wait().unwrap();

        engine = spawn(&data);
        window = wait_for_window(&data, &mut engine.0);
        let context = format!("after kill {kill} (seed {})", seed());
        assert_sound(&data, &context);
        let after = snapshot(&data);
        for (id, row) in &committed {
            assert_eq!(
                after.get(id),
                Some(row),
                "{context}: row {id} changed or vanished"
            );
        }
    }

    let outcome = churn.join().unwrap();
    assert_eq!(outcome.failures(), 0, "the churn itself failed");
    wait_until_settled(&data, Duration::from_secs(5));
    quit(window, &mut engine.0);
    assert_sound(&data, "after the final stop");

    let run = Run { data, root };
    let verdict = judge(&run, &outcome);
    assert_eq!(verdict.report.malformed, Vec::<u64>::new());
    assert_eq!(
        verdict.report.undetected,
        Vec::<u64>::new(),
        "undetected loss after {KILLS} kills (seed {}): {}",
        seed(),
        verdict.summary
    );
    assert_eq!(verdict.unfinished_gaps, 0, "{}", verdict.summary);
    // Every restart after a kill leaves a visible gap row (F2).
    assert!(verdict.gaps >= KILLS, "{}", verdict.summary);
}
