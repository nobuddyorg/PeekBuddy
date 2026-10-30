//! The writer pipeline against a real database file: ordering, stalls,
//! spill and replay, restarts, lost batches, flush and cursors (#52).
#![expect(clippy::unwrap_used, reason = "tests unwrap")]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use peekbuddy_core::event::{Event, Flags, Kind, NameKind, NameRef, RootIdx, Source, VolIdx};
use peekbuddy_store::batch::Batch;
use peekbuddy_store::pipeline::{Config, Hooks, Pipeline, Push};
use peekbuddy_store::policy::Policy;
use peekbuddy_store::writer::{Cursor, RootInfo, VolInfo, Writer};
use proptest::prelude::*;
use proptest::test_runner::{Config as ProptestConfig, TestRunner};
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

struct Dirs {
    root: PathBuf,
}

impl Dirs {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("pb-pipeline-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Self { root }
    }
    fn db(&self) -> PathBuf {
        self.root.join("segment.db")
    }
    fn spill(&self) -> PathBuf {
        self.root.join("spill")
    }
}

impl Drop for Dirs {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
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

/// Fills one batch with consecutive `ts` values.
fn fill(batch: &mut Batch, first: u64, count: u64) {
    for ts in first..first + count {
        batch
            .push(event(ts), &units(&format!("d\\f{ts}.txt")))
            .unwrap();
    }
}

fn push_events(producer: &peekbuddy_store::pipeline::Producer, first: u64, count: u64) -> bool {
    let mut batch = producer.new_batch();
    fill(&mut batch, first, count);
    matches!(producer.push(batch), Push::Overflow(_))
}

/// The `ts` of every stored event, in id order.
fn stored_ts(db: &Path) -> Vec<u64> {
    let conn = Connection::open(db).unwrap();
    let mut statement = conn.prepare("SELECT ts FROM event ORDER BY id").unwrap();
    statement
        .query_map([], |row| row.get::<_, i64>(0))
        .unwrap()
        .map(|ts| u64::try_from(ts.unwrap()).unwrap())
        .collect()
}

fn ids_are_contiguous(db: &Path) -> bool {
    let conn = Connection::open(db).unwrap();
    let (count, min, max): (i64, i64, i64) = conn
        .query_row(
            "SELECT count(*), coalesce(min(id), 0), coalesce(max(id), 0) FROM event",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    count == 0 || (min == 1 && max == count)
}

fn config(dirs: &Dirs) -> Config {
    let mut config = Config::new(dirs.spill());
    config.ring_batches = 4;
    config
}

#[test]
fn rows_arrive_in_exactly_the_pushed_order() {
    let dirs = Dirs::new("order");
    let pipeline = Pipeline::start(open_writer(&dirs.db()), config(&dirs)).unwrap();
    let producer = pipeline.producer();
    for first in (1..=2_000).step_by(50) {
        push_events(&producer, first, 50);
    }
    let flushed = pipeline.flush(Duration::from_secs(20)).unwrap();
    assert_eq!(flushed.rows, 2_000);
    assert_eq!(flushed.lost_batches, 0);
    let _writer = pipeline.close().unwrap();
    assert_eq!(stored_ts(&dirs.db()), (1..=2_000).collect::<Vec<_>>());
    assert!(ids_are_contiguous(&dirs.db()));
}

#[test]
fn a_stalled_writer_spills_and_the_replay_keeps_the_order() {
    let dirs = Dirs::new("stall");
    let gate = Arc::new(Mutex::new(()));
    let blocked = Arc::new(AtomicBool::new(true));
    let mut config = config(&dirs);
    config.hooks = Hooks {
        before_commit: Some({
            let (gate, blocked) = (Arc::clone(&gate), Arc::clone(&blocked));
            Arc::new(move || {
                while blocked.load(Ordering::Acquire) {
                    drop(gate.lock().unwrap());
                    std::thread::sleep(Duration::from_millis(5));
                }
            })
        }),
        on_commit: None,
        before_attempt: None,
    };
    let pipeline = Pipeline::start(open_writer(&dirs.db()), config).unwrap();
    let producer = pipeline.producer();
    // The first batch is taken by the stalled writer; the ring holds 4; the
    // rest overflows to the spill file.
    // Batches of 250 events: four of them reach the 1000-event cap, so the
    // stalled writer holds a full group and cannot absorb the rest.
    let mut spilled = false;
    for index in 0..30u64 {
        spilled |= push_events(&producer, index * 250 + 1, 250);
    }
    assert!(
        spilled,
        "nothing overflowed although the writer was stalled"
    );
    blocked.store(false, Ordering::Release);
    let flushed = pipeline.flush(Duration::from_secs(30)).unwrap();
    assert_eq!(flushed.rows, 7_500);
    let stats = pipeline.stats();
    assert!(stats.spilled_batches > 0, "{stats:?}");
    assert_eq!(stats.lost_batches, 0);
    let _writer = pipeline.close().unwrap();
    assert_eq!(stored_ts(&dirs.db()), (1..=7_500).collect::<Vec<_>>());
    assert!(ids_are_contiguous(&dirs.db()));
    assert!(
        !dirs.spill().join("spill.bin").exists(),
        "the spill file was not removed"
    );
}

#[test]
fn a_spill_left_by_a_previous_run_is_replayed_first() {
    let dirs = Dirs::new("restart");
    // Run 1: stall, overflow, then close without letting the writer replay.
    let blocked = Arc::new(AtomicBool::new(true));
    let mut config1 = config(&dirs);
    config1.hooks = Hooks {
        before_commit: Some({
            let blocked = Arc::clone(&blocked);
            Arc::new(move || {
                while blocked.load(Ordering::Acquire) {
                    std::thread::sleep(Duration::from_millis(5));
                }
            })
        }),
        on_commit: None,
        before_attempt: None,
    };
    let pipeline = Pipeline::start(open_writer(&dirs.db()), config1).unwrap();
    let producer = pipeline.producer();
    for index in 0..20u64 {
        push_events(&producer, index * 250 + 1, 250);
    }
    // Release the stall only after close() has been asked for.
    let release = Arc::clone(&blocked);
    let closer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        release.store(false, Ordering::Release);
    });
    drop(pipeline.close());
    closer.join().unwrap();
    let committed_before = stored_ts(&dirs.db()).len();
    assert!(
        committed_before < 5_000,
        "everything was committed before the restart"
    );
    assert!(dirs.spill().join("spill.bin").exists());

