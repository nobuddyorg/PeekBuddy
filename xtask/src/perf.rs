//! `cargo xtask perf`: end-to-end measurements of the real exe (#27).
//!
//! Every launch runs inside a job object that kills the whole process tree
//! when the job closes, in an isolated data directory. Numbers come from the
//! exe's own `--perf-probe` document (#24, #161), so the harness never sees
//! a different clock than the engine. Scenarios that need parts of the engine
//! that do not exist yet are reported as "not exercised" with the issue that
//! delivers them, never silently skipped.
//!
//! Gating follows F11: deterministic rows are strict; timing rows are
//! compared base against head in the same job (`--ab <base-exe>`: warn at
//! +15 %, fail at +50 %); resource rows are reported.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::budgets::{self, Budget};

/// `schemaVersion` of `perf-results.json`.
const SCHEMA_VERSION: u32 = 1;
/// Warn when head is this much slower than base.
const AB_WARN: f64 = 0.15;
/// Fail when head is this much slower than base.
const AB_FAIL: f64 = 0.50;
/// The CPU time of a process is counted in clock ticks of 15.6 ms, so two
/// runs of one exe differ by whole ticks: 31 ms against 47 ms is +50 % of
/// nothing. A difference below two ticks is not a change.
const CPU_RESOLUTION_MS: f64 = 31.25;

const USAGE: &str = "usage: cargo xtask perf [--scenario startup|idle|memory|ingest|handles|viewer-memory|pr|all] \
[--exe path] [--runs n] [--warmup n] [--idle-secs n] [--profile name]... [--ab <base-exe>] \
[--expect-fail <row id>] [--out file]";

/// The churn profiles of the ingest scenario, in the order they run.
const INGEST_PROFILES: [&str; 3] = ["burst-100k", "sustained-20k", "latency-10k"];

/// What to measure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scenario {
    Startup,
    Idle,
    Memory,
    /// The real exe under the churn profiles (#25, #269).
    Ingest,
    /// Pause and resume cycles over the pipe (#269).
    Handles,
    /// The viewer with a 1M-row and a 10M-row log (#347): minutes to build
    /// the stores, so only the nightly asks for it, never `all`.
    ViewerMemory,
    /// What every pull request runs: startup and idle, in minutes (#27).
    Pr,
    All,
}

impl Scenario {
    fn includes_startup(self) -> bool {
        matches!(self, Self::Startup | Self::Pr | Self::All)
    }

    fn includes_idle(self) -> bool {
        matches!(self, Self::Idle | Self::Memory | Self::Pr | Self::All)
    }

    fn includes_ingest(self) -> bool {
        matches!(self, Self::Ingest | Self::All)
    }

    fn includes_handles(self) -> bool {
        matches!(self, Self::Handles | Self::All)
    }

    fn includes_viewer_memory(self) -> bool {
        matches!(self, Self::ViewerMemory)
    }
}

/// Parsed command line.
#[derive(Debug, Clone, PartialEq)]
struct Options {
    scenario: Scenario,
    exe: PathBuf,
    runs: usize,
    warmup: usize,
    idle_secs: u64,
    ab_base: Option<PathBuf>,
    /// Ingest profiles; the three named ones when empty.
    profiles: Vec<String>,
    /// The row that must fail: the self-check of the harness (#269) runs a
    /// build with a deliberate regression and passes only when this row fails.
    expect_fail: Option<String>,
    out: PathBuf,
}

fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        scenario: Scenario::All,
        exe: PathBuf::from("target/release/peekbuddy.exe"),
        runs: 10,
        warmup: 3,
        idle_secs: 60,
        ab_base: None,
        profiles: Vec::new(),
        expect_fail: None,
        out: PathBuf::from("target/perf/perf-results.json"),
    };
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        let mut value = |name: &str| {
            args.next()
                .cloned()
                .ok_or_else(|| format!("{name} needs a value\n{USAGE}"))
        };
        match flag.as_str() {
            "--scenario" => {
                options.scenario = match value("--scenario")?.as_str() {
                    "startup" => Scenario::Startup,
                    "idle" => Scenario::Idle,
                    "memory" => Scenario::Memory,
                    "ingest" => Scenario::Ingest,
                    "handles" => Scenario::Handles,
                    "viewer-memory" => Scenario::ViewerMemory,
                    "pr" => Scenario::Pr,
                    "all" => Scenario::All,
                    other => return Err(format!("unknown scenario {other}\n{USAGE}")),
                };
            }
            "--exe" => options.exe = PathBuf::from(value("--exe")?),
            "--runs" => options.runs = number(&value("--runs")?)?,
            "--warmup" => options.warmup = number(&value("--warmup")?)?,
            "--idle-secs" => options.idle_secs = number(&value("--idle-secs")?)?,
            "--ab" => options.ab_base = Some(PathBuf::from(value("--ab")?)),
            "--profile" => options.profiles.push(value("--profile")?),
            "--expect-fail" => options.expect_fail = Some(value("--expect-fail")?),
            "--out" => options.out = PathBuf::from(value("--out")?),
            other => return Err(format!("unknown option {other}\n{USAGE}")),
        }
    }
    if options.runs == 0 {
        return Err(format!("--runs must be at least 1\n{USAGE}"));
    }
    Ok(options)
}

fn number<T: std::str::FromStr>(text: &str) -> Result<T, String> {
    text.parse()
        .map_err(|_| format!("{text} is not a number\n{USAGE}"))
}

/// The numbers of one startup run, from the probe at "engine ready".
#[derive(Debug, Clone, Copy, PartialEq)]
struct Startup {
    ready_ms: f64,
    cpu_ms: f64,
    disk_read_mb: f64,
}

/// The numbers of one idle run, from the probe's two samples.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Idle {
    seconds: f64,
    cpu_percent: f64,
    wakes_per_min: Option<f64>,
    private_mb: f64,
    working_set_mb: f64,
    handles: f64,
}

