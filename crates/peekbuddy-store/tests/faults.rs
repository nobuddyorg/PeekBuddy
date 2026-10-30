//! Fault injection in the store layer (#57): a full disk and an unwritable
//! file at the Nth write, with the pipeline around the writer. Nothing may be
//! lost or corrupted, the failing state must be visible, and the store must
//! recover on its own once writes work again.
#![expect(clippy::unwrap_used, reason = "tests unwrap")]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use peekbuddy_core::event::{Event, Flags, Kind, NameKind, NameRef, RootIdx, Source, VolIdx};
use peekbuddy_store::batch::Batch;
use peekbuddy_store::pipeline::{Config, Hooks, Pipeline, Producer, Push};
use peekbuddy_store::writer::{RootInfo, VolInfo, Writer};
use rusqlite::Connection;

fn event(ts: u64) -> Event {
    Event {
        ts,
        qpc: ts,
        src_ts: 0,
        source: Source::Rdcw,
        kind: Kind::Created,
        raw_mask: 1,
        source_info: 0,
        is_dir: false,
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

struct Dirs(PathBuf);

impl Dirs {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("pb-faults-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }

    fn db(&self) -> PathBuf {
        self.0.join("segment.db")
    }

    fn spill(&self) -> PathBuf {
        self.0.join("spill")
    }
}

impl Drop for Dirs {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn open_writer(db: &Path) -> Writer {
    let mut writer = Writer::open(db).unwrap();
    writer
        .register_root(&RootInfo {
            idx: RootIdx(0),
            vol: VolInfo {
                guid: String::from("vol-1"),
                serial: 1,
                letter: Some(String::from("C:")),
                journal_id: None,
            },
            path: units(r"\\?\C:\watched"),
            recursive: true,
        })
        .unwrap();
    writer
}

fn push(producer: &Producer, first: u64, count: u64) -> bool {
    let mut batch = producer.new_batch();
    for ts in first..first + count {
        // A long name: every batch needs new pages, so a frozen database size
        // fails the commit at once instead of after the free pages are used.
        batch
            .push(
                event(ts),
                &units(&format!("d\\f{ts}-{}.txt", "x".repeat(200))),
            )
            .unwrap();
    }
    matches!(producer.push(batch), Push::Overflow(_))
}

fn stored_ts(db: &Path) -> Vec<u64> {
    let conn = Connection::open(db).unwrap();
    let mut statement = conn.prepare("SELECT ts FROM event ORDER BY id").unwrap();
    statement
        .query_map([], |row| row.get::<_, i64>(0))
        .unwrap()
        .map(|ts| u64::try_from(ts.unwrap()).unwrap())
        .collect()
}

fn integrity_ok(db: &Path) -> bool {
    let conn = Connection::open(db).unwrap();
    let result: String = conn
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap();
    result == "ok"
}

fn wait_for(what: &str, mut condition: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(30);
    while !condition() {
        assert!(Instant::now() < until, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// What the injected fault does to the writer.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Fault {
    /// The database may not grow: `SQLITE_FULL`.
    Full,
    /// Every write is refused: `SQLITE_READONLY`.
    ReadOnly,
}

/// A pipeline whose writer hits `fault` while the returned flag is set.
fn faulty_pipeline(dirs: &Dirs, fault: Fault, faulting: &Arc<AtomicBool>) -> Pipeline {
    let mut config = Config::new(dirs.spill());
    config.ring_batches = 4;
    let (flag, pages) = (Arc::clone(faulting), Arc::new(AtomicU32::new(0)));
    config.hooks = Hooks {
        before_attempt: Some(Arc::new(move |writer: &Writer| {
            match (fault, flag.load(Ordering::Relaxed)) {
                (Fault::Full, true) => {
                    // Freeze the database at the size it has now.
                    let now = pages.load(Ordering::Relaxed);
                    let pages_now = if now == 0 {
                        let now = writer.page_count().unwrap();
                        pages.store(now, Ordering::Relaxed);
                        now
                    } else {
                        now
                    };
                    writer.limit_pages(Some(pages_now)).unwrap();
                }
                (Fault::Full, false) => {
                    pages.store(0, Ordering::Relaxed);
                    writer.limit_pages(None).unwrap();
                }
                (Fault::ReadOnly, on) => writer.set_query_only(on).unwrap(),
            }
        })),
        ..Hooks::default()
    };
    Pipeline::start(open_writer(&dirs.db()), config).unwrap()
}

/// Pushes until the writer reports it is failing, then keeps pushing so the
/// ring fills and the overflow spills. Returns the next unused `ts`.
fn push_through_the_fault(pipeline: &Pipeline, start: u64) -> u64 {
    let producer = pipeline.producer();
    let mut next = start;
    wait_for("the writer to fail", || {
        push(&producer, next, 20);
        next += 20;
        pipeline.stats().failing
    });
    for _ in 0..40 {
        push(&producer, next, 20);
        next += 20;
    }
    next
}

#[test]
fn a_full_disk_loses_nothing_and_recovers_by_itself() {
    let dirs = Dirs::new("full");
    let faulting = Arc::new(AtomicBool::new(false));
    let pipeline = faulty_pipeline(&dirs, Fault::Full, &faulting);
    let producer = pipeline.producer();
    // Healthy first: the database has pages to freeze at.
    push(&producer, 1, 50);
    pipeline.flush(Duration::from_secs(30)).unwrap();

    faulting.store(true, Ordering::Relaxed);
    let next = push_through_the_fault(&pipeline, 51);
    let stats = pipeline.stats();
    assert!(stats.failing, "the failing state is visible");
    assert!(stats.commit_failures > 0);
    assert_eq!(stats.lost_batches, 0, "nothing is dropped");
    // The spill thread counts a batch after it wrote it, which may be a moment
    // after the pushes returned.
    wait_for("the spill file to take over", || {
        pipeline.stats().spilled_batches > 0
    });

    // Space is freed: the writer recovers, everything arrives once, in order.
    faulting.store(false, Ordering::Relaxed);
    pipeline.flush(Duration::from_secs(60)).unwrap();
    assert!(!pipeline.stats().failing);
    assert_eq!(pipeline.stats().lost_batches, 0);
    drop(pipeline.close());
    assert_eq!(stored_ts(&dirs.db()), (1..next).collect::<Vec<u64>>());
    assert!(integrity_ok(&dirs.db()));
}

#[test]
fn an_unwritable_file_is_the_same_story() {
    let dirs = Dirs::new("readonly");
    let faulting = Arc::new(AtomicBool::new(false));
    let pipeline = faulty_pipeline(&dirs, Fault::ReadOnly, &faulting);
    let producer = pipeline.producer();
    push(&producer, 1, 30);
    pipeline.flush(Duration::from_secs(30)).unwrap();

    faulting.store(true, Ordering::Relaxed);
    let next = push_through_the_fault(&pipeline, 31);
    assert!(pipeline.stats().failing);
    assert_eq!(pipeline.stats().lost_batches, 0);

    faulting.store(false, Ordering::Relaxed);
    pipeline.flush(Duration::from_secs(60)).unwrap();
    drop(pipeline.close());
    assert_eq!(stored_ts(&dirs.db()), (1..next).collect::<Vec<u64>>());
    assert!(integrity_ok(&dirs.db()));
}

#[test]
fn closing_while_the_disk_is_full_keeps_the_data_for_the_next_start() {
    let dirs = Dirs::new("close");
    let faulting = Arc::new(AtomicBool::new(false));
    let pipeline = faulty_pipeline(&dirs, Fault::Full, &faulting);
    push(&pipeline.producer(), 1, 50);
    pipeline.flush(Duration::from_secs(30)).unwrap();

    faulting.store(true, Ordering::Relaxed);
    let next = push_through_the_fault(&pipeline, 51);
    // Shut down with the disk still full: the data goes to the spill file.
    let stats_before = pipeline.stats();
    assert_eq!(stats_before.lost_batches, 0);
    drop(pipeline.close());
    assert!(u64::try_from(stored_ts(&dirs.db()).len()).unwrap() < next - 1);

    // The next start, with space, replays it.
    let restarted = Pipeline::start(open_writer(&dirs.db()), Config::new(dirs.spill())).unwrap();
    // The old spill is replayed before anything new; a flush covers what was
    // pushed in this run, so push one more row and wait for it.
    push(&restarted.producer(), next, 1);
    restarted.flush(Duration::from_secs(60)).unwrap();
    drop(restarted.close());
    // Every row arrives exactly once. The ids may not follow capture order
    // around the shutdown: the group the writer held went to the end of the
    // spill file, behind the newer batches that overflowed earlier. The `ts`
    // column keeps the true order.
    let mut got = stored_ts(&dirs.db());
    got.sort_unstable();
    assert_eq!(got, (1..=next).collect::<Vec<u64>>());
    assert!(integrity_ok(&dirs.db()));
}

fn write_rows(writer: &mut Writer, first: u64, count: u64) {
    let mut batch = Batch::with_capacity(usize::try_from(count).unwrap() + 1, 1 << 20);
    for ts in first..first + count {
        batch
            .push(event(ts), &units(&format!("d\\f{ts}.txt")))
            .unwrap();
    }
    writer.write(&[batch], &[]).unwrap();
}

#[test]
fn a_reader_holding_a_snapshot_stops_the_wal_resetting_until_it_lets_go() {
    let dirs = Dirs::new("wal");
    let mut writer = open_writer(&dirs.db());
    let mut next = 1;
    for _ in 0..40 {
        write_rows(&mut writer, next, 200);
        next += 200;
    }
    let grown = writer.wal_bytes();
    assert!(grown > 100_000, "the WAL holds the commits: {grown}");

    // A reader opens a snapshot and sits on it, as a stuck viewer would.
    let reader = Connection::open(dirs.db()).unwrap();
    reader.execute_batch("BEGIN").unwrap();
    let seen: i64 = reader
        .query_row("SELECT count(*) FROM event", [], |row| row.get(0))
        .unwrap();
    assert_eq!(seen, 8_000);

    // The writer keeps ingesting; checkpoints cannot reset the log, so it grows.
    writer.checkpoint_passive().unwrap();
    for _ in 0..40 {
        write_rows(&mut writer, next, 200);
        next += 200;
    }
    let while_held = writer.wal_bytes();
    assert!(
        while_held > grown,
        "ingest continues and the WAL grows: {while_held} <= {grown}"
    );

    // The reader lets go: the next checkpoint and commit reuse the log from
    // its start, so it stops growing.
    reader.execute_batch("COMMIT").unwrap();
    writer.checkpoint_passive().unwrap();
    for _ in 0..40 {
        write_rows(&mut writer, next, 200);
        next += 200;
        writer.checkpoint_passive().unwrap();
    }
    let after = writer.wal_bytes();
    assert!(
        after <= while_held,
        "the WAL is reused: {after} > {while_held}"
    );
    assert!(integrity_ok(&dirs.db()));
}
