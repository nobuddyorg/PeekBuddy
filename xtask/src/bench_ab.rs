//! `cargo xtask bench-ab`: same-runner A/B timing gate (#26, F11).
//!
//! Builds the criterion benches of the merge base and of the working tree
//! in one job, runs them in alternating rounds (at least 10 per side, short
//! criterion runs), and compares the medians of the per-round medians: warn
//! at +15 %, fail at +50 %. Only benches whose `bench.*` row in
//! `perf/budgets.toml` has `gate = "pr-ab"` can fail the run; the others are
//! reported. A bench without a row is an error, so every bench is budgeted.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::budgets;

/// Warn above this head/base ratio.
pub const WARN_RATIO: f64 = 1.15;
/// Fail above this head/base ratio.
pub const FAIL_RATIO: f64 = 1.50;

/// Criterion settings per round: short, the rounds give the statistics.
const CRITERION_ARGS: &[&str] = &[
    "--bench",
    "--noplot",
    "--warm-up-time",
    "0.3",
    "--measurement-time",
    "1",
    "--sample-size",
    "10",
];

/// Outcome for one bench.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Within +15 %.
    Ok,
    /// Slower by more than 15 %.
    Warn,
    /// Slower by more than 50 % in a gated bench.
    Fail,
    /// Slower by more than 50 % in a bench that is only trended.
    Slow,
    /// Only on the head side.
    New,
    /// Only on the base side.
    Removed,
}

impl Verdict {
    fn label(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warn => "warn (> +15 %)",
            Self::Fail => "**FAIL** (> +50 %)",
            Self::Slow => "slow (> +50 %, not gated)",
            Self::New => "new",
            Self::Removed => "removed",
        }
    }
}

/// One row of the report.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// Criterion id, for example `perf/milestone_record`.
    pub id: String,
    /// Median of the base rounds, ns.
    pub base: Option<f64>,
    /// Median of the head rounds, ns.
    pub head: Option<f64>,
    /// Whether the bench can fail the run.
    pub gated: bool,
    /// Outcome.
    pub verdict: Verdict,
}

/// The budget id of a criterion id: `bench.` plus the id with `/` as `.`.
#[must_use]
pub fn budget_id(bench: &str) -> String {
    format!("bench.{}", bench.replace('/', "."))
}

/// The median of `values` (mean of the middle two for an even count).
#[must_use]
pub fn median(values: &[f64]) -> Option<f64> {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let mid = sorted.len() / 2;
    match sorted.len() {
        0 => None,
        n if n % 2 == 1 => sorted.get(mid).copied(),
        _ => Some((sorted.get(mid - 1)? + sorted.get(mid)?) / 2.0),
    }
}

/// The verdict for base and head medians.
#[must_use]
pub fn verdict(base: Option<f64>, head: Option<f64>, gated: bool) -> Verdict {
    match (base, head) {
        (None, _) => Verdict::New,
        (Some(_), None) => Verdict::Removed,
        (Some(base), Some(head)) => {
            let ratio = head / base;
            if ratio > FAIL_RATIO {
                if gated { Verdict::Fail } else { Verdict::Slow }
            } else if ratio > WARN_RATIO {
                Verdict::Warn
            } else {
                Verdict::Ok
            }
        }
    }
}

/// Builds the report rows from per-round medians of both sides.
///
/// # Errors
///
/// Returns the head benches that have no `bench.*` budget row.
pub fn compare(
    base: &BTreeMap<String, Vec<f64>>,
    head: &BTreeMap<String, Vec<f64>>,
    budget_gates: &BTreeMap<String, String>,
) -> Result<Vec<Row>, String> {
    let missing: Vec<String> = head
        .keys()
        .filter(|id| !budget_gates.contains_key(&budget_id(id)))
        .map(|id| budget_id(id))
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "bench-ab: no perf/budgets.toml row for {}",
            missing.join(", ")
        ));
    }
    let mut ids: Vec<&String> = base.keys().chain(head.keys()).collect();
    ids.sort();
    ids.dedup();
    Ok(ids
        .into_iter()
        .map(|id| {
            let gated = budget_gates
                .get(&budget_id(id))
                .is_some_and(|gate| gate == "pr-ab");
            let base = base.get(id).and_then(|v| median(v));
            let head = head.get(id).and_then(|v| median(v));
            Row {
                id: id.clone(),
                base,
                head,
                gated,
                verdict: verdict(base, head, gated),
            }
        })
        .collect())
}