fn field(value: &Value, path: &[&str]) -> Result<f64, String> {
    let mut current = value;
    for key in path {
        current = current
            .get(*key)
            .ok_or_else(|| format!("probe document has no {}", path.join(".")))?;
    }
    current
        .as_f64()
        .ok_or_else(|| format!("probe field {} is not a number", path.join(".")))
}

fn ready_us(probe: &Value) -> Result<f64, String> {
    probe
        .get("milestones")
        .and_then(Value::as_array)
        .and_then(|rows| rows.iter().find(|row| row["name"] == "EngineReady"))
        .and_then(|row| row["firstUs"].as_f64())
        .ok_or_else(|| String::from("probe document has no EngineReady milestone"))
}

fn startup_from(probe: &Value) -> Result<Startup, String> {
    let start = ["samples", "start"];
    let ticks = field(probe, &[start[0], start[1], "kernelTicks"])?
        + field(probe, &[start[0], start[1], "userTicks"])?;
    Ok(Startup {
        ready_ms: ready_us(probe)? / 1_000.0,
        cpu_ms: ticks / 10_000.0,
        disk_read_mb: field(probe, &[start[0], start[1], "readBytes"])? / 1_000_000.0,
    })
}

/// What an outside observer saw while the engine idled: seconds observed, CPU
/// ticks (100 ns) used and context switches, when the OS reports them.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Observed {
    seconds: f64,
    cpu_ticks: f64,
    switches: Option<f64>,
}

/// Idle numbers: CPU and wakeups from the outside observation (the probe
/// reading its own counters would add its own wakeups), memory and handles
/// from the probe's closing sample.
fn idle_from(probe: &Value, seen: Observed) -> Result<Idle, String> {
    let sample = |name: &str| field(probe, &["samples", "end", name]);
    let seconds = seen.seconds;
    Ok(Idle {
        seconds,
        // 100 ns ticks over wall-clock seconds.
        cpu_percent: if seconds > 0.0 {
            seen.cpu_ticks / 10_000_000.0 / seconds * 100.0
        } else {
            0.0
        },
        wakes_per_min: seen
            .switches
            .filter(|_| seconds > 0.0)
            .map(|switches| switches / (seconds / 60.0)),
        private_mb: sample("privateBytes")? / 1_048_576.0,
        working_set_mb: sample("workingSet")? / 1_048_576.0,
        handles: sample("handles")?,
    })
}

/// Minimum, median and maximum of a series.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Stats {
    min: f64,
    median: f64,
    max: f64,
}

fn stats(values: &[f64]) -> Option<Stats> {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let middle = sorted.len() / 2;
    let median = if sorted.len() % 2 == 1 {
        *sorted.get(middle)?
    } else {
        (sorted.get(middle.checked_sub(1)?)? + sorted.get(middle)?) / 2.0
    };
    Some(Stats {
        min: *sorted.first()?,
        median,
        max: *sorted.last()?,
    })
}

/// How a row came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Pass,
    Warn,
    Fail,
    /// Reported only: the row is not gated by this scenario.
    Report,
    /// The scenario cannot run yet; the reason is in the row.
    NotExercised,
}

impl Verdict {
    const fn name(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Warn => "warn",
            Self::Fail => "fail",
            Self::Report => "report",
            Self::NotExercised => "not exercised",
        }
    }
}

/// One line of the result table.
#[derive(Debug, Clone, PartialEq)]
struct Row {
    id: &'static str,
    value: Option<f64>,
    unit: String,
    limit: Option<f64>,
    verdict: Verdict,
    note: String,
}

fn budget<'a>(budgets: &'a [Budget], id: &str) -> Option<&'a Budget> {
    budgets.iter().find(|budget| budget.id == id)
}

/// Judges `value` against the budget row `id`. Deterministic rows are strict;
/// every other gate only reports an absolute number, because wall-clock
/// limits on shared runners are meaningless (F11): they are decided by A/B.
fn judge(budgets: &[Budget], id: &'static str, value: f64, note: &str) -> Row {
    let Some(row) = budget(budgets, id) else {
        return Row {
            id,
            value: Some(value),
            unit: String::new(),
            limit: None,
            verdict: Verdict::Fail,
            note: format!("no perf/budgets.toml row for {id}"),
        };
    };
    let within = row.limit.is_none_or(|limit| match row.bound.as_str() {
        "min" => value >= limit,
        _ => value <= limit,
    });
    let strict = row.gate == "pr-strict" || row.gate == "nightly-strict";
    let verdict = match (strict, within) {
        (true, true) => Verdict::Pass,
        (true, false) => Verdict::Fail,
        (false, _) => Verdict::Report,
    };
    let mut note = note.to_owned();
    if !strict && !within {
        let _ = write!(
            note,
            " (above the provisional limit; timing rows are gated by A/B)"
        );
    }
    Row {
        id,
        value: Some(value),
        unit: row.unit.clone(),
        limit: row.limit,
        verdict,
        note,
    }
}

fn not_exercised(id: &'static str, why: &str) -> Row {
    Row {
        id,
        value: None,
        unit: String::new(),
        limit: None,
        verdict: Verdict::NotExercised,
        note: why.to_owned(),
    }
}

/// Compares head against base medians: warn at +15 %, fail at +50 %.
fn ab_verdict(base: f64, head: f64) -> Verdict {
    if base <= 0.0 {
        return Verdict::Report;
    }
    let change = head / base - 1.0;
    if change >= AB_FAIL {
        Verdict::Fail
    } else if change >= AB_WARN {
        Verdict::Warn
    } else {
        Verdict::Pass
    }
}

/// [`ab_verdict`] for a measure that comes in steps of `resolution`: a
/// difference smaller than the resolution passes whatever its ratio.
fn ab_verdict_stepped(base: f64, head: f64, resolution: f64) -> Verdict {
    if (head - base).abs() < resolution {
        Verdict::Pass
    } else {
        ab_verdict(base, head)
    }
}

