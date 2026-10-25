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

const USAGE: &str = "usage: cargo xtask perf [--scenario startup|idle|memory|all] [--exe path] \
[--runs n] [--warmup n] [--idle-secs n] [--ab <base-exe>] [--out file]";

/// What to measure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scenario {
    Startup,
    Idle,
    Memory,
    All,
}

impl Scenario {
    fn includes_startup(self) -> bool {
        matches!(self, Self::Startup | Self::All)
    }

    fn includes_idle(self) -> bool {
        matches!(self, Self::Idle | Self::Memory | Self::All)
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
                    "all" => Scenario::All,
                    other => return Err(format!("unknown scenario {other}\n{USAGE}")),
                };
            }
            "--exe" => options.exe = PathBuf::from(value("--exe")?),
            "--runs" => options.runs = number(&value("--runs")?)?,
            "--warmup" => options.warmup = number(&value("--warmup")?)?,
            "--idle-secs" => options.idle_secs = number(&value("--idle-secs")?)?,
            "--ab" => options.ab_base = Some(PathBuf::from(value("--ab")?)),
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
    let strict = row.gate == "pr-strict";
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

fn startup_runs(options: &Options, exe: &Path, tag: &str) -> Result<Vec<Startup>, String> {
    let mut runs = Vec::new();
    for index in 0..options.warmup + options.runs {
        let probe = launch::probe(exe, 0, &format!("{tag}{index}"))?;
        if index >= options.warmup {
            runs.push(startup_from(&probe)?);
        }
    }
    Ok(runs)
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
    let head = startup_runs(options, &options.exe, "h")?;
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
    if let Some(base_exe) = &options.ab_base {
        let base = startup_runs(options, base_exe, "b")?;
        let base_ready = stats(&base.iter().map(|run| run.ready_ms).collect::<Vec<_>>());
        let base_cpu = stats(&base.iter().map(|run| run.cpu_ms).collect::<Vec<_>>());
        if let (Some(b), Some(h)) = (base_ready, ready) {
            ready_row.verdict = ab_verdict(b.median, h.median);
            let _ = write!(ready_row.note, "; A/B base {:.2} ms", b.median);
        }
        if let (Some(b), Some(h)) = (base_cpu, cpu) {
            cpu_row.verdict = ab_verdict(b.median, h.median);
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
    rows.push(not_exercised(
        "engine.handles.stable",
        "needs arm/ingest/pause/resume cycles (#63, #45)",
    ));
    rows.push(not_exercised(
        "ingest",
        "no capture backend or writer yet (#45, #52)",
    ));
    Ok((rows, Value::Object(detail)))
}

#[cfg(not(windows))]
fn runner_info() -> Value {
    Value::Null
}

/// `cargo xtask perf`.
pub fn run(args: &[String]) -> Result<String, String> {
    let options = parse_args(args)?;
    if !options.exe.is_file() {
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
        AB_FAIL, Observed, Scenario, Verdict, ab_verdict, idle_from, judge, markdown,
        not_exercised, parse_args, startup_from, stats,
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
            bound: String::from("max"),
            limit: Some(limit),
            class: String::new(),
            gate: gate.to_owned(),
            status: String::from("provisional"),
            source: String::new(),
            workload: String::new(),
            how: String::new(),
            arch: Vec::new(),
        };
        vec![
            row("engine.idle.wakes", "pr-strict", 2.0),
            row("engine.ready.warm", "pr-ab", 100.0),
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

    #[test]
    fn the_command_line_is_parsed_and_checked() {
        let args = |list: &[&str]| list.iter().map(ToString::to_string).collect::<Vec<_>>();
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
}