    // Run 2: the old spill is replayed before new events, ids continue.
    let pipeline = Pipeline::start(open_writer(&dirs.db()), config(&dirs)).unwrap();
    let producer = pipeline.producer();
    push_events(&producer, 100_000, 10);
    pipeline.flush(Duration::from_secs(30)).unwrap();
    let _writer = pipeline.close().unwrap();
    // Queued batches were committed on close and spilled ones replayed on
    // the next start: nothing is missing and the order is the pushed order.
    let mut expected: Vec<u64> = (1..=5_000).collect();
    expected.extend(100_000..100_010);
    assert_eq!(stored_ts(&dirs.db()), expected);
    assert!(ids_are_contiguous(&dirs.db()));
}

#[test]
fn a_batch_that_cannot_be_spilled_is_counted_lost_not_silent() {
    let dirs = Dirs::new("lost");
    let blocked = Arc::new(AtomicBool::new(true));
    let mut config = config(&dirs);
    config.hooks = Hooks {
        before_commit: Some({
            let blocked = Arc::clone(&blocked);
            Arc::new(move || {
                while blocked.load(Ordering::Acquire) {
                    std::thread::sleep(Duration::from_millis(5));
                }
            })
        }),
        on_commit: None,
        before_attempt: None,
    };
    let pipeline = Pipeline::start(open_writer(&dirs.db()), config).unwrap();
    // The spill directory disappears: appending to it fails, like a full disk.
    std::fs::remove_dir_all(dirs.spill()).unwrap();
    let producer = pipeline.producer();
    for index in 0..20u64 {
        push_events(&producer, index * 250 + 1, 250);
    }
    std::thread::sleep(Duration::from_millis(200));
    blocked.store(false, Ordering::Release);
    let flushed = pipeline.flush(Duration::from_secs(20)).unwrap();
    assert!(flushed.lost_batches > 0, "a loss went unnoticed");
    let stats = pipeline.stats();
    assert_eq!(stats.rows + stats.lost_batches * 250, 5_000, "{stats:?}");
    drop(pipeline.close());
}