fn markdown(rows: &[Row]) -> String {
    let mut out = String::from(
        "| Budget row | Value | Limit | Verdict | Note |\n| --- | --- | --- | --- | --- |\n",
    );
    for row in rows {
        let value = row.value.map_or_else(
            || String::from("-"),
            |value| format!("{value:.2} {}", row.unit),
        );
        let limit = row.limit.map_or_else(
            || String::from("-"),
            |limit| format!("{limit} {}", row.unit),
        );
        let _ = writeln!(
            out,
            "| `{}` | {value} | {limit} | {} | {} |",
            row.id,
            row.verdict.name(),
            row.note
        );
    }
    out
}

fn rows_json(rows: &[Row]) -> Value {
    Value::Array(
        rows.iter()
            .map(|row| {
                serde_json::json!({
                    "id": row.id,
                    "value": row.value,
                    "unit": row.unit,
                    "limit": row.limit,
                    "verdict": row.verdict.name(),
                    "note": row.note,
                })
            })
            .collect(),
    )
}

/// What an outside observer saw of the engine while it idled.
#[derive(Debug, Clone, Copy)]
struct External {
    /// Seconds between the two observations.
    seconds: f64,
    /// CPU time (user + kernel) used in between, 100 ns ticks.
    cpu_ticks: u64,
    /// Context switches of all its threads in between, when reported.
    switches: Option<u64>,
}

/// Launching needs job objects and `NtQuerySystemInformation`: Windows only.
#[cfg(not(windows))]
mod launch {
    use serde_json::Value;

    use super::External;

    pub fn probe(_exe: &std::path::Path, _seconds: u64, _tag: &str) -> Result<Value, String> {
        Err(String::from("cargo xtask perf runs on Windows only"))
    }

    pub fn idle(_exe: &std::path::Path, _seconds: u64) -> Result<(Value, External), String> {
        Err(String::from("cargo xtask perf runs on Windows only"))
    }

    pub fn driver(_exe: &std::path::Path, _test: &str, _profile: &str) -> Result<Value, String> {
        Err(String::from("cargo xtask perf runs on Windows only"))
    }
}

#[cfg(windows)]
mod launch {
    use std::os::windows::io::AsHandle as _;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use peekbuddy_win::job::Job;
    use peekbuddy_win::metrics::{self, ContextSwitches};
    use serde_json::Value;

    use super::External;

    fn scratch(tag: &str) -> Result<PathBuf, String> {
        let dir = std::env::temp_dir().join(format!("pbperf-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(|error| format!("{}: {error}", dir.display()))?;
        Ok(dir)
    }

    /// Runs `exe --perf-probe seconds` in a job with its own data directory
    /// and returns the probe document. Nothing survives this function: the
    /// job kills the tree and the directory is removed.
    pub fn probe(exe: &Path, seconds: u64, tag: &str) -> Result<Value, String> {
        let dir = scratch(tag)?;
        let out = dir.join("probe.json");
        let result = launch(exe, seconds, &dir, &out, &mut |_| Ok(()));
        let _ = std::fs::remove_dir_all(&dir);
        result
    }

    /// Like [`probe`], and samples the engine from this process after a
    /// settle period and again shortly before its exit timer fires, so the
    /// probe's own sampling and the teardown are not in the numbers.
    pub fn idle(exe: &Path, seconds: u64) -> Result<(Value, External), String> {
        let dir = scratch("idle")?;
        let out = dir.join("probe.json");
        let mut seen = None;
        let result = launch(exe, seconds, &dir, &out, &mut |pid| {
            let settle = Duration::from_secs((seconds / 4).min(3));
            let tail = Duration::from_secs((seconds / 4).min(2));
            let process = metrics::open_process(pid).map_err(|error| error.to_string())?;
            let mut switches = ContextSwitches::new();
            std::thread::sleep(settle);
            let first = (
                Instant::now(),
                metrics::cpu_times(process.as_handle()).map_err(|error| error.to_string())?,
                switches.sample(pid),
            );
            std::thread::sleep(Duration::from_secs(seconds).saturating_sub(settle + tail));
            let last = (
                Instant::now(),
                metrics::cpu_times(process.as_handle()).map_err(|error| error.to_string())?,
                switches.sample(pid),
            );
            seen = Some(External {
                seconds: last.0.duration_since(first.0).as_secs_f64(),
                cpu_ticks: (last.1.user + last.1.kernel)
                    .saturating_sub(first.1.user + first.1.kernel),
                switches: first.2.zip(last.2).map(|(a, b)| b.saturating_sub(a)),
            });
            Ok(())
        });
        let _ = std::fs::remove_dir_all(&dir);
        let document = result?;
        let external = seen.ok_or_else(|| String::from("the engine was never observed"))?;
        Ok((document, external))
    }

    /// Runs one scenario of the driver test (`crates/peekbuddy/tests/perf_driver.rs`)
    /// against `exe` and returns the JSON it wrote. The driver runs in release
    /// mode, like the exe; it starts the engine itself, in its own job.
    pub fn driver(exe: &Path, test: &str, profile: &str) -> Result<Value, String> {
        let dir = scratch(test)?;
        let out = dir.join("driver.json");
        let absolute =
            std::fs::canonicalize(exe).map_err(|error| format!("{}: {error}", exe.display()))?;
        let output = Command::new("cargo")
            .args([
                "test",
                "--release",
                "--locked",
                "-p",
                "peekbuddy",
                "--test",
                "perf_driver",
                "--",
                "--ignored",
                "--exact",
                test,
            ])
            .env("PEEKBUDDY_PERF_EXE", &absolute)
            .env("PEEKBUDDY_PERF_OUT", &out)
            .env("PEEKBUDDY_PERF_PROFILE", profile)
            .stdin(Stdio::null())
            .output()
            .map_err(|error| format!("cargo test: {error}"))?;
        let result = if output.status.success() {
            std::fs::read_to_string(&out)
                .map_err(|error| format!("driver output {}: {error}", out.display()))
                .and_then(|text| {
                    serde_json::from_str(&text).map_err(|error| format!("driver output: {error}"))
                })
        } else {
            let shown = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stderr),
                String::from_utf8_lossy(&output.stdout)
            );
            let tail: String = shown
                .chars()
                .rev()
                .take(3000)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            Err(format!(
                "the {test} driver failed ({}):\n{tail}",
                output.status
            ))
        };
        let _ = std::fs::remove_dir_all(&dir);
        result
    }

