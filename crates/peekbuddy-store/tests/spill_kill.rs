//! A process killed in the middle of a spill append (#266, #216): the file
//! keeps a torn tail, the next start finds it, cuts it and replays only the
//! whole records before it, in order. The kill is real (`Child::kill`, which
//! is `TerminateProcess` on Windows); the child is this test binary run in a
//! mode chosen by an environment variable.
#![expect(clippy::unwrap_used, reason = "tests unwrap")]

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use peekbuddy_core::event::{Event, Flags, Kind, NameKind, NameRef, RootIdx, Source, VolIdx};
use peekbuddy_store::batch::Batch;
use peekbuddy_store::pipeline::{Config, Pipeline};
use peekbuddy_store::spill::{FILE_NAME, Spill, crc32, encode_batch};
use peekbuddy_store::writer::{RootInfo, VolInfo, Writer};
use rusqlite::Connection;

/// Set in the child: the spill directory to write to.
const CHILD_DIR: &str = "PB_SPILL_KILL_DIR";
/// Set in the child: `half` writes half a record and parks, `loop` appends
/// until it is killed.
const CHILD_MODE: &str = "PB_SPILL_KILL_MODE";
/// Events in each batch.
const PER_BATCH: u64 = 40;

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

fn batch(first: u64) -> Batch {
    let mut batch = Batch::with_capacity(64, 8192);
    for ts in first..first + PER_BATCH {
        let name: Vec<u16> = format!("d\\f{ts}-{}.txt", "x".repeat(60))
            .encode_utf16()
            .collect();
        batch.push(event(ts), &name).unwrap();
    }
    batch
}

/// The bytes of one record of `batch`: length, CRC, payload.
fn record(batch: &Batch) -> Vec<u8> {
    let mut payload = Vec::new();
    encode_batch(batch, &mut Vec::new(), &mut payload).unwrap();
    let mut out = Vec::new();
    out.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_le_bytes());
    out.extend_from_slice(&crc32(&payload).to_le_bytes());
    out.extend_from_slice(&payload);
    out
}

struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("pb-spillkill-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The child half. Returns only when this process is not the child.
fn run_as_child() {
    let (Some(dir), Some(mode)) = (std::env::var_os(CHILD_DIR), std::env::var_os(CHILD_MODE))
    else {
        return;
    };
    let dir = PathBuf::from(dir);
    let mut spill = Spill::open(&dir, 0, 0).unwrap();
    if mode == "half" {
        for index in 0..5 {
            spill.append(&batch(1 + index * PER_BATCH)).unwrap();
        }
        let torn = record(&batch(1 + 5 * PER_BATCH));
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(dir.join(FILE_NAME))
            .unwrap();
        file.write_all(&torn[..torn.len() / 2]).unwrap();
        file.sync_data().unwrap();
        std::fs::write(dir.join("half"), (torn.len() / 2).to_string()).unwrap();
        loop {
            std::thread::sleep(Duration::from_secs(60));
        }
    }
    let mut first = 1;
    loop {
        spill.append(&batch(first)).unwrap();
        first += PER_BATCH;
    }
}

fn child(dir: &Path, mode: &str, test: &str) -> Child {
    Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test, "--nocapture", "--test-threads=1"])
        .env(CHILD_DIR, dir)
        .env(CHILD_MODE, mode)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

fn replayed(spill: &mut Spill) -> Vec<u64> {
    let mut out = Vec::new();
    let mut into = Batch::with_capacity(64, 8192);
    while spill.next(&mut into).unwrap().is_some() {
        out.extend(into.iter().map(|(event, _)| event.ts));
    }
    out
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
            path: r"\\?\C:\watched".encode_utf16().collect(),
            recursive: true,
        })
        .unwrap();
    writer
}

#[test]
fn a_kill_in_the_middle_of_an_append_leaves_a_tail_that_is_detected_not_replayed() {
    const TEST: &str =
        "a_kill_in_the_middle_of_an_append_leaves_a_tail_that_is_detected_not_replayed";
    run_as_child();
    let dir = Dir::new("half");
    let spill_dir = dir.0.join("spill");
    let mut process = child(&spill_dir, "half", TEST);
    let marker = spill_dir.join("half");
    let until = Instant::now() + Duration::from_secs(60);
    while !marker.exists() {
        assert!(Instant::now() < until, "the child never reached the append");
        std::thread::sleep(Duration::from_millis(20));
    }
    // The marker is written after the half record is on disk, so reading it
    // is safe once the file has content.
    std::thread::sleep(Duration::from_millis(50));
    let half: u64 = std::fs::read_to_string(&marker)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    process.kill().unwrap();
    process.wait().unwrap();

    // The next start: the pipeline over the same spill directory.
    let db = dir.0.join("segment.db");
    let pipeline = Pipeline::start(open_writer(&db), Config::new(spill_dir.clone())).unwrap();
    // The old file is replayed by the writer on its own.
    let until = Instant::now() + Duration::from_secs(30);
    while pipeline.stats().rows < 5 * PER_BATCH {
        assert!(Instant::now() < until, "{:?}", pipeline.stats());
        std::thread::sleep(Duration::from_millis(20));
    }
    pipeline.flush(Duration::from_secs(30)).unwrap();
    let stats = pipeline.stats();
    assert_eq!(stats.torn_tail_bytes, half, "the tail is reported");
    drop(pipeline.close());
    let conn = Connection::open(&db).unwrap();
    let stored: Vec<u64> = conn
        .prepare("SELECT ts FROM event ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get::<_, i64>(0))
        .unwrap()
        .map(|ts| u64::try_from(ts.unwrap()).unwrap())
        .collect();
    // Five whole batches arrived in order; nothing of the torn sixth did.
    assert_eq!(stored, (1..=5 * PER_BATCH).collect::<Vec<u64>>());
}

#[test]
fn kills_at_random_points_never_replay_a_partial_record() {
    const TEST: &str = "kills_at_random_points_never_replay_a_partial_record";
    run_as_child();
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    for round in 0..12 {
        let dir = Dir::new(&format!("loop{round}"));
        let spill_dir = dir.0.join("spill");
        let mut process = child(&spill_dir, "loop", TEST);
        let file = spill_dir.join(FILE_NAME);
        let until = Instant::now() + Duration::from_secs(60);
        while std::fs::metadata(&file).map_or(0, |meta| meta.len()) < 20_000 {
            assert!(Instant::now() < until, "the child wrote nothing");
            std::thread::sleep(Duration::from_millis(10));
        }
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        std::thread::sleep(Duration::from_millis(state % 120));
        process.kill().unwrap();
        process.wait().unwrap();

        let size = std::fs::metadata(&file).unwrap().len();
        let mut spill = Spill::open(&spill_dir, 0, 0).unwrap();
        let torn = spill.torn_bytes();
        let ts = replayed(&mut spill);
        // Whole batches only, in order, from the first.
        assert_eq!(ts.len() as u64 % PER_BATCH, 0, "round {round}");
        assert_eq!(
            ts,
            (1..=ts.len() as u64).collect::<Vec<u64>>(),
            "round {round}"
        );
        // Every byte is a replayed record or the reported tail.
        assert_eq!(
            std::fs::metadata(&file).unwrap().len() + torn,
            size,
            "round {round}"
        );
        // Opening again finds nothing more to cut.
        assert_eq!(Spill::open(&spill_dir, 0, 0).unwrap().torn_bytes(), 0);
    }
}
