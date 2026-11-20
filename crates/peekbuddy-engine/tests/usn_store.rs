//! The USN source resuming from the checkpoints the real store keeps (#260):
//! a data directory of its own, the day segments, the writer pipeline, and a
//! restart of both between two runs. Read-only towards the journal: a changed
//! journal id is a saved position with another id, written through the store.
//! A volume without a journal makes every test return early.
#![cfg(all(windows, feature = "usn"))]
#![expect(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns may unwrap"
)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use peekbuddy_core::capture::EventSink;
use peekbuddy_core::event::{Event, GapReason, Kind, RootIdx, VolIdx};
use peekbuddy_core::usn::checkpoint::Checkpoint;
use peekbuddy_engine::pause_ctl::now_ticks;
use peekbuddy_engine::usn_checkpoints::RootCheckpoints;
use peekbuddy_engine::usn_source::{OnGap, UsnRunner, UsnSpec};
use peekbuddy_store::batch::Batch;
use peekbuddy_store::pipeline::{Config, Pipeline, Producer};
use peekbuddy_store::retention::{Caps, NoViewer};
use peekbuddy_store::segment::{DayKey, Rotation};
use peekbuddy_store::writer::{RootInfo, SegmentsConfig, VolInfo, Writer};

const ROOT: RootIdx = RootIdx(0);

type Rows = Arc<Mutex<Vec<(Event, String)>>>;

/// What the runner delivers goes to the store, and is noted for the test.
struct Tee {
    producer: Producer,
    batch: Option<Batch>,
    rows: Rows,
}

impl EventSink for Tee {
    fn push(&mut self, event: &Event, name: &[u16]) -> bool {
        self.rows
            .lock()
            .unwrap()
            .push((*event, String::from_utf16_lossy(name)));
        let mut batch = self
            .batch
            .take()
            .unwrap_or_else(|| self.producer.new_batch());
        if batch.push(*event, name).is_err() {
            let _ = self.producer.push(batch);
            batch = self.producer.new_batch();
            batch.push(*event, name).unwrap();
        }
        self.batch = Some(batch);
        true
    }

    fn batch_end(&mut self) {
        if let Some(batch) = self.batch.take() {
            let _ = self.producer.push(batch);
        }
    }
}