    fn launch(
        exe: &Path,
        seconds: u64,
        dir: &Path,
        out: &Path,
        observe: &mut dyn FnMut(u32) -> Result<(), String>,
    ) -> Result<Value, String> {
        let job = Job::new().map_err(|error| format!("job object: {error}"))?;
        let mut child = Command::new(exe)
            .args(["--perf-probe", &seconds.to_string(), "--perf-probe-out"])
            .arg(out)
            .arg("--data-dir")
            .arg(dir.join("data"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("{}: {error}", exe.display()))?;
        job.assign(child.as_handle())
            .map_err(|error| format!("assign to job: {error}"))?;
        observe(child.id())?;
        let deadline = Instant::now() + Duration::from_secs(seconds + 60);
        let status = loop {
            if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
                break status;
            }
            if Instant::now() > deadline {
                return Err(format!("{} did not exit in time", exe.display()));
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        if !status.success() {
            return Err(format!("{} exited with {status}", exe.display()));
        }
        let text = std::fs::read_to_string(out)
            .map_err(|error| format!("probe output {}: {error}", out.display()))?;
        serde_json::from_str(&text).map_err(|error| format!("probe output: {error}"))
    }
}

#[cfg(windows)]
fn runner_info() -> Value {
    use peekbuddy_win::osinfo;

    serde_json::json!({
        "label": std::env::var("PEEKBUDDY_RUNNER_LABEL")
            .or_else(|_| std::env::var("RUNNER_NAME"))
            .unwrap_or_else(|_| String::from("local")),
        "image": std::env::var("ImageVersion").ok(),
        "os": osinfo::os_build(),
        "ubr": osinfo::update_build_revision(),
        "vcpu": std::thread::available_parallelism().map(usize::from).ok(),
        "arch": if cfg!(target_arch = "aarch64") { "aarch64" } else { "x86_64" },
        "defenderRealtime": osinfo::defender_realtime_enabled(),
    })
}

/// Head runs, and base runs when there is a base exe. The two alternate, so
/// that a runner that is slow for a while (a cold file cache, a neighbour)
/// slows both the same way: with all head runs first, the same exe compared
/// with itself differed by 60 % on a hosted runner.
fn startup_runs(
    options: &Options,
    base: Option<&Path>,
) -> Result<(Vec<Startup>, Vec<Startup>), String> {
    let (mut head, mut base_runs) = (Vec::new(), Vec::new());
    for index in 0..options.warmup + options.runs {
        let probe = launch::probe(&options.exe, 0, &format!("h{index}"))?;
        let base_probe = base
            .map(|exe| launch::probe(exe, 0, &format!("b{index}")))
            .transpose()?;
        if index >= options.warmup {
            head.push(startup_from(&probe)?);
            if let Some(base_probe) = base_probe {
                base_runs.push(startup_from(&base_probe)?);
            }
        }
    }
    Ok((head, base_runs))
}

type Detail = serde_json::Map<String, Value>;

/// Converts tick and byte counts to `f64` for the report; a count above 2^52
/// loses its last digits, which is irrelevant for 100 ns ticks and bytes.
#[expect(
    clippy::cast_precision_loss,
    reason = "report numbers, not exact counts"
)]
const fn as_f64(count: u64) -> f64 {
    count as f64
}

fn measure_startup(
    options: &Options,
    budgets: &[Budget],
    rows: &mut Vec<Row>,
    detail: &mut Detail,
) -> Result<(), String> {
    let (head, base) = startup_runs(options, options.ab_base.as_deref())?;
    let ready = stats(&head.iter().map(|run| run.ready_ms).collect::<Vec<_>>());
    let cpu = stats(&head.iter().map(|run| run.cpu_ms).collect::<Vec<_>>());
    let disk = stats(&head.iter().map(|run| run.disk_read_mb).collect::<Vec<_>>());
    let note = format!(
        "median of {} warm runs; skeleton: ready = window up (#60, #45)",
        head.len()
    );
    let mut ready_row = judge(
        budgets,
        "engine.ready.warm",
        ready.map_or(0.0, |s| s.median),
        &note,
    );
    let mut cpu_row = judge(
        budgets,
        "engine.startup.cpu",
        cpu.map_or(0.0, |s| s.median),
        &note,
    );
    if options.ab_base.is_some() {
        let base_ready = stats(&base.iter().map(|run| run.ready_ms).collect::<Vec<_>>());
        let base_cpu = stats(&base.iter().map(|run| run.cpu_ms).collect::<Vec<_>>());
        if let (Some(b), Some(h)) = (base_ready, ready) {
            ready_row.verdict = ab_verdict(b.median, h.median);
            let _ = write!(ready_row.note, "; A/B base {:.2} ms", b.median);
        }
        if let (Some(b), Some(h)) = (base_cpu, cpu) {
            cpu_row.verdict = ab_verdict_stepped(b.median, h.median, CPU_RESOLUTION_MS);
            let _ = write!(cpu_row.note, "; A/B base {:.2} ms", b.median);
        }
    }
    rows.push(ready_row);
    rows.push(cpu_row);
    rows.push(judge(
        budgets,
        "engine.startup.disk_read",
        disk.map_or(0.0, |s| s.median),
        &note,
    ));
    rows.push(not_exercised(
        "engine.ready.cold",
        "needs a reboot or standby-list flush; owner-PC run (#84)",
    ));
    detail.insert(
        String::from("startup"),
        serde_json::json!({
            "runs": head.len(),
            "readyMs": ready.map(|s| [s.min, s.median, s.max]),
            "cpuMs": cpu.map(|s| [s.min, s.median, s.max]),
            "diskReadMb": disk.map(|s| [s.min, s.median, s.max]),
        }),
    );
    Ok(())
}

