//! The churn sketches in the segment (#82): what the writer keeps of the
//! busiest paths and extensions of each hour.
#![expect(clippy::unwrap_used, reason = "tests unwrap")]

use std::path::{Path, PathBuf};

use peekbuddy_core::event::{Event, Flags, Kind, NameKind, NameRef, RootIdx, Source, VolIdx};
use peekbuddy_store::batch::Batch;
use peekbuddy_store::schema::TICKS_PER_HOUR;
use peekbuddy_store::writer::{RootInfo, VolInfo, Writer};
use rusqlite::Connection;

fn event(ts: u64, kind: Kind, is_dir: bool) -> Event {
    Event {
        ts,
        qpc: ts,
        src_ts: 0,
        source: Source::Rdcw,
        kind,
        raw_mask: 1,
        source_info: 0,
        is_dir,
        size: 1,
        attrs: 0x20,
        reparse_tag: 0,
        file_id: u128::from(ts),
        parent_file_id: 5,
        usn: 0,
        vol: VolIdx(0),
        root: RootIdx(0),
        name: NameRef::EMPTY,
        name_kind: NameKind::Long,
        flags: Flags::NONE,
    }
}

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("pb-churn-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn db(&self) -> PathBuf {
        self.0.join("events.db")
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn open(path: &Path) -> Writer {
    let mut writer = Writer::open(path).unwrap();
    writer
        .register_root(&RootInfo {
            idx: RootIdx(0),
            vol: VolInfo {
                guid: String::from("vol-1"),
                serial: 1,
                letter: Some(String::from("C:")),
                journal_id: None,
            },
            path: units(r"\\?\C:\Watched"),
            recursive: true,
        })
        .unwrap();
    writer
}

fn batch(rows: &[(u64, Kind, &str)]) -> Batch {
    let mut batch = Batch::with_capacity(rows.len() + 1, 65_536);
    for &(ts, kind, name) in rows {
        batch.push(event(ts, kind, false), &units(name)).unwrap();
    }
    batch
}

type Row = (i64, i64, i64, Option<String>, i64, i64);

fn churn_rows(path: &Path) -> Vec<Row> {
    let conn = Connection::open(path).unwrap();
    let mut statement = conn
        .prepare(
            "SELECT hour_ts, dim, key, label, n, err FROM churn \
             ORDER BY hour_ts, dim, n DESC, key",
        )
        .unwrap();
    statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn node_name(path: &Path, node: i64) -> String {
    Connection::open(path)
        .unwrap()
        .query_row("SELECT name FROM node WHERE id = ?1", [node], |r| r.get(0))
        .unwrap()
}

const HOUR: u64 = TICKS_PER_HOUR as u64;

#[test]
fn the_busiest_path_and_extension_of_the_hour_lead_the_table() {
    let dir = Dir::new("lead");
    let mut writer = open(&dir.db());
    let mut rows = Vec::new();
    for i in 0..50_u64 {
        rows.push((HOUR + i, Kind::Modified, "logs\\app.log"));
    }
    for i in 0..10_u64 {
        rows.push((HOUR + 100 + i, Kind::Created, "src\\main.rs"));
        rows.push((HOUR + 200 + i, Kind::Modified, "Notes.TXT"));
    }
    rows.push((HOUR + 300, Kind::Modified, "README"));
    writer.write(&[batch(&rows)], &[]).unwrap();
    writer.flush_churn().unwrap();

    let table = churn_rows(&dir.db());
    let paths: Vec<_> = table.iter().filter(|r| r.1 == 0).collect();
    let exts: Vec<_> = table.iter().filter(|r| r.1 == 1).collect();
    // Four distinct names in an hour: all followed, counts exact, no error.
    assert_eq!(paths.len(), 4);
    assert!(paths.iter().all(|r| r.5 == 0));
    assert_eq!(node_name(&dir.db(), paths[0].2), "app.log");
    assert_eq!(paths[0].4, 50);
    assert_eq!(
        table.iter().filter(|r| r.1 == 0).map(|r| r.0).next(),
        Some(HOUR.cast_signed())
    );
    // Extensions: LOG 50, RS 10, TXT 10; README has none.
    let labels: Vec<_> = exts.iter().map(|r| (r.3.clone().unwrap(), r.4)).collect();
    assert_eq!(
        labels,
        [
            (String::from("LOG"), 50),
            (String::from("RS"), 10),
            (String::from("TXT"), 10)
        ]
    );
}

#[test]
fn directories_modified_and_system_rows_are_not_churn() {
    let dir = Dir::new("quiet");
    let mut writer = open(&dir.db());
    let mut b = Batch::with_capacity(8, 1024);
    b.push(event(HOUR, Kind::Modified, true), &units("sub"))
        .unwrap();
    b.push(event(HOUR + 1, Kind::Created, true), &units("newdir"))
        .unwrap();
    b.push(
        event(
            HOUR + 2,
            Kind::Overflow(peekbuddy_core::event::GapReason::RingFull),
            false,
        ),
        &[],
    )
    .unwrap();
    b.push(event(HOUR + 3, Kind::ReconcileStarted, false), &[])
        .unwrap();
    writer.write(&[b], &[]).unwrap();
    writer.flush_churn().unwrap();
    let table = churn_rows(&dir.db());
    // Only the created directory.
    assert_eq!(table.iter().filter(|r| r.1 == 0).count(), 1);
    assert_eq!(table.iter().filter(|r| r.1 == 1).count(), 0);
}

#[test]
fn a_finished_hour_is_written_when_the_next_one_starts() {
    let dir = Dir::new("rollover");
    let mut writer = open(&dir.db());
    writer
        .write(&[batch(&[(HOUR + 5, Kind::Created, "a.txt")])], &[])
        .unwrap();
    // Nothing is due yet: the hour runs, the first minute is not over.
    assert_eq!(churn_rows(&dir.db()), []);
    writer
        .write(&[batch(&[(2 * HOUR + 5, Kind::Created, "b.txt")])], &[])
        .unwrap();
    let table = churn_rows(&dir.db());
    // The first hour is final and written; the second is still running.
    assert!(table.iter().all(|r| r.0 == HOUR.cast_signed()), "{table:?}");
    assert_eq!(table.iter().filter(|r| r.1 == 0).count(), 1);
}

#[test]
fn a_running_hour_is_written_once_a_minute_of_event_time_has_passed() {
    let dir = Dir::new("minute");
    let mut writer = open(&dir.db());
    writer
        .write(&[batch(&[(HOUR + 10, Kind::Created, "a.txt")])], &[])
        .unwrap();
    assert_eq!(churn_rows(&dir.db()), []);
    writer
        .write(
            &[batch(&[(HOUR + 10 + 610_000_000, Kind::Modified, "a.txt")])],
            &[],
        )
        .unwrap();
    let table = churn_rows(&dir.db());
    let path = table.iter().find(|r| r.1 == 0).unwrap();
    assert_eq!(path.4, 2);
    // A later write replaces the rows, it does not add to them.
    writer
        .write(
            &[batch(&[(
                HOUR + 10 + 1_300_000_000,
                Kind::Modified,
                "a.txt",
            )])],
            &[],
        )
        .unwrap();
    let table = churn_rows(&dir.db());
    assert_eq!(table.iter().filter(|r| r.1 == 0).count(), 1);
    assert_eq!(table.iter().find(|r| r.1 == 0).unwrap().4, 3);
}

#[test]
fn many_distinct_paths_stay_within_the_counter_budget_and_the_error_bound() {
    let dir = Dir::new("many");
    let mut writer = open(&dir.db());
    let names: Vec<String> = (0..1_000).map(|i| format!("d\\f{i}.dat")).collect();
    let mut rows: Vec<(u64, Kind, &str)> = Vec::new();
    // One hot file, then a long tail of files seen once.
    for i in 0..400_u64 {
        rows.push((HOUR + i, Kind::Modified, "hot\\core.db"));
    }
    for (i, name) in names.iter().enumerate() {
        rows.push((HOUR + 1_000 + i as u64, Kind::Created, name.as_str()));
    }
    for chunk in rows.chunks(200) {
        writer.write(&[batch(chunk)], &[]).unwrap();
    }
    writer.flush_churn().unwrap();
    let table = churn_rows(&dir.db());
    let paths: Vec<_> = table.iter().filter(|r| r.1 == 0).collect();
    assert_eq!(paths.len(), peekbuddy_store::churn::K);
    assert_eq!(node_name(&dir.db(), paths[0].2), "core.db");
    // N = 1400, K = 100: no counter is more than 14 too high.
    assert!(paths.iter().all(|r| r.5 <= 14), "{paths:?}");
    assert!(paths[0].4 >= 400 && paths[0].4 - paths[0].5 <= 400);
}
