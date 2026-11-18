//! The rate and the latency of the store pipeline alone (#497): batches pushed
//! the way capture pushes them, a real on-disk writer, no file system watch.
//!
//! What the churn generator cannot do on one PC (it reaches 3k events/s) this
//! does: 10k and 20k events/s paced, and a burst as fast as the producer can
//! push. The latency is the age of the first event of a commit at the end of
//! its write, so it is an upper bound of the latency of the other events.
//!
//! Ignored: it takes about 40 s. Run it with
//! `cargo test --release -p peekbuddy-store --test ingest_rate -- --ignored`.
#![expect(
    clippy::unwrap_used,
    clippy::cast_precision_loss,
    reason = "test helpers outside #[test] fns may unwrap; counts and rates far below 2^52"
)]

use std::io::Write as _;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use peekbuddy_core::event::{Event, Flags, Kind, NameKind, NameRef, RootIdx, Source, VolIdx};
use peekbuddy_store::pipeline::{Config, Hooks, Pipeline, Producer};
use peekbuddy_store::writer::{RootInfo, VolInfo, Writer};

/// 100 ns ticks from 1601-01-01 to 1970-01-01.
const UNIX_EPOCH_TICKS: u64 = 116_444_736_000_000_000;

fn now_ticks() -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(0));
    UNIX_EPOCH_TICKS + nanos / 100
}

fn event(ts: u64, id: u64) -> Event {
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
        file_id: u128::from(id),
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

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// What each commit reports: `(rows, microseconds the first event had waited,
/// microseconds the write took)`.
type Commits = Arc<Mutex<Vec<(u64, u64, u64)>>>;

/// A pipeline over a fresh store, and what each commit reports: `(rows,
/// microseconds the first event had waited)`.
fn start(name: &str) -> (Pipeline, Commits, Dir) {
    let dir = Dir(std::env::temp_dir().join(format!("pb-ingest-{name}-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&dir.0);
    std::fs::create_dir_all(&dir.0).unwrap();
    let mut writer = Writer::open(&dir.0.join("segment.db")).unwrap();
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
    let commits: Commits = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&commits);
    let mut config = Config::new(dir.0.join("spill"));
    if let Some(mib) = std::env::var("PEEKBUDDY_WAL_MIB")
        .ok()
        .and_then(|text| text.parse::<u64>().ok())
    {
        config.wal_checkpoint_bytes = mib * 1024 * 1024;
    }
    config.hooks = Hooks {
        on_commit: Some(Arc::new(move |info| {
            seen.lock()
                .unwrap()
                .push((info.commit.rows, info.age_us, info.write_us));
        })),
        ..Hooks::default()
    };
    (Pipeline::start(writer, config).unwrap(), commits, dir)
}

/// Pushes `events` events in batches of `per_batch`, one batch every
/// `per_batch / rate` seconds (no pause for `None`); returns the seconds it took.
fn push(producer: &Producer, events: u64, per_batch: u64, rate: Option<u64>) -> f64 {
    let began = Instant::now();
    let mut sent = 0_u64;
    while sent < events {
        if let Some(rate) = rate {
            let due = Duration::from_secs_f64(sent as f64 / rate as f64);
            while began.elapsed() < due {
                std::hint::spin_loop();
            }
        }
        let mut batch = producer.new_batch();
        let ts = now_ticks();
        for i in 0..per_batch.min(events - sent) {
            let id = sent + i;
            batch
                .push(
                    event(ts, id),
                    &units(&format!("d{}\\f{}.txt", id % 50, id % 20_000)),
                )
                .unwrap();
        }
        sent += per_batch;
        let _ = producer.push(batch);
    }
    began.elapsed().as_secs_f64()
}

/// The `p`-th percentile (0 to 100) in milliseconds of the age (`write` false)
/// or of the write (`write` true), each counted once per row for the age and
/// once per commit for the write.
fn percentile(commits: &[(u64, u64, u64)], p: f64, write: bool) -> f64 {
    let mut sorted: Vec<(u64, u64)> = commits
        .iter()
        .map(|&(rows, age, took)| (if write { took } else { age }, if write { 1 } else { rows }))
        .collect();
    sorted.sort_unstable();
    let total: u64 = sorted.iter().map(|&(_, weight)| weight).sum();
    let mut seen = 0_u64;
    for (value, weight) in sorted {
        seen += weight;
        if seen as f64 >= total as f64 * p / 100.0 {
            return value as f64 / 1_000.0;
        }
    }
    0.0
}

fn report(line: &str) {
    let _ = writeln!(std::io::stderr(), "ingest_rate: {line}");
}

/// Paced at `rate` events/s for `seconds`, in batches of `per_batch`: nothing
/// may be lost, and the commits say how long a row waited.
fn paced(name: &str, rate: u64, seconds: u64, per_batch: u64) {
    let (pipeline, commits, _dir) = start(name);
    let producer = pipeline.producer();
    let events = rate * seconds;
    let took = push(&producer, events, per_batch, Some(rate));
    pipeline.flush(Duration::from_secs(60)).unwrap();
    let stats = pipeline.stats();
    let seen = commits.lock().unwrap().clone();
    report(&format!(
        "{name}: {events} events at {rate}/s in {took:.1} s, {} rows, {} commits, lost {}, ring high water {}; age of the first event at the end of a commit: p50 {:.1} ms, p99 {:.1} ms; write per commit: p50 {:.1} ms, p99 {:.1} ms, max {:.1} ms; {} checkpoints, the longest {:.1} ms",
        stats.rows,
        stats.commits,
        stats.lost_batches,
        stats.ring_high_water,
        percentile(&seen, 50.0, false),
        percentile(&seen, 99.0, false),
        percentile(&seen, 50.0, true),
        percentile(&seen, 99.0, true),
        percentile(&seen, 100.0, true),
        stats.checkpoints,
        stats.checkpoint_max_us as f64 / 1_000.0,
    ));
    assert_eq!(stats.rows, events, "{name}: every event is stored");
    assert_eq!(stats.lost_batches, 0, "{name}: no batch is lost");
}

#[test]
#[ignore = "about 40 s: the rate and latency of the pipeline (#497)"]
fn the_pipeline_keeps_up_at_the_rates_of_the_budget() {
    paced("sustained-10k", 10_000, 10, 8);
    paced("sustained-20k", 20_000, 10, 16);

    // A burst: full batches as fast as the producer can push them.
    let (pipeline, _commits, _dir) = start("burst");
    let producer = pipeline.producer();
    let events = 500_000_u64;
    let began = Instant::now();
    let pushed = push(&producer, events, 256, None);
    pipeline.flush(Duration::from_secs(120)).unwrap();
    let total = began.elapsed().as_secs_f64();
    let stats = pipeline.stats();
    report(&format!(
        "burst: {events} events pushed in {pushed:.2} s ({:.0}/s), all stored after {total:.2} s ({:.0}/s), {} rows, lost {}, spilled batches {}, ring high water {}",
        events as f64 / pushed,
        events as f64 / total,
        stats.rows,
        stats.lost_batches,
        stats.spilled_batches,
        stats.ring_high_water,
    ));
    assert_eq!(stats.rows, events, "burst: every event is stored");
    assert_eq!(stats.lost_batches, 0, "burst: no batch is lost");
}