fn measure_idle(
    options: &Options,
    budgets: &[Budget],
    rows: &mut Vec<Row>,
    detail: &mut Detail,
) -> Result<(), String> {
    let (probe, external) = launch::idle(&options.exe, options.idle_secs)?;
    let idle = idle_from(
        &probe,
        Observed {
            seconds: external.seconds,
            cpu_ticks: as_f64(external.cpu_ticks),
            switches: external.switches.map(as_f64),
        },
    )?;
    let note = format!("{:.0} s idle, no roots yet (#45)", idle.seconds);
    rows.push(judge(budgets, "engine.idle.cpu", idle.cpu_percent, &note));
    match idle.wakes_per_min {
        Some(wakes) => rows.push(judge(budgets, "engine.idle.wakes", wakes, &note)),
        None => rows.push(not_exercised(
            "engine.idle.wakes",
            "the OS did not report context switches",
        )),
    }
    rows.push(not_exercised(
        "engine.idle.periodic_timers",
        "no timer registry in the probe yet; the window offers no periodic timer API",
    ));
    rows.push(judge(
        budgets,
        "engine.private_bytes.idle",
        idle.private_mb,
        &note,
    ));
    detail.insert(
        String::from("idle"),
        serde_json::json!({
            "seconds": idle.seconds,
            "cpuPercent": idle.cpu_percent,
            "wakesPerMin": idle.wakes_per_min,
            "privateMb": idle.private_mb,
            "workingSetMb": idle.working_set_mb,
            "handles": idle.handles,
        }),
    );
    Ok(())
}

/// A number of a driver document.
fn number_of(doc: &Value, key: &str) -> Result<f64, String> {
    doc.get(key)
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("the driver document has no number `{key}`"))
}

/// The rows one ingest document gives, and the undetected losses it counts
/// (an unfinished gap counts as one: a gap that never reconciled covers
/// nothing).
fn ingest_rows(budgets: &[Budget], doc: &Value, rows: &mut Vec<Row>) -> Result<f64, String> {
    let profile = doc
        .get("profile")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let exercised = doc
        .get("exercised")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let note = format!(
        "{profile}: {} ledger events in {:.1} s",
        number_of(doc, "ledgerEvents")?,
        number_of(doc, "churnSeconds")?
    );
    let rate = number_of(doc, "achievedPerSecond")?;
    match profile {
        "burst-100k" => {
            if exercised {
                rows.push(judge(budgets, "ingest.burst", rate, &note));
            } else {
                rows.push(not_exercised(
                    "ingest.burst",
                    &format!("the generator reached {rate} events/s, below the target ({note})"),
                ));
            }
            rows.push(judge(
                budgets,
                "engine.private_bytes.burst",
                number_of(doc, "peakPrivateMb")?,
                &note,
            ));
        }
        "sustained-20k" => {
            if exercised {
                rows.push(judge(budgets, "ingest.sustained", rate, &note));
            } else {
                rows.push(not_exercised(
                    "ingest.sustained",
                    &format!("the generator reached {rate} events/s, below the target ({note})"),
                ));
            }
        }
        "latency-10k" => {
            let samples = number_of(doc, "latencySamples")?;
            let note = format!("{note}; {samples} rows seen by a 5 ms poller of the log");
            rows.push(judge(
                budgets,
                "event.latency.p50",
                number_of(doc, "latencyP50Ms")?,
                &note,
            ));
            rows.push(judge(
                budgets,
                "event.latency.p99",
                number_of(doc, "latencyP99Ms")?,
                &note,
            ));
        }
        // The small profile of the harness self-check: no budget, only the
        // loss count below.
        "edge-mix" => {}
        other => return Err(format!("no rows are defined for the profile {other}")),
    }
    Ok(number_of(doc, "undetected")? + number_of(doc, "unfinishedGaps")?)
}

fn measure_ingest(
    options: &Options,
    budgets: &[Budget],
    rows: &mut Vec<Row>,
    detail: &mut Detail,
) -> Result<(), String> {
    let profiles: Vec<String> = if options.profiles.is_empty() {
        INGEST_PROFILES
            .iter()
            .map(|name| (*name).to_owned())
            .collect()
    } else {
        options.profiles.clone()
    };
    let mut lost = 0.0;
    let mut documents = Vec::new();
    for profile in &profiles {
        let doc = launch::driver(&options.exe, "ingest", profile)?;
        lost += ingest_rows(budgets, &doc, rows)?;
        documents.push(doc);
    }
    rows.push(judge(
        budgets,
        "ingest.undetected_loss",
        lost,
        &format!(
            "{} profiles, every operation logged or covered by a reconciled gap",
            profiles.len()
        ),
    ));
    rows.push(not_exercised(
        "capture.rearm_latency",
        "the span of the capture thread is not visible from outside the engine (#21)",
    ));
    detail.insert(String::from("ingest"), Value::Array(documents));
    Ok(())
}

fn measure_handles(
    options: &Options,
    budgets: &[Budget],
    rows: &mut Vec<Row>,
    detail: &mut Detail,
) -> Result<(), String> {
    let doc = launch::driver(&options.exe, "handles", "")?;
    let delta = number_of(&doc, "delta")?;
    let after = doc
        .get("handlesAfterCycle")
        .map_or_else(String::new, Value::to_string);
    rows.push(judge(
        budgets,
        "engine.handles.stable",
        delta,
        &format!("pause and resume over the pipe, three cycles; handles after each: {after}"),
    ));
    detail.insert(String::from("handles"), doc);
    Ok(())
}

/// The log sizes of the private bytes run: the growth between them is the
/// budget.
const MEMORY_ROWS: [u64; 2] = [1_000_000, 10_000_000];