fn format_ns(value: Option<f64>) -> String {
    match value {
        None => "-".to_owned(),
        Some(ns) if ns >= 1_000_000.0 => format!("{:.2} ms", ns / 1_000_000.0),
        Some(ns) if ns >= 1_000.0 => format!("{:.2} us", ns / 1_000.0),
        Some(ns) => format!("{ns:.2} ns"),
    }
}

/// The Markdown report.
#[must_use]
pub fn render(rows: &[Row], rounds: usize, shape: &str) -> String {
    let mut out = String::from("## Bench A/B (same runner)\n\n");
    let _ = writeln!(
        out,
        "{rounds} alternating rounds per side, median of per-round medians; \
         warn at +15 %, fail at +50 % for `pr-ab` benches. Runner: {shape}.\n"
    );
    out.push_str("| bench | gate | base | head | change | verdict |\n");
    out.push_str("| --- | --- | --- | --- | --- | --- |\n");
    for row in rows {
        let change = match (row.base, row.head) {
            (Some(base), Some(head)) => format!("{:+.1} %", (head / base - 1.0) * 100.0),
            _ => "-".to_owned(),
        };
        let gate = if row.gated { "pr-ab" } else { "trend" };
        let _ = writeln!(
            out,
            "| `{}` | {gate} | {} | {} | {change} | {} |",
            row.id,
            format_ns(row.base),
            format_ns(row.head),
            row.verdict.label()
        );
    }
    out
}

/// Bench executables from `cargo bench --no-run --message-format json`.
#[must_use]
pub fn executables(json_lines: &str) -> Vec<PathBuf> {
    json_lines
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|message| {
            message["reason"] == "compiler-artifact"
                && message["target"]["kind"]
                    .as_array()
                    .is_some_and(|kinds| kinds.iter().any(|kind| kind == "bench"))
        })
        .filter_map(|message| message["executable"].as_str().map(PathBuf::from))
        .collect()
}

/// Median point estimates (ns) under a criterion home: one entry per
/// `<id>/new/estimates.json`.
///
/// # Errors
///
/// Returns an I/O or parse error.
pub fn estimates(home: &Path) -> Result<BTreeMap<String, f64>, String> {
    let mut out = BTreeMap::new();
    if !home.exists() {
        return Ok(out);
    }
    let mut dirs = vec![home.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if !path.is_dir() {
                continue;
            }
            let file = path.join("new").join("estimates.json");
            if file.is_file() {
                let text = std::fs::read_to_string(&file)
                    .map_err(|e| format!("{}: {e}", file.display()))?;
                let value: serde_json::Value =
                    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))?;
                let median = value["median"]["point_estimate"]
                    .as_f64()
                    .ok_or_else(|| format!("{}: no median", file.display()))?;
                let id = path
                    .strip_prefix(home)
                    .map_err(|e| e.to_string())?
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                out.insert(id, median);
            } else if path.file_name().is_some_and(|name| name != "report") {
                dirs.push(path);
            }
        }
    }
    Ok(out)
}

/// Options of `cargo xtask bench-ab`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// Git revision of the base side; default: merge base with `origin/main`.
    pub base: Option<String>,
    /// Rounds per side (at least 10 for the gate).
    pub rounds: usize,
    /// Criterion filter passed to every run.
    pub filter: Option<String>,
}

