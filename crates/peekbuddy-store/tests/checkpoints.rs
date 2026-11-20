//! Resuming a journal read through the real store (#260): the position is
//! saved behind the rows it covers, survives a restart of the store in a data
//! directory of its own, and plans the resume the way the USN source does at
//! start. An intact journal continues without a gap and without reading a
//! record twice; a changed journal id writes exactly one gap row.
#![expect(clippy::unwrap_used, reason = "tests unwrap")]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use peekbuddy_core::event::{
    Event, Flags, GapReason, Kind, NameKind, NameRef, RootIdx, Source, VolIdx,
};
use peekbuddy_core::usn::checkpoint::{Checkpoint, JournalData, Resume, plan_resume};
use peekbuddy_store::checkpoints::RootCheckpoints;
use peekbuddy_store::pipeline::{Config, Pipeline, Producer, Push};
use peekbuddy_store::retention::{Caps, NoViewer};
use peekbuddy_store::segment::{DayKey, Rotation};
use peekbuddy_store::writer::{RootInfo, SegmentsConfig, VolInfo, Writer};
use rusqlite::Connection;

const ROOT: RootIdx = RootIdx(0);
/// 2026-10-01 as FILETIME ticks, roughly: every row lands in one segment.
const NOW: u64 = 134_037_000_000_000_000;
/// Records per journal buffer.
const BUFFER: i64 = 4;

struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("pb-checkpoints-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn segments(&self) -> PathBuf {
        self.0.join("segments")
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The journal as the source finds it at start.
#[derive(Clone, Copy)]
struct Journal {
    id: u64,
    first: i64,
    next: i64,
}

impl Journal {
    const fn data(self) -> JournalData {
        JournalData {
            journal_id: self.id,
            first_usn: self.first,
            next_usn: self.next,
            lowest_valid_usn: self.first,
        }
    }
}

/// The store the engine runs: day segments under `dir`, the root, the pipeline.
fn start(dir: &Dir) -> Pipeline {
    let (mut writer, skipped) = Writer::open_segments(
        SegmentsConfig {
            rotation: Rotation {
                dir: dir.segments(),
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
        NOW,
    )
    .unwrap();
    assert!(skipped.is_empty(), "{skipped:?}");
    writer
        .register_root(&RootInfo {
            idx: ROOT,
            vol: VolInfo {
                guid: String::from(r"\\?\Volume{1}\"),
                serial: 1,
                letter: Some(String::from("C:")),
                journal_id: None,
            },
            path: r"\\?\C:\Watched".encode_utf16().collect(),
            recursive: true,
        })
        .unwrap();
    Pipeline::start(writer, Config::new(dir.0.join("spill"))).unwrap()
}

/// Flushes, stops the writer and runs the final checkpoint, like quitting.
fn stop(pipeline: Pipeline) {
    pipeline.flush(Duration::from_secs(10)).unwrap();
    let mut writer = pipeline.close().unwrap();
    writer.wait_background();
    writer.flush_system_rows().unwrap();
    writer.checkpoint_truncate().unwrap();
}

fn row(kind: Kind, usn: i64) -> Event {
    Event {
        ts: NOW + u64::try_from(usn).unwrap(),
        qpc: 0,
        src_ts: 0,
        source: Source::Usn,
        kind,
        raw_mask: 0,
        source_info: 0,
        is_dir: false,
        size: 0,
        attrs: 0,
        reparse_tag: 0,
        file_id: 0,
        parent_file_id: 0,
        usn,
        vol: VolIdx(0),
        root: ROOT,
        name: NameRef::EMPTY,
        name_kind: NameKind::Long,
        flags: Flags::NONE,
    }
}

fn push(producer: &Producer, rows: impl IntoIterator<Item = (Event, String)>) {
    let mut batch = producer.new_batch();
    for (event, name) in rows {
        let units: Vec<u16> = name.encode_utf16().collect();
        batch.push(event, &units).unwrap();
    }
    assert!(matches!(producer.push(batch), Push::Queued(_)));
}

/// One run of the source against the store: what `UsnRunner` does at setup
/// (plan the resume from the saved position, write the gap row a journal
/// that is not intact owes, save the position it reads from) and then in its
/// loop for the records `from..until` (a buffer of rows, then the position
/// behind it). Returns the plan.
fn run(dir: &Dir, journal: Journal, until: i64) -> Resume {
    let pipeline = start(dir);
    let producer = pipeline.producer();
    let checkpoints = RootCheckpoints::new(pipeline.producer(), ROOT);
    let plan = plan_resume(checkpoints.saved(), &journal.data());
    let from = match plan {
        Resume::Continue { from } | Resume::Fresh { from } => from,
        Resume::Gap { reason, from } => {
            push(&producer, [(row(Kind::Overflow(reason), 0), String::new())]);
            from
        }
    };
    checkpoints.record(Checkpoint {
        journal_id: journal.id,
        next_usn: from,
    });
    let mut next = from;
    while next < until {
        let end = (next + BUFFER).min(until);
        push(
            &producer,
            (next..end).map(|usn| (row(Kind::Created, usn), format!("f{usn}.txt"))),
        );
        checkpoints.record(Checkpoint {
            journal_id: journal.id,
            next_usn: end,
        });
        next = end;
    }
    stop(pipeline);
    plan
}

/// The position the store has for the root after a restart.
fn saved(dir: &Dir) -> Option<Checkpoint> {
    let pipeline = start(dir);
    let saved = RootCheckpoints::new(pipeline.producer(), ROOT).saved();
    stop(pipeline);
    saved
}

fn segment(dir: &Dir) -> Connection {
    Connection::open(dir.segments().join("events-2026-10-01.db")).unwrap()
}

/// The USNs of the stored record rows, in id order.
fn record_usns(dir: &Dir) -> Vec<i64> {
    let (tag, arg) = Kind::Created.to_wire();
    let conn = segment(dir);
    let mut select = conn
        .prepare("SELECT usn FROM event WHERE kind = ?1 ORDER BY id")
        .unwrap();
    select
        .query_map([i64::from(tag) * 256 + i64::from(arg)], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// The stored gap rows of the root.
fn gap_rows(dir: &Dir) -> Vec<i64> {
    let conn = segment(dir);
    let mut select = conn
        .prepare("SELECT kind FROM event WHERE kind / 256 = ?1 ORDER BY id")
        .unwrap();
    let (tag, _) = Kind::Overflow(GapReason::JournalIdChanged).to_wire();
    select
        .query_map([i64::from(tag)], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn wire(kind: Kind) -> i64 {
    let (tag, arg) = kind.to_wire();
    i64::from(tag) * 256 + i64::from(arg)
}

/// The first run starts at the end of the journal (no history to miss) and
/// reads the records 100..110 written while it runs.
fn first_run(dir: &Dir) {
    let plan = run(
        dir,
        Journal {
            id: 7,
            first: 0,
            next: 100,
        },
        110,
    );
    assert_eq!(plan, Resume::Fresh { from: 100 });
    assert_eq!(
        saved(dir),
        Some(Checkpoint {
            journal_id: 7,
            next_usn: 110
        }),
        "the position survives the restart"
    );
}

#[test]
fn a_restart_through_the_store_resumes_an_intact_journal_without_a_gap() {
    let dir = Dir::new("intact");
    first_run(&dir);
    // 110..115 were written while nobody listened.
    let plan = run(
        &dir,
        Journal {
            id: 7,
            first: 0,
            next: 115,
        },
        120,
    );
    assert_eq!(plan, Resume::Continue { from: 110 });
    assert_eq!(gap_rows(&dir), Vec::<i64>::new());
    // Every record once: nothing was missed and nothing read again.
    assert_eq!(record_usns(&dir), (100..120).collect::<Vec<i64>>());
}

#[test]
fn a_restart_through_the_store_after_a_changed_journal_id_writes_one_gap() {
    let dir = Dir::new("changed");
    first_run(&dir);
    // The journal was deleted and created again: a new id, new numbers.
    let recreated = Journal {
        id: 8,
        first: 0,
        next: 50,
    };
    let plan = run(&dir, recreated, 60);
    assert_eq!(
        plan,
        Resume::Gap {
            reason: GapReason::JournalIdChanged,
            from: 50
        }
    );
    assert_eq!(
        gap_rows(&dir),
        [wire(Kind::Overflow(GapReason::JournalIdChanged))]
    );
    // The saved position names the new journal now.
    assert_eq!(
        saved(&dir),
        Some(Checkpoint {
            journal_id: 8,
            next_usn: 60
        })
    );
    // The next restart finds the journal intact again: still one gap.
    let plan = run(
        &dir,
        Journal {
            id: 8,
            first: 0,
            next: 60,
        },
        62,
    );
    assert_eq!(plan, Resume::Continue { from: 60 });
    assert_eq!(gap_rows(&dir).len(), 1);
    let usns = record_usns(&dir);
    assert_eq!(
        usns,
        (100..110).chain(50..62).collect::<Vec<i64>>(),
        "no record of either journal twice"
    );
}