/// Runs the `viewer_memory` bench (`benches/benches/viewer_memory.rs`) with
/// a store of `rows` rows under `dir` and returns the JSON it wrote.
fn viewer_memory_run(rows: u64, dir: &Path) -> Result<Value, String> {
    let out = dir.join(format!("viewer-memory-{rows}.json"));
    let output = std::process::Command::new("cargo")
        .args([
            "bench",
            "--locked",
            "-p",
            "benches",
            "--bench",
            "viewer_memory",
        ])
        .env("PEEKBUDDY_QUERY_ROWS", rows.to_string())
        .env("PEEKBUDDY_QUERY_DIR", dir.join(format!("store-{rows}")))
        .env("PEEKBUDDY_PERF_OUT", &out)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|error| format!("cargo bench: {error}"))?;
    if !output.status.success() {
        let shown = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
        let tail: String = shown
            .chars()
            .rev()
            .take(3000)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        return Err(format!(
            "the viewer_memory run with {rows} rows failed ({}):\n{tail}",
            output.status
        ));
    }
    let text = std::fs::read_to_string(&out)
        .map_err(|error| format!("viewer_memory output {}: {error}", out.display()))?;
    serde_json::from_str(&text).map_err(|error| format!("viewer_memory output: {error}"))
}

/// The row of the growth of the viewer's private bytes from the small to
/// the large log. A bound that holds by construction (the screen and the
/// caches have fixed limits), so it fails the nightly outright.
fn viewer_memory_row(budgets: &[Budget], small: &Value, large: &Value) -> Result<Row, String> {
    let after = |doc: &Value| number_of(doc, "privateBytesAfter");
    let growth = (after(large)? - after(small)?) / 1_048_576.0;
    let note = format!(
        "{:.1} MB with {:.0} rows, {:.1} MB with {:.0} rows; {:.0} and {:.0} rows held",
        after(small)? / 1_048_576.0,
        number_of(small, "rows")?,
        after(large)? / 1_048_576.0,
        number_of(large, "rows")?,
        number_of(small, "heldRows")?,
        number_of(large, "heldRows")?,
    );
    Ok(judge(budgets, "viewer.private_bytes.growth", growth, &note))
}