#[test]
fn cursors_are_committed_with_the_rows_they_cover() {
    let dirs = Dirs::new("cursor");
    let pipeline = Pipeline::start(open_writer(&dirs.db()), config(&dirs)).unwrap();
    let producer = pipeline.producer();
    push_events(&producer, 1, 10);
    producer.set_cursor(Cursor {
        root: RootIdx(0),
        usn: Some(777),
    });
    pipeline.flush(Duration::from_secs(10)).unwrap();
    // A cursor alone (no rows) is written too, at the next opportunity.
    producer.set_cursor(Cursor {
        root: RootIdx(0),
        usn: Some(888),
    });
    std::thread::sleep(Duration::from_millis(300));
    drop(pipeline.close());
    let conn = Connection::open(dirs.db()).unwrap();
    let (usn, last): (i64, i64) = conn
        .query_row("SELECT usn, last_event_id FROM cursor", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(usn, 888);
    assert_eq!(last, 10);
    let ahead: i64 = conn
        .query_row(
            "SELECT count(*) FROM cursor WHERE last_event_id > (SELECT max(id) FROM event)",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(ahead, 0);
}

#[test]
fn the_commit_observer_sees_every_commit_and_the_group_commit_batches_them() {
    let dirs = Dirs::new("observer");
    let commits = Arc::new(AtomicU64::new(0));
    let rows = Arc::new(AtomicU64::new(0));
    let mut config = config(&dirs);
    config.policy = Policy {
        linger: Duration::from_millis(20),
        ..Policy::default()
    };
    config.hooks = Hooks {
        before_commit: None,
        on_commit: Some({
            let (commits, rows) = (Arc::clone(&commits), Arc::clone(&rows));
            Arc::new(move |info| {
                commits.fetch_add(1, Ordering::Relaxed);
                rows.fetch_add(info.commit.rows, Ordering::Relaxed);
            })
        }),
        before_attempt: None,
    };
    let pipeline = Pipeline::start(open_writer(&dirs.db()), config).unwrap();
    let producer = pipeline.producer();
    // Many small batches within one linger become few commits.
    for index in 0..40u64 {
        push_events(&producer, index * 5 + 1, 5);
    }
    pipeline.flush(Duration::from_secs(10)).unwrap();
    assert_eq!(rows.load(Ordering::Relaxed), 200);
    assert!(commits.load(Ordering::Relaxed) < 40, "no batching happened");
    drop(pipeline.close());
}

#[test]
fn closing_refuses_new_batches_and_returns_the_writer() {
    let dirs = Dirs::new("close");
    let pipeline = Pipeline::start(open_writer(&dirs.db()), config(&dirs)).unwrap();
    let producer = pipeline.producer();
    push_events(&producer, 1, 5);
    let writer = pipeline.close().unwrap();
    assert_eq!(writer.next_id(), 6);
    let mut batch = producer.new_batch();
    fill(&mut batch, 9, 1);
    assert!(matches!(producer.push(batch), Push::Closed(_)));
    writer.checkpoint_truncate().unwrap();
}

#[derive(Debug, Clone)]
enum Op {
    Push(u8),
    Stall,
    Resume,
    Flush,
    Restart,
}

fn ops() -> impl Strategy<Value = Vec<Op>> {
    proptest::collection::vec(
        prop_oneof![
            4 => (1u8..30).prop_map(Op::Push),
            1 => Just(Op::Stall),
            1 => Just(Op::Resume),
            1 => Just(Op::Flush),
            1 => Just(Op::Restart),
        ],
        1..24,
    )
}

fn start_with_gate(dirs: &Dirs, stalled: &Arc<AtomicBool>) -> Pipeline {
    let mut config = config(dirs);
    config.policy = Policy {
        linger: Duration::from_millis(2),
        ..Policy::default()
    };
    config.hooks = Hooks {
        before_commit: Some({
            let stalled = Arc::clone(stalled);
            Arc::new(move || {
                while stalled.load(Ordering::Acquire) {
                    std::thread::sleep(Duration::from_millis(2));
                }
            })
        }),
        on_commit: None,
        before_attempt: None,
    };
    Pipeline::start(open_writer(&dirs.db()), config).unwrap()
}

/// Cases for the heavy property tests. `PROPTEST_CASES` (256 in CI) is far too
/// many for a test that writes database files, and the `proptest!` macro lets
/// it override any explicit count, so these tests run a `TestRunner` of their
/// own: four cases by default, `PEEKBUDDY_HEAVY_CASES` for the nightly job.
fn heavy_cases() -> u32 {
    std::env::var("PEEKBUDDY_HEAVY_CASES")
        .ok()
        .and_then(|text| text.parse().ok())
        .unwrap_or(4)
}

#[test]
fn any_interleaving_gives_every_pushed_row_exactly_once_in_order() {
    let mut runner = TestRunner::new(ProptestConfig {
        cases: heavy_cases(),
        ..ProptestConfig::default()
    });
    runner
        .run(&ops(), |ops| {
            let dirs = Dirs::new(&format!("prop{}", rand_tag()));
            let stalled = Arc::new(AtomicBool::new(false));
            let mut pipeline = start_with_gate(&dirs, &stalled);
            let mut next_ts = 1u64;
            let mut pushed: Vec<u64> = Vec::new();
            for op in ops {
                match op {
                    Op::Push(count) => {
                        let producer = pipeline.producer();
                        let count = u64::from(count);
                        push_events(&producer, next_ts, count);
                        pushed.extend(next_ts..next_ts + count);
                        next_ts += count;
                    }
                    Op::Stall => stalled.store(true, Ordering::Release),
                    Op::Resume => stalled.store(false, Ordering::Release),
                    Op::Flush => {
                        stalled.store(false, Ordering::Release);
                        pipeline.flush(Duration::from_secs(30)).unwrap();
                    }
                    Op::Restart => {
                        // A restart after a flush: nothing in flight is lost.
                        stalled.store(false, Ordering::Release);
                        pipeline.flush(Duration::from_secs(30)).unwrap();
                        drop(pipeline.close());
                        pipeline = start_with_gate(&dirs, &stalled);
                    }
                }
            }
            stalled.store(false, Ordering::Release);
            pipeline.flush(Duration::from_secs(30)).unwrap();
            drop(pipeline.close());
            prop_assert_eq!(stored_ts(&dirs.db()), pushed);
            prop_assert!(ids_are_contiguous(&dirs.db()));
            Ok(())
        })
        .unwrap();
}

fn rand_tag() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}
