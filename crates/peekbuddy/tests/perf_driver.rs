//! The two scenarios of `cargo xtask perf` that need the engine's log and its
//! pipe: **ingest** (the real exe under a churn profile, #25) and **handles**
//! (pause and resume cycles over the pipe, #27, #269). `cargo xtask perf`
//! runs them in release mode with `PEEKBUDDY_PERF_EXE` (the exe to measure),
//! `PEEKBUDDY_PERF_OUT` (the JSON file to write) and `PEEKBUDDY_PERF_PROFILE`
//! (the churn profile); without these they are ignored tests that do nothing.
//!
//! The numbers come from outside the engine, with the clocks of the churn
//! generator: the ledger says when an operation ended, the log says what was
//! durable and when a poller first saw it.
#![cfg(windows)]
#![expect(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns may unwrap"
)]

#[path = "support/engine.rs"]
mod engine;
#[path = "support/judge.rs"]
mod judge;

use std::collections::HashMap;
use std::os::windows::io::AsHandle as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use peekbuddy_churn::exec::windows::{WinClock, WinTarget};
use peekbuddy_churn::exec::{Config, run};
use peekbuddy_churn::ops::RawKind;
use peekbuddy_churn::plan::Shape;
use peekbuddy_churn::profile::{Verdict, profile};
use peekbuddy_core::ipc::{Command as IpcCommand, Message, PROTOCOL_VERSION};
use peekbuddy_core::query::{CancelFlag, Direction, Filter, PageRequest};
use peekbuddy_store::read::Reader;
use peekbuddy_testkit::TestDir;
use peekbuddy_win::job::Job;
use peekbuddy_win::metrics;
use peekbuddy_win::pipe::PipeClient;
use serde_json::json;

use engine::{Kill, quit, quit_within, wait_for_window, wait_until_settled};
use judge::{Run, judge, raw_kind, relative};

/// 100 ns ticks from 1601-01-01 to 1970-01-01.
const UNIX_EPOCH_TICKS: u64 = 116_444_736_000_000_000;

fn now_ticks() -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(0));
    UNIX_EPOCH_TICKS + nanos / 100
}

/// Where the JSON goes; `None` when the harness is not driving this run.
fn out() -> Option<PathBuf> {
    std::env::var_os("PEEKBUDDY_PERF_OUT").map(PathBuf::from)
}

/// The engine in a job object that dies with the test.
struct Engine {
    child: Kill,
    _job: Job,
}

fn start(data: &Path) -> Engine {
    let child = engine::spawn(data);
    let job = Job::new().unwrap();
    job.assign(child.0.as_handle()).unwrap();
    Engine { child, _job: job }
}

fn watched(dir: &TestDir) -> (PathBuf, PathBuf) {
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
    (data, root)
}

/// The `p`-th percentile (0 to 100) of `sorted`, in the unit of the values.
fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::cast_sign_loss,
        reason = "an index into a slice, rounded from a fraction of its length"
    )]
    let at = (((sorted.len() - 1) as f64) * p / 100.0).round() as usize;
    sorted[at.min(sorted.len() - 1)]
}

/// The first time each `(path, kind)` row was seen in the log, in order.
type Sightings = HashMap<(String, RawKind), Vec<u64>>;