fn measure_viewer_memory(
    budgets: &[Budget],
    rows: &mut Vec<Row>,
    detail: &mut Detail,
) -> Result<(), String> {
    let dir = std::env::temp_dir().join(format!("pbperf-{}-viewer-memory", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    let result = (|| {
        let [small, large] = MEMORY_ROWS;
        let small = viewer_memory_run(small, &dir)?;
        let large = viewer_memory_run(large, &dir)?;
        let row = viewer_memory_row(budgets, &small, &large)?;
        Ok::<_, String>((row, serde_json::json!({ "small": small, "large": large })))
    })();
    let _ = std::fs::remove_dir_all(&dir);
    let (row, doc) = result?;
    rows.push(row);
    detail.insert(String::from("viewerMemory"), doc);
    Ok(())
}

fn measure(options: &Options) -> Result<(Vec<Row>, Value), String> {
    let budgets = budgets::load()?;
    let mut rows = Vec::new();
    let mut detail = Detail::new();
    if options.scenario.includes_startup() {
        measure_startup(options, &budgets, &mut rows, &mut detail)?;
    }
    if options.scenario.includes_idle() {
        measure_idle(options, &budgets, &mut rows, &mut detail)?;
    }
    if options.scenario.includes_handles() {
        measure_handles(options, &budgets, &mut rows, &mut detail)?;
    }
    if options.scenario.includes_ingest() {
        measure_ingest(options, &budgets, &mut rows, &mut detail)?;
    }
    if options.scenario.includes_viewer_memory() {
        measure_viewer_memory(&budgets, &mut rows, &mut detail)?;
    }
    Ok((rows, Value::Object(detail)))
}

#[cfg(not(windows))]
fn runner_info() -> Value {
    Value::Null
}

/// `cargo xtask perf`.
pub fn run(args: &[String]) -> Result<String, String> {
    let options = parse_args(args)?;
    // The viewer run builds its own bench binary; no exe is involved.
    if !options.scenario.includes_viewer_memory() && !options.exe.is_file() {
        return Err(format!(
            "{} does not exist; build it with cargo build --release first",
            options.exe.display()
        ));
    }
    let (rows, detail) = measure(&options)?;
    let document = serde_json::json!({
        "schemaVersion": SCHEMA_VERSION,
        "runner": runner_info(),
        "exe": options.exe.display().to_string(),
        "detail": detail,
        "rows": rows_json(&rows),
    });
    if let Some(parent) = options.out.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    std::fs::write(
        &options.out,
        serde_json::to_string_pretty(&document).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("{}: {error}", options.out.display()))?;
    let table = markdown(&rows);
    if let Some(summary) = std::env::var_os("GITHUB_STEP_SUMMARY") {
        let _ = std::fs::OpenOptions::new()
            .append(true)
            .open(summary)
            .and_then(|mut file| {
                std::io::Write::write_all(&mut file, format!("### perf\n\n{table}\n").as_bytes())
            });
    }
    let failed: Vec<_> = rows
        .iter()
        .filter(|row| row.verdict == Verdict::Fail)
        .map(|row| row.id)
        .collect();
    if let Some(wanted) = &options.expect_fail {
        // The self-check: a build with a deliberate regression must fail
        // exactly the row that guards against it.
        return if failed.contains(&wanted.as_str()) {
            Ok(format!(
                "{table}\nperf: the row {wanted} failed, as it must for this build"
            ))
        } else {
            Err(format!(
                "{table}\nperf: the row {wanted} did not fail; the harness missed the regression (failed rows: {})",
                failed.join(", ")
            ))
        };
    }
    if failed.is_empty() {
        Ok(format!(
            "{table}\nperf: ok (results in {})",
            options.out.display()
        ))
    } else {
        Err(format!("{table}\nperf: failed rows: {}", failed.join(", ")))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        AB_FAIL, Observed, Scenario, Verdict, ab_verdict, ab_verdict_stepped, idle_from, judge,
        markdown, not_exercised, parse_args, startup_from, stats,
    };
    use crate::budgets::Budget;

    fn probe() -> serde_json::Value {
        let sample = |at: u64, ticks: u64, switches: Option<u64>, reads: u64| {
            json!({
                "atUs": at, "privateBytes": 10_485_760, "workingSet": 20_971_520,
                "peakWorkingSet": 25_000_000, "pageFaults": 100,
                "kernelTicks": ticks, "userTicks": ticks, "handles": 90,
                "gdiObjects": 0, "userObjects": 0, "contextSwitches": switches,
                "readBytes": reads, "writeBytes": 0, "readOps": 0, "writeOps": 0
            })
        };
        json!({
            "milestones": [
                {"name": "ProcessStart", "firstUs": 200, "lastUs": 200, "count": 1},
                {"name": "EngineReady", "firstUs": 25_000, "lastUs": 25_000, "count": 1}
            ],
            "samples": {
                "start": sample(30_000, 400_000, Some(100), 500_000),
                "end": sample(60_030_000, 1_000_000, Some(160), 700_000)
            }
        })
    }

    fn budgets() -> Vec<Budget> {
        let row = |id: &str, gate: &str, limit: f64| Budget {
            id: id.to_owned(),
            metric: String::new(),
            unit: String::from("per min"),
            bound: String::from(if id == "ingest.burst" { "min" } else { "max" }),
            limit: Some(limit),
            class: String::new(),
            gate: gate.to_owned(),
            scenario: String::new(),
            status: String::from("provisional"),
            source: String::new(),
            workload: String::new(),
            how: String::new(),
            arch: Vec::new(),
        };
        vec![
            row("engine.idle.wakes", "pr-strict", 2.0),
            row("engine.ready.warm", "pr-ab", 100.0),
            row("ingest.burst", "pr-ab", 100_000.0),
            row("ingest.sustained", "pr-ab", 20_000.0),
            row("ingest.undetected_loss", "pr-strict", 0.0),
            row("engine.private_bytes.burst", "nightly-trend", 40.0),
            row("event.latency.p50", "pr-ab", 10.0),
            row("event.latency.p99", "pr-ab", 50.0),
            row("engine.handles.stable", "pr-strict", 0.0),
        ]
    }

    #[test]
    fn startup_numbers_come_from_the_ready_sample() {
        let startup = startup_from(&probe()).unwrap();
        assert!((startup.ready_ms - 25.0).abs() < 1e-9);
        // (400000 + 400000) ticks of 100 ns = 80 ms.
        assert!((startup.cpu_ms - 80.0).abs() < 1e-9);
        assert!((startup.disk_read_mb - 0.5).abs() < 1e-9);
        assert!(startup_from(&json!({})).is_err());
    }

    #[test]
    fn idle_numbers_come_from_the_outside_observation() {
        let seen = Observed {
            seconds: 60.0,
            // 0.12 s of CPU in 100 ns ticks.
            cpu_ticks: 1_200_000.0,
            switches: Some(30.0),
        };
        let idle = idle_from(&probe(), seen).unwrap();
        assert!((idle.seconds - 60.0).abs() < 1e-9);
        // 0.12 s over 60 s = 0.2 %.
        assert!((idle.cpu_percent - 0.2).abs() < 1e-9);
        // 30 switches in 60 s = 30 per minute.
        assert!((idle.wakes_per_min.unwrap() - 30.0).abs() < 1e-9);
        // Memory and handles are the probe's closing sample.
        assert!((idle.private_mb - 10.0).abs() < 1e-9);
        assert!((idle.handles - 90.0).abs() < 1e-9);
    }

    #[test]
    fn missing_context_switches_leave_wakes_unmeasured() {
        let seen = Observed {
            seconds: 60.0,
            cpu_ticks: 0.0,
            switches: None,
        };
        assert_eq!(idle_from(&probe(), seen).unwrap().wakes_per_min, None);
    }

    #[test]
    fn stats_of_odd_even_and_empty_series() {
        let odd = stats(&[3.0, 1.0, 2.0]).unwrap();
        assert_eq!((odd.min, odd.median, odd.max), (1.0, 2.0, 3.0));
        let even = stats(&[4.0, 1.0, 3.0, 2.0]).unwrap();
        assert_eq!(even.median, 2.5);
        assert_eq!(stats(&[]), None);
    }

    #[test]
    fn only_deterministic_rows_are_gated_on_absolute_numbers() {
        let rows = budgets();
        assert_eq!(
            judge(&rows, "engine.idle.wakes", 1.5, "").verdict,
            Verdict::Pass
        );
        assert_eq!(
            judge(&rows, "engine.idle.wakes", 3.0, "").verdict,
            Verdict::Fail
        );
        // A timing row above its limit is reported, not failed (F11).
        let timing = judge(&rows, "engine.ready.warm", 500.0, "");
        assert_eq!(timing.verdict, Verdict::Report);
        assert!(timing.note.contains("A/B"), "{}", timing.note);
        // A row missing from budgets.toml is an error, not a pass.
        assert_eq!(judge(&rows, "engine.nope", 1.0, "").verdict, Verdict::Fail);
    }

    #[test]
    fn ab_warns_at_fifteen_and_fails_at_fifty_percent() {
        assert_eq!(ab_verdict(100.0, 110.0), Verdict::Pass);
        assert_eq!(ab_verdict(100.0, 116.0), Verdict::Warn);
        assert_eq!(ab_verdict(100.0, 100.0 * (1.0 + AB_FAIL)), Verdict::Fail);
        assert_eq!(ab_verdict(100.0, 60.0), Verdict::Pass);
        assert_eq!(ab_verdict(0.0, 5.0), Verdict::Report);
        // Whole clock ticks: 2 against 3 ticks is no change, 2 against 20 is.
        assert_eq!(ab_verdict_stepped(31.25, 46.88, 31.25), Verdict::Pass);
        assert_eq!(ab_verdict_stepped(46.88, 31.25, 31.25), Verdict::Pass);
        assert_eq!(ab_verdict_stepped(31.25, 312.5, 31.25), Verdict::Fail);
    }

    #[test]
    fn the_table_names_unexercised_rows_instead_of_hiding_them() {
        let text = markdown(&[
            judge(&budgets(), "engine.idle.wakes", 1.0, "ok"),
            not_exercised("ingest", "no capture backend yet (#45)"),
        ]);
        assert!(text.contains("| `engine.idle.wakes` | 1.00 per min | 2 per min | pass | ok |"));
        assert!(
            text.contains("| `ingest` | - | - | not exercised | no capture backend yet (#45) |")
        );
    }

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn the_command_line_is_parsed_and_checked() {
        let options = parse_args(&args(&[
            "--scenario",
            "idle",
            "--runs",
            "5",
            "--idle-secs",
            "10",
        ]))
        .unwrap();
        assert_eq!(options.scenario, Scenario::Idle);
        assert_eq!((options.runs, options.idle_secs), (5, 10));
        assert!(parse_args(&args(&["--scenario", "nope"])).is_err());
        assert!(parse_args(&args(&["--runs", "0"])).is_err());
        assert!(parse_args(&args(&["--runs"])).is_err());
        assert!(parse_args(&args(&["--frobnicate"])).is_err());
    }

    fn ingest_doc(profile: &str, exercised: bool, undetected: u64, gaps: u64) -> serde_json::Value {
        json!({
            "scenario": "ingest", "profile": profile, "exercised": exercised,
            "ledgerEvents": 1_000_000, "achievedPerSecond": 120_000, "churnSeconds": 8.5,
            "undetected": undetected, "unfinishedGaps": gaps, "peakPrivateMb": 31.5,
            "latencyP50Ms": 4.0, "latencyP99Ms": 60.0, "latencySamples": 300_000
        })
    }

    #[test]
    fn a_burst_document_gives_rate_and_memory_rows_and_counts_losses() {
        let mut rows = Vec::new();
        let lost = super::ingest_rows(&budgets(), &ingest_doc("burst-100k", true, 0, 0), &mut rows)
            .unwrap();
        assert!((lost - 0.0).abs() < f64::EPSILON);
        let ids: Vec<&str> = rows.iter().map(|row| row.id).collect();
        assert_eq!(ids, ["ingest.burst", "engine.private_bytes.burst"]);
        // 120,000 events/s against a minimum of 100,000: within, and only
        // reported (the gate is the A/B).
        assert_eq!(rows[0].verdict, Verdict::Report);
        assert_eq!(rows[0].value, Some(120_000.0));
    }

    #[test]
    fn a_generator_that_was_too_slow_says_not_exercised() {
        let mut rows = Vec::new();
        super::ingest_rows(
            &budgets(),
            &ingest_doc("sustained-20k", false, 0, 0),
            &mut rows,
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "ingest.sustained");
        assert_eq!(rows[0].verdict, Verdict::NotExercised);
    }

    #[test]
    fn a_latency_document_gives_both_percentiles() {
        let mut rows = Vec::new();
        super::ingest_rows(
            &budgets(),
            &ingest_doc("latency-10k", true, 0, 0),
            &mut rows,
        )
        .unwrap();
        let ids: Vec<&str> = rows.iter().map(|row| row.id).collect();
        assert_eq!(ids, ["event.latency.p50", "event.latency.p99"]);
    }

    #[test]
    fn losses_and_unfinished_gaps_are_both_losses() {
        let mut rows = Vec::new();
        let lost = super::ingest_rows(&budgets(), &ingest_doc("burst-100k", true, 3, 2), &mut rows)
            .unwrap();
        assert!((lost - 5.0).abs() < f64::EPSILON);
        // Strict: the row the sum is judged by fails at the first loss.
        let row = judge(&budgets(), "ingest.undetected_loss", lost, "n");
        assert_eq!(row.verdict, Verdict::Fail);
    }

    #[test]
    fn an_unknown_profile_has_no_rows() {
        let mut rows = Vec::new();
        assert!(
            super::ingest_rows(&budgets(), &ingest_doc("nope", true, 0, 0), &mut rows).is_err()
        );
    }

    #[test]
    fn the_growth_between_the_two_logs_is_judged_strictly() {
        let rows = vec![{
            let mut row = budgets().remove(0);
            "viewer.private_bytes.growth".clone_into(&mut row.id);
            row.gate = String::from("nightly-strict");
            row.limit = Some(5.0);
            row
        }];
        let doc = |rows: u64, after: u64| json!({ "rows": rows, "privateBytesAfter": after, "heldRows": 900 });
        let mb = 1_048_576;
        let small = doc(1_000_000, 100 * mb);
        let within = super::viewer_memory_row(&rows, &small, &doc(10_000_000, 104 * mb)).unwrap();
        assert_eq!(within.verdict, Verdict::Pass);
        assert!((within.value.unwrap() - 4.0).abs() < 1e-9);
        let over = super::viewer_memory_row(&rows, &small, &doc(10_000_000, 106 * mb)).unwrap();
        assert_eq!(over.verdict, Verdict::Fail);
        // A smaller second log is no growth.
        let less = super::viewer_memory_row(&rows, &small, &doc(10_000_000, 90 * mb)).unwrap();
        assert_eq!(less.verdict, Verdict::Pass);
        // A document without the number is an error, not a pass.
        assert!(super::viewer_memory_row(&rows, &small, &json!({})).is_err());
    }

    #[test]
    fn the_new_scenarios_and_flags_parse() {
        let options = parse_args(&args(&[
            "--scenario",
            "ingest",
            "--profile",
            "burst-100k",
            "--profile",
            "latency-10k",
            "--expect-fail",
            "ingest.undetected_loss",
        ]))
        .unwrap();
        assert_eq!(options.scenario, Scenario::Ingest);
        assert_eq!(options.profiles, ["burst-100k", "latency-10k"]);
        assert_eq!(
            options.expect_fail.as_deref(),
            Some("ingest.undetected_loss")
        );
        assert!(Scenario::All.includes_ingest() && Scenario::All.includes_handles());
        assert!(!Scenario::Idle.includes_ingest() && !Scenario::Startup.includes_handles());
        assert_eq!(
            parse_args(&args(&["--scenario", "handles"]))
                .unwrap()
                .scenario,
            Scenario::Handles
        );
    }
}