/// A data directory of its own; the watched directory lives inside it.
struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Self {
        static COUNT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "pb-usn-store-{tag}-{}-{}",
            std::process::id(),
            COUNT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(dir.join("watched")).unwrap();
        Self(dir)
    }

    fn watched(&self) -> PathBuf {
        self.0.join("watched")
    }

    fn data(&self) -> PathBuf {
        self.0.join("data")
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The store the engine runs, opened on the data directory.
fn open_store(dir: &Dir) -> Pipeline {
    let (mut writer, skipped) = Writer::open_segments(
        SegmentsConfig {
            rotation: Rotation {
                dir: dir.data().join("segments"),
                day_of: Arc::new(|_| DayKey {
                    year: 2026,
                    month: 10,
                    day: 1,
                }),
                size_cap: u64::MAX,
            },
            caps: Caps::default(),
            retire: Arc::new(NoViewer),
        },
        now_ticks().0,
    )
    .unwrap();
    assert!(skipped.is_empty(), "{skipped:?}");
    writer
        .register_root(&RootInfo {
            idx: ROOT,
            vol: VolInfo {
                guid: String::from("vol-under-test"),
                serial: 1,
                letter: None,
                journal_id: None,
            },
            path: dir.watched().to_string_lossy().encode_utf16().collect(),
            recursive: true,
        })
        .unwrap();
    Pipeline::start(writer, Config::new(dir.data().join("spill"))).unwrap()
}

/// Quitting: everything to disk, the writer stopped, the final checkpoint.
fn close_store(pipeline: Pipeline) {
    pipeline.flush(Duration::from_secs(10)).unwrap();
    let mut writer = pipeline.close().unwrap();
    writer.wait_background();
    writer.flush_system_rows().unwrap();
    writer.checkpoint_truncate().unwrap();
}

/// The position the store has after a restart.
fn saved(dir: &Dir) -> Option<Checkpoint> {
    let pipeline = open_store(dir);
    let saved = RootCheckpoints::new(pipeline.producer(), ROOT).saved();
    close_store(pipeline);
    saved
}

struct Started {
    runner: UsnRunner,
    rows: Rows,
    gaps: Arc<Mutex<Vec<GapReason>>>,
}

/// Starts a runner on the watched directory with the store's checkpoints;
/// `None` when the volume has no journal.
fn start(dir: &Dir, store: &Pipeline) -> Option<Started> {
    let rows: Rows = Arc::default();
    let gaps: Arc<Mutex<Vec<GapReason>>> = Arc::default();
    let on_gap: OnGap = {
        let gaps = Arc::clone(&gaps);
        Box::new(move |reason| gaps.lock().unwrap().push(reason))
    };
    let watched = dir.watched();
    let spec = UsnSpec {
        vol: VolIdx(0),
        root: ROOT,
        path: &watched,
    };
    let sink = Tee {
        producer: store.producer(),
        batch: None,
        rows: Arc::clone(&rows),
    };
    match UsnRunner::start(
        &spec,
        Box::new(RootCheckpoints::new(store.producer(), ROOT)),
        Box::new(sink),
        on_gap,
        now_ticks,
    ) {
        Ok(runner) => Some(Started { runner, rows, gaps }),
        // ERROR_INVALID_FUNCTION (1) or ERROR_JOURNAL_NOT_ACTIVE (1179).
        Err(error) if matches!(error.raw_os_error(), Some(1 | 1179)) => None,
        Err(error) => {
            assert_eq!(error.to_string(), "", "start failed");
            None
        }
    }
}

fn write(dir: &Dir, name: &str) {
    std::fs::write(dir.watched().join(name), b"x").unwrap();
}

/// Waits until a row with `name` and `kind` is there.
fn wait_for(rows: &Rows, name: &str, kind: Kind) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !rows
        .lock()
        .unwrap()
        .iter()
        .any(|(event, n)| n == name && event.kind == kind)
    {
        assert!(Instant::now() < deadline, "no {kind:?} row for {name}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn gap_rows(rows: &Rows) -> Vec<GapReason> {
    rows.lock()
        .unwrap()
        .iter()
        .filter_map(|(event, _)| match event.kind {
            Kind::Overflow(reason) => Some(reason),
            _ => None,
        })
        .collect()
}

fn created(rows: &Rows, name: &str) -> usize {
    rows.lock()
        .unwrap()
        .iter()
        .filter(|(event, n)| n == name && event.kind == Kind::Created)
        .count()
}

#[test]
fn a_restart_through_the_store_continues_without_a_gap_or_a_record_read_twice() {
    let dir = Dir::new("intact");
    let store = open_store(&dir);
    let Some(mut first) = start(&dir, &store) else {
        return;
    };
    write(&dir, "before.txt");
    wait_for(&first.rows, "before.txt", Kind::Created);
    assert!(first.runner.stop().is_none());
    close_store(store);
    // The store kept the position.
    let position = saved(&dir).unwrap();

    // Written while neither the source nor the store runs.
    write(&dir, "between.txt");

    let store = open_store(&dir);
    let Some(mut second) = start(&dir, &store) else {
        return;
    };
    wait_for(&second.rows, "between.txt", Kind::Created);
    assert!(second.runner.stop().is_none());
    close_store(store);
    assert_eq!(gap_rows(&second.rows), []);
    assert_eq!(*second.gaps.lock().unwrap(), []);
    // The create of `before.txt` lies before the saved position, and no
    // journal record of the first run is delivered again. (Later records of
    // a file, a write or the close, are records of their own.)
    assert_eq!(created(&second.rows, "before.txt"), 0);
    let before = usns(&first.rows);
    let again = usns(&second.rows);
    assert_ne!(again, Vec::<i64>::new());
    assert!(again.iter().all(|usn| *usn >= position.next_usn));
    assert!(again.iter().all(|usn| !before.contains(usn)), "{again:?}");
}

/// The USNs of the delivered record rows.
fn usns(rows: &Rows) -> Vec<i64> {
    rows.lock()
        .unwrap()
        .iter()
        .filter_map(|(event, _)| (event.usn != 0).then_some(event.usn))
        .collect()
}

#[test]
fn a_changed_journal_id_in_the_store_gives_exactly_one_gap_row() {
    let dir = Dir::new("idchange");
    let store = open_store(&dir);
    let Some(mut first) = start(&dir, &store) else {
        return;
    };
    assert!(first.runner.stop().is_none());
    // The journal was recreated while the engine was away: the saved
    // position names the old journal.
    let checkpoints = RootCheckpoints::new(store.producer(), ROOT);
    let mut position = checkpoints.saved().unwrap();
    position.journal_id ^= 1;
    checkpoints.record(position);
    close_store(store);
    assert_eq!(saved(&dir), Some(position));

    let store = open_store(&dir);
    let Some(mut second) = start(&dir, &store) else {
        return;
    };
    assert_eq!(gap_rows(&second.rows), [GapReason::JournalIdChanged]);
    assert_eq!(*second.gaps.lock().unwrap(), [GapReason::JournalIdChanged]);
    // Reading goes on after the gap.
    write(&dir, "after.txt");
    wait_for(&second.rows, "after.txt", Kind::Created);
    assert!(second.runner.stop().is_none());
    close_store(store);
    assert_eq!(gap_rows(&second.rows).len(), 1);
    // The store names the live journal again: the next start has no gap.
    let live = saved(&dir).unwrap();
    assert_ne!(live.journal_id, position.journal_id);
    let store = open_store(&dir);
    let Some(mut third) = start(&dir, &store) else {
        return;
    };
    assert!(third.runner.stop().is_none());
    close_store(store);
    assert_eq!(gap_rows(&third.rows), []);
}