/// Polls the log every few milliseconds and notes when each row first shows.
fn poll_log(
    data: PathBuf,
    root: Vec<u16>,
    stop: Arc<AtomicBool>,
) -> std::thread::JoinHandle<Sightings> {
    std::thread::spawn(move || {
        let reader = Reader::new(&data.join("segments"));
        let cancel = CancelFlag::new();
        let mut seen = Sightings::new();
        let mut cursor: Option<i64> = None;
        loop {
            let stopping = stop.load(Ordering::Acquire);
            if data.join("segments").join("catalog.db").exists() {
                let page = reader.page(
                    &PageRequest {
                        generation: 0,
                        filter: Filter::default(),
                        cursor,
                        direction: Direction::Newer,
                        limit: 5_000,
                    },
                    &cancel,
                );
                if let Ok(page) = page {
                    let at = now_ticks();
                    for row in &page.rows {
                        if let (Some(rel), Some(kind)) =
                            (relative(&row.path, &root), raw_kind(row.raw_mask))
                        {
                            seen.entry((String::from_utf16_lossy(&rel).to_lowercase(), kind))
                                .or_default()
                                .push(at);
                        }
                    }
                    if let Some(newest) = page.newest_id() {
                        cursor = Some(newest);
                    }
                    if page.more {
                        continue;
                    }
                }
            }
            if stopping {
                return seen;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    })
}

/// Peak private bytes of the process, sampled until `stop`.
fn sample_memory(
    pid: u32,
    stop: Arc<AtomicBool>,
    peak: Arc<AtomicU64>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let Ok(process) = metrics::open_process(pid) else {
            return;
        };
        while !stop.load(Ordering::Acquire) {
            if let Ok(memory) = metrics::memory(process.as_handle()) {
                peak.fetch_max(memory.private_bytes, Ordering::Relaxed);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    })
}

#[test]
#[ignore = "run by `cargo xtask perf --scenario ingest`"]
fn ingest() {
    let Some(out) = out() else {
        return;
    };
    let name =
        std::env::var("PEEKBUDDY_PERF_PROFILE").unwrap_or_else(|_| String::from("latency-10k"));
    let dir = TestDir::new("perf-ingest").unwrap();
    let (data, root) = watched(&dir);
    let mut engine = start(&data);
    let window = wait_for_window(&data, &mut engine.child.0);
    wait_until_settled(&data, Duration::from_secs(3));

    let root_units: Vec<u16> = root
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .encode_utf16()
        .collect();
    let stop = Arc::new(AtomicBool::new(false));
    let peak = Arc::new(AtomicU64::new(0));
    let sampler = sample_memory(engine.child.0.id(), Arc::clone(&stop), Arc::clone(&peak));
    let poller = poll_log(data.clone(), root_units, Arc::clone(&stop));

    let started = Instant::now();
    let config = Config {
        profile: profile(&name).unwrap(),
        seed: 5,
        threads: 4,
        shape: Shape::default(),
    };
    let outcome = run(&config, |_| (WinTarget::new(&root), WinClock::new()));
    let churn_seconds = started.elapsed().as_secs_f64();
    wait_until_settled(&data, Duration::from_secs(5));
    stop.store(true, Ordering::Release);
    let sightings = poller.join().unwrap();
    sampler.join().unwrap();
    let closing = Instant::now();
    quit_within(window, &mut engine.child.0, Duration::from_mins(15));
    let shutdown_seconds = closing.elapsed().as_secs_f64();

    let verdict = judge(
        &Run {
            data: data.clone(),
            root: root.clone(),
        },
        &outcome,
    );

    // Change to durable: from the end of an operation to the first sighting of
    // its row in the log at or after the start of the operation.
    let mut latencies: Vec<f64> = Vec::new();
    for record in outcome.records.iter().filter(|record| record.result == 0) {
        for expected in record.expected_records().unwrap_or_default() {
            let key = (
                String::from_utf16_lossy(&expected.path).to_lowercase(),
                expected.kind,
            );
            if let Some(times) = sightings.get(&key)
                && let Some(at) = times.iter().find(|at| **at >= record.t_start)
            {
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "a latency in ticks, far below 2^52"
                )]
                latencies.push(at.saturating_sub(record.t_end) as f64 / 10_000.0);
            }
        }
    }
    latencies.sort_by(f64::total_cmp);

    #[expect(
        clippy::cast_precision_loss,
        reason = "counts and bytes in a report, far below 2^52"
    )]
    let document = json!({
        "scenario": "ingest",
        "profile": name,
        "exercised": outcome.verdict == Verdict::Exercised,
        "ledgerEvents": outcome.events,
        "achievedPerSecond": outcome.achieved_per_s,
        "churnSeconds": churn_seconds,
        "shutdownSeconds": shutdown_seconds,
        "undetected": verdict.report.undetected.len(),
        "gapRows": verdict.gaps,
        "unfinishedGaps": verdict.unfinished_gaps,
        "peakPrivateMb": peak.load(Ordering::Relaxed) as f64 / 1_048_576.0,
        "latencyP50Ms": percentile(&latencies, 50.0),
        "latencyP99Ms": percentile(&latencies, 99.0),
        "latencySamples": latencies.len(),
        "summary": verdict.summary.lines().next().unwrap_or_default(),
    });
    std::fs::write(out, serde_json::to_string_pretty(&document).unwrap()).unwrap();
}

#[test]
#[ignore = "run by `cargo xtask perf --scenario handles`"]
fn handles() {
    let Some(out) = out() else {
        return;
    };
    let dir = TestDir::new("perf-handles").unwrap();
    let (data, root) = watched(&dir);
    let mut engine = start(&data);
    let window = wait_for_window(&data, &mut engine.child.0);
    wait_until_settled(&data, Duration::from_secs(3));
    let process = metrics::open_process(engine.child.0.id()).unwrap();
    let pipe = peekbuddy_engine::ipc_server::pipe_name(Some(&data)).unwrap();

    let mut after: Vec<u32> = Vec::new();
    for cycle in 0..3 {
        {
            let mut client =
                PipeClient::connect(&pipe, Some(&engine::exe()), Duration::from_secs(10)).unwrap();
            client
                .send(&Message::Hello {
                    protocol_version: PROTOCOL_VERSION,
                    build: String::from("perf"),
                })
                .unwrap();
            let _ = client.receive().unwrap();
            for command in [IpcCommand::PauseAll, IpcCommand::ResumeAll] {
                client.send(&Message::Command(command)).unwrap();
                let _ = client.receive().unwrap();
            }
        }
        // Ingest between the cycles, so a watch that was re-armed is used.
        for index in 0..20 {
            std::fs::write(root.join(format!("cycle{cycle}-{index}.txt")), b"x").unwrap();
        }
        wait_until_settled(&data, Duration::from_secs(3));
        after.push(metrics::handle_count(process.as_handle()).unwrap());
    }
    quit(window, &mut engine.child.0);

    let (first, last) = (after[0], after[2]);
    let document = json!({
        "scenario": "handles",
        "handlesAfterCycle": after,
        "delta": i64::from(last) - i64::from(first),
    });
    std::fs::write(out, serde_json::to_string_pretty(&document).unwrap()).unwrap();
}