/// Parses `[--base <rev>] [--rounds <n>] [--filter <regex>]`.
///
/// # Errors
///
/// Returns a usage message for unknown options or bad values.
pub fn parse(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        base: None,
        rounds: 10,
        filter: None,
    };
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next()
                .cloned()
                .ok_or_else(|| format!("{arg} needs a value"))
        };
        match arg.as_str() {
            "--base" => options.base = Some(value()?),
            "--rounds" => {
                options.rounds = value()?.parse().map_err(|e| format!("--rounds: {e}"))?;
            }
            "--filter" => options.filter = Some(value()?),
            other => {
                return Err(format!(
                    "unknown option {other}; usage: cargo xtask bench-ab [--base <rev>] \
                     [--rounds <n>] [--filter <regex>]"
                ));
            }
        }
    }
    if options.rounds < 1 {
        return Err("--rounds must be at least 1".to_owned());
    }
    Ok(options)
}

fn run_checked(command: &mut Command) -> Result<String, String> {
    let output = command
        .output()
        .map_err(|e| format!("cannot run {command:?}: {e}"))?;
    if output.status.success() {
        String::from_utf8(output.stdout).map_err(|e| e.to_string())
    } else {
        Err(format!(
            "{command:?} failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

/// Builds the benches in `source` into `target` and returns the executables
/// (empty when the revision has no benches package).
fn build(source: &Path, target: &Path) -> Result<Vec<PathBuf>, String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let output = Command::new(cargo)
        .current_dir(source)
        .env("CARGO_TARGET_DIR", target)
        .args([
            "bench",
            "--no-run",
            "--package",
            "benches",
            "--locked",
            "--message-format",
            "json",
        ])
        .output()
        .map_err(|e| format!("cannot run cargo bench: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("did not match any packages") {
            return Ok(Vec::new());
        }
        return Err(format!(
            "cargo bench --no-run in {}:\n{stderr}",
            source.display()
        ));
    }
    Ok(executables(&String::from_utf8_lossy(&output.stdout)))
}

/// One criterion round of every executable; returns the medians by id.
fn round(
    exes: &[PathBuf],
    source: &Path,
    home: &Path,
    filter: Option<&str>,
) -> Result<BTreeMap<String, f64>, String> {
    for exe in exes {
        let mut command = Command::new(exe);
        command
            .current_dir(source)
            .env("CRITERION_HOME", home)
            .args(CRITERION_ARGS);
        if let Some(filter) = filter {
            command.arg(filter);
        }
        run_checked(&mut command)?;
    }
    estimates(home)
}

/// `cargo xtask bench-ab`.
///
/// # Errors
///
/// Returns the report as the error when a gated bench fails, or the error
/// of a failed step.
pub fn run(args: &[String]) -> Result<String, String> {
    let options = parse(args)?;
    let out = PathBuf::from("target").join("bench-ab");
    let base_src = out.join("base-src");
    let base_rev = match &options.base {
        Some(rev) => rev.clone(),
        None => run_checked(Command::new("git").args(["merge-base", "origin/main", "HEAD"]))?
            .trim()
            .to_owned(),
    };
    // A fresh worktree of the base revision next to the working tree.
    let _ = Command::new("git")
        .args(["worktree", "remove", "--force"])
        .arg(&base_src)
        .output();
    run_checked(
        Command::new("git")
            .args(["worktree", "add", "--detach"])
            .arg(&base_src)
            .arg(&base_rev),
    )?;
    let base_src = std::fs::canonicalize(&base_src).map_err(|e| e.to_string())?;
    let here = std::env::current_dir().map_err(|e| e.to_string())?;
    let out = here.join(&out);
    let base_exes = build(&base_src, &out.join("base-target"))?;
    let head_exes = build(&here, &out.join("head-target"))?;

    let mut base: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    let mut head: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for index in 0..options.rounds {
        // Alternate which side goes first so drift hits both alike.
        let order = if index % 2 == 0 {
            [false, true]
        } else {
            [true, false]
        };
        for is_head in order {
            let (exes, source, side, samples) = if is_head {
                (&head_exes, &here, "head", &mut head)
            } else {
                (&base_exes, &base_src, "base", &mut base)
            };
            let home = out.join("rounds").join(side).join(index.to_string());
            let _ = std::fs::remove_dir_all(&home);
            for (id, median) in round(exes, source, &home, options.filter.as_deref())? {
                samples.entry(id).or_default().push(median);
            }
        }
    }

    let gates: BTreeMap<String, String> = budgets::load()?
        .into_iter()
        .map(|budget| (budget.id, budget.gate))
        .collect();
    let rows = compare(&base, &head, &gates)?;
    let shape = format!(
        "{} {} with {} logical CPUs, base {}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::thread::available_parallelism().map_or(0, std::num::NonZero::get),
        base_rev.get(..12).unwrap_or(&base_rev)
    );
    let report = render(&rows, options.rounds, &shape);
    std::fs::write(out.join("report.md"), &report).map_err(|e| e.to_string())?;
    if let Some(summary) = std::env::var_os("GITHUB_STEP_SUMMARY") {
        let mut text = std::fs::read_to_string(&summary).unwrap_or_default();
        text.push_str(&report);
        std::fs::write(&summary, text).map_err(|e| e.to_string())?;
    }
    let _ = Command::new("git")
        .args(["worktree", "remove", "--force"])
        .arg(&base_src)
        .output();
    if rows.iter().any(|row| row.verdict == Verdict::Fail) {
        Err(report)
    } else {
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{
        Options, Row, Verdict, budget_id, compare, estimates, executables, median, parse, render,
        verdict,
    };

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn medians() {
        assert_eq!(median(&[]), None);
        assert_eq!(median(&[3.0]), Some(3.0));
        assert_eq!(median(&[5.0, 1.0, 3.0]), Some(3.0));
        assert_eq!(median(&[4.0, 1.0, 3.0, 2.0]), Some(2.5));
        assert_eq!(median(&[10.0, 1.0, 1.0, 1.0, 10.0]), Some(1.0));
    }

    #[test]
    fn verdict_thresholds() {
        assert_eq!(verdict(Some(100.0), Some(115.0), true), Verdict::Ok);
        assert_eq!(verdict(Some(100.0), Some(115.1), true), Verdict::Warn);
        assert_eq!(verdict(Some(100.0), Some(150.0), true), Verdict::Warn);
        assert_eq!(verdict(Some(100.0), Some(150.1), true), Verdict::Fail);
        assert_eq!(verdict(Some(100.0), Some(150.1), false), Verdict::Slow);
        assert_eq!(verdict(Some(100.0), Some(10.0), true), Verdict::Ok);
        assert_eq!(verdict(None, Some(1.0), true), Verdict::New);
        assert_eq!(verdict(Some(1.0), None, true), Verdict::Removed);
    }

    fn samples(list: &[(&str, &[f64])]) -> BTreeMap<String, Vec<f64>> {
        list.iter()
            .map(|(id, values)| ((*id).to_owned(), values.to_vec()))
            .collect()
    }

    #[test]
    fn compare_gates_only_pr_ab_rows() {
        let base = samples(&[
            ("a/x", &[10.0, 11.0, 9.0]),
            ("a/y", &[10.0]),
            ("a/old", &[1.0]),
        ]);
        let head = samples(&[
            ("a/x", &[20.0, 21.0, 19.0]),
            ("a/y", &[20.0]),
            ("a/new", &[1.0]),
        ]);
        let gates: BTreeMap<String, String> = [
            ("bench.a.x", "pr-ab"),
            ("bench.a.y", "nightly-trend"),
            ("bench.a.new", "pr-ab"),
        ]
        .iter()
        .map(|(id, gate)| ((*id).to_owned(), (*gate).to_owned()))
        .collect();
        let rows = compare(&base, &head, &gates).unwrap();
        let verdicts: Vec<(&str, Verdict)> = rows
            .iter()
            .map(|row| (row.id.as_str(), row.verdict))
            .collect();
        assert_eq!(
            verdicts,
            [
                ("a/new", Verdict::New),
                ("a/old", Verdict::Removed),
                ("a/x", Verdict::Fail),
                ("a/y", Verdict::Slow)
            ]
        );
        assert_eq!(rows[2].base, Some(10.0));
        assert_eq!(rows[2].head, Some(20.0));
    }

    #[test]
    fn a_bench_without_a_budget_row_is_an_error() {
        let head = samples(&[("a/x", &[1.0])]);
        let error = compare(&BTreeMap::new(), &head, &BTreeMap::new()).unwrap_err();
        assert!(error.contains("bench.a.x"), "{error}");
    }

    #[test]
    fn budget_ids_use_dots() {
        assert_eq!(
            budget_id("perf/milestone_record"),
            "bench.perf.milestone_record"
        );
    }

    #[test]
    fn the_report_shows_every_row() {
        let rows = [
            Row {
                id: "a/x".into(),
                base: Some(1_500.0),
                head: Some(3_000_000.0),
                gated: true,
                verdict: Verdict::Fail,
            },
            Row {
                id: "a/new".into(),
                base: None,
                head: Some(0.5),
                gated: false,
                verdict: Verdict::New,
            },
        ];
        let report = render(&rows, 10, "linux x86_64 with 4 logical CPUs");
        assert!(
            report.contains(
                "| `a/x` | pr-ab | 1.50 us | 3.00 ms | +199900.0 % | **FAIL** (> +50 %) |"
            ),
            "{report}"
        );
        assert!(
            report.contains("| `a/new` | trend | - | 0.50 ns | - | new |"),
            "{report}"
        );
        assert!(report.contains("10 alternating rounds"));
        assert!(report.contains("4 logical CPUs"));
    }

    #[test]
    fn executables_come_from_bench_artifacts_only() {
        let json = concat!(
            r#"{"reason":"compiler-artifact","target":{"kind":["lib"]},"executable":null}"#,
            "\n",
            r#"{"reason":"compiler-artifact","target":{"kind":["bench"]},"executable":"/t/hot_paths-1"}"#,
            "\n",
            r#"{"reason":"compiler-artifact","target":{"kind":["test"]},"executable":"/t/test-1"}"#,
            "\n",
            r#"{"reason":"build-finished","success":true}"#,
            "\n",
            "not json\n",
        );
        assert_eq!(
            executables(json),
            [std::path::PathBuf::from("/t/hot_paths-1")]
        );
    }

    #[test]
    fn estimates_are_read_by_id() {
        let home = std::env::temp_dir().join(format!("bench-ab-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        for (id, median) in [("perf/milestone_record", 14.5), ("metrics/list", 1200.0)] {
            let dir = home.join(id).join("new");
            std::fs::create_dir_all(&dir).unwrap();
            let json = format!(
                r#"{{"median":{{"point_estimate":{median}}},"mean":{{"point_estimate":1}}}}"#
            );
            std::fs::write(dir.join("estimates.json"), json).unwrap();
        }
        std::fs::create_dir_all(home.join("report")).unwrap();
        let found = estimates(&home).unwrap();
        std::fs::remove_dir_all(&home).unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(found["perf/milestone_record"], 14.5);
        assert_eq!(found["metrics/list"], 1200.0);
        assert!(estimates(&home).unwrap().is_empty());
    }

    #[test]
    fn options() {
        assert_eq!(
            parse(&args(&[])).unwrap(),
            Options {
                base: None,
                rounds: 10,
                filter: None
            }
        );
        assert_eq!(
            parse(&args(&[
                "--base", "abc", "--rounds", "3", "--filter", "perf"
            ]))
            .unwrap(),
            Options {
                base: Some("abc".into()),
                rounds: 3,
                filter: Some("perf".into())
            }
        );
        assert!(parse(&args(&["--rounds", "0"])).is_err());
        assert!(parse(&args(&["--rounds"])).is_err());
        assert!(parse(&args(&["--rounds", "x"])).is_err());
        assert!(parse(&args(&["--nope"])).is_err());
    }
}
