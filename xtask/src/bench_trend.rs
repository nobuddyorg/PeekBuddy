//! `cargo xtask bench-trend`: the nightly benchmark trend (#26, F11).
//!
//! Runs every criterion bench of the working tree in alternating short
//! rounds (like `bench-ab`, one side only), writes the medians as
//! `customSmallerIsBetter` JSON for github-action-benchmark, and compares
//! each bench with the rolling median of its last [`WINDOW`] nightly points
//! from the published history. A bench more than +25 % above that median is
//! an alert; github-action-benchmark itself only compares with the previous
//! point.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;

use serde::Serialize;

use crate::bench_ab;

/// Nightly points in the rolling median.
pub const WINDOW: usize = 7;

/// Fewer points than this give no baseline yet.
pub const MIN_POINTS: usize = 3;

/// Alert above this ratio to the rolling median.
pub const ALERT_RATIO: f64 = 1.25;

/// One entry of the `customSmallerIsBetter` input.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Point {
    /// Criterion id.
    pub name: String,
    /// Always `ns`.
    pub unit: &'static str,
    /// Median of the per-round medians.
    pub value: f64,
}

/// Outcome for one bench.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trend {
    /// Within +25 % of the rolling median.
    Ok,
    /// More than +25 % above the rolling median tonight and the night before.
    Alert,
    /// More than +25 % above the rolling median tonight only: the hosted
    /// runners come in two speeds, so one slow night proves nothing (#403).
    Watch,
    /// Fewer than [`MIN_POINTS`] earlier points.
    NoBaseline,
    /// Would alert on the history, but the same-runner A/B against the base
    /// revision finds no slowdown: the runner was slow, not the code.
    NotConfirmed,
}

/// One row of the report.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// Criterion id.
    pub name: String,
    /// Tonight's value, ns.
    pub value: f64,
    /// Rolling median of the earlier points, ns.
    pub baseline: Option<f64>,
    /// Outcome.
    pub trend: Trend,
    /// Head over base of the same-runner A/B that judged an alert, when one
    /// was run.
    pub ab_ratio: Option<f64>,
}

/// Earlier values per bench from github-action-benchmark's `data.js`
/// (`window.BENCHMARK_DATA = {...}`), oldest first, for the entry `entry`.
///
/// # Errors
///
/// Returns a parse error for text that is neither empty nor `data.js`.
pub fn history(data_js: &str, entry: &str) -> Result<BTreeMap<String, Vec<f64>>, String> {
    let mut out: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    let Some(start) = data_js.find('{') else {
        return Ok(out);
    };
    let json = data_js
        .get(start..)
        .unwrap_or_default()
        .trim()
        .trim_end_matches(';');
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("bench history: {e}"))?;
    for point in value["entries"][entry].as_array().into_iter().flatten() {
        for bench in point["benches"].as_array().into_iter().flatten() {
            if let (Some(name), Some(v)) = (bench["name"].as_str(), bench["value"].as_f64()) {
                out.entry(name.to_owned()).or_default().push(v);
            }
        }
    }
    Ok(out)
}

/// Compares tonight's medians with the rolling median of the last
/// [`WINDOW`] earlier points. A bench alerts only when the night before was
/// above its own rolling median too: the hosted runners alternate between a
/// fast and a slow machine, and a single slow night is not a regression.
#[must_use]
pub fn compare(tonight: &BTreeMap<String, f64>, earlier: &BTreeMap<String, Vec<f64>>) -> Vec<Row> {
    tonight
        .iter()
        .map(|(name, &value)| {
            let points = earlier.get(name).map_or(&[][..], Vec::as_slice);
            let recent = points
                .get(points.len().saturating_sub(WINDOW)..)
                .unwrap_or_default();
            let baseline = (recent.len() >= MIN_POINTS)
                .then(|| bench_ab::median(recent))
                .flatten();
            let trend = match baseline {
                None => Trend::NoBaseline,
                Some(base) if value > base * ALERT_RATIO => {
                    if previous_was_high(points, base) {
                        Trend::Alert
                    } else {
                        Trend::Watch
                    }
                }
                Some(_) => Trend::Ok,
            };
            Row {
                name: name.clone(),
                value,
                baseline,
                trend,
                ab_ratio: None,
            }
        })
        .collect()
}

/// Judges the alerts of `rows` again with a same-runner A/B of the working
/// tree against `base_rev`: a bench keeps its alert only when the paired
/// measurement finds it more than [`bench_ab::WARN_RATIO`] slower than the
/// base. The hosted runners alternate between a fast and a slow machine, and
/// the history cannot tell that from a regression; a measurement of both
/// revisions on one machine can.
///
/// # Errors
///
/// Returns the error of the A/B run.
pub fn confirm(rows: &mut [Row], base_rev: &str, rounds: usize) -> Result<(), String> {
    let alerts: Vec<String> = rows
        .iter()
        .filter(|row| row.trend == Trend::Alert)
        .map(|row| row.name.clone())
        .collect();
    if alerts.is_empty() {
        return Ok(());
    }
    let measured = bench_ab::measure(Some(base_rev), rounds, Some(&alerts.join("|")))?;
    for row in rows.iter_mut().filter(|row| row.trend == Trend::Alert) {
        let base = measured
            .base
            .get(&row.name)
            .and_then(|v| bench_ab::median(v));
        let head = measured
            .head
            .get(&row.name)
            .and_then(|v| bench_ab::median(v));
        if let (Some(base), Some(head)) = (base, head) {
            let ratio = head / base;
            row.ab_ratio = Some(ratio);
            if ratio <= bench_ab::WARN_RATIO {
                row.trend = Trend::NotConfirmed;
            }
        }
    }
    Ok(())
}

/// Whether the last of `points` was itself more than +25 % above the rolling
/// median of the [`WINDOW`] points before it (or above `fallback` when those
/// are too few for a median).
fn previous_was_high(points: &[f64], fallback: f64) -> bool {
    let Some((&last, before)) = points.split_last() else {
        return false;
    };
    let before = before
        .get(before.len().saturating_sub(WINDOW)..)
        .unwrap_or_default();
    let base = if before.len() >= MIN_POINTS {
        bench_ab::median(before).unwrap_or(fallback)
    } else {
        fallback
    };
    last > base * ALERT_RATIO
}

fn format_ns(value: f64) -> String {
    if value >= 1_000_000.0 {
        format!("{:.2} ms", value / 1_000_000.0)
    } else if value >= 1_000.0 {
        format!("{:.2} us", value / 1_000.0)
    } else {
        format!("{value:.2} ns")
    }
}

/// The Markdown report.
#[must_use]
pub fn render(rows: &[Row], rounds: usize, shape: &str) -> String {
    let mut out = String::from("## Bench trend (nightly)\n\n");
    let taken = if rounds == 0 {
        String::from("medians as the bench job reported them")
    } else {
        format!("{rounds} rounds, median of per-round medians")
    };
    let _ = writeln!(
        out,
        "{taken}; alert above +25 % of the \
         rolling median of the last {WINDOW} nightly points. Runner: {shape}.\n"
    );
    out.push_str("| bench | tonight | rolling median | change | trend |\n");
    out.push_str("| --- | --- | --- | --- | --- |\n");
    for row in rows {
        let (baseline, change) = match row.baseline {
            Some(base) => (
                format_ns(base),
                format!("{:+.1} %", (row.value / base - 1.0) * 100.0),
            ),
            None => ("-".to_owned(), "-".to_owned()),
        };
        let trend = match row.trend {
            Trend::Ok => "ok",
            Trend::Alert => "**ALERT** (> +25 %, two nights running)",
            Trend::Watch => "high tonight (alerts if tomorrow is too)",
            Trend::NoBaseline => "no baseline yet",
            Trend::NotConfirmed => "high on the history, not slower in the same-runner A/B",
        };
        let trend = match row.ab_ratio {
            Some(ratio) => format!("{trend} ({:+.1} % vs the base)", (ratio - 1.0) * 100.0),
            None => trend.to_owned(),
        };
        let _ = writeln!(
            out,
            "| `{}` | {} | {baseline} | {change} | {trend} |",
            row.name,
            format_ns(row.value)
        );
    }
    out
}

/// Options of `cargo xtask bench-trend`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// Rounds.
    pub rounds: usize,
    /// github-action-benchmark `data.js` with the earlier points.
    pub history: Option<PathBuf>,
    /// Entry name in `data.js`.
    pub entry: String,
    /// Compare the points of this `customSmallerIsBetter` file instead of
    /// running the benches (the query bench job, #433).
    pub points: Option<PathBuf>,
    /// Judge the alerts again with a same-runner A/B against this revision.
    pub confirm_base: Option<String>,
}

/// Parses `[--rounds <n>] [--history <data.js>] [--entry <name>]`.
///
/// # Errors
///
/// Returns a usage message for unknown options or bad values.
pub fn parse(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        rounds: 10,
        history: None,
        entry: "benches (windows-2025)".to_owned(),
        points: None,
        confirm_base: None,
    };
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next()
                .cloned()
                .ok_or_else(|| format!("{arg} needs a value"))
        };
        match arg.as_str() {
            "--rounds" => {
                options.rounds = value()?.parse().map_err(|e| format!("--rounds: {e}"))?;
            }
            "--history" => options.history = Some(PathBuf::from(value()?)),
            "--entry" => options.entry = value()?,
            "--points" => options.points = Some(PathBuf::from(value()?)),
            "--confirm-base" => options.confirm_base = Some(value()?),
            other => {
                return Err(format!(
                    "unknown option {other}; usage: cargo xtask bench-trend [--rounds <n>] \
                     [--history <data.js>] [--entry <name>] [--points <output.json>]                      [--confirm-base <rev>]"
                ));
            }
        }
    }
    if options.rounds < 1 {
        return Err("--rounds must be at least 1".to_owned());
    }
    Ok(options)
}

/// The values of a `customSmallerIsBetter` file by name.
fn read_points(path: &std::path::Path) -> Result<BTreeMap<String, f64>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|point| Some((point["name"].as_str()?.to_owned(), point["value"].as_f64()?)))
        .collect())
}

/// Runs the benches `options.rounds` times and writes `output.json`: the
/// median of the per-round medians of each.
fn measure(
    options: &Options,
    here: &std::path::Path,
    out: &std::path::Path,
) -> Result<BTreeMap<String, f64>, String> {
    let exes = bench_ab::build(here, &out.join("target"))?;
    let mut samples: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for index in 0..options.rounds {
        let home = out.join("rounds").join(index.to_string());
        let _ = std::fs::remove_dir_all(&home);
        for (id, median) in bench_ab::round(&exes, here, &home, None)? {
            samples.entry(id).or_default().push(median);
        }
    }
    let tonight: BTreeMap<String, f64> = samples
        .iter()
        .filter_map(|(id, values)| Some((id.clone(), bench_ab::median(values)?)))
        .collect();
    let points: Vec<Point> = tonight
        .iter()
        .map(|(name, &value)| Point {
            name: name.clone(),
            unit: "ns",
            value,
        })
        .collect();
    let json = serde_json::to_string_pretty(&points).map_err(|e| e.to_string())?;
    std::fs::write(out.join("output.json"), json).map_err(|e| e.to_string())?;
    Ok(tonight)
}

/// `cargo xtask bench-trend`: writes `target/bench-trend/output.json` and
/// `report.md`.
///
/// # Errors
///
/// Returns the report as the error when a bench alerts, after the output
/// is written, or the error of a failed step.
pub fn run(args: &[String]) -> Result<String, String> {
    let options = parse(args)?;
    let here = std::env::current_dir().map_err(|e| e.to_string())?;
    let out = here.join("target").join("bench-trend");
    let tonight = if let Some(path) = &options.points {
        std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
        read_points(path)?
    } else {
        measure(&options, &here, &out)?
    };

    let earlier = match &options.history {
        Some(path) if path.is_file() => history(
            &std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?,
            &options.entry,
        )?,
        _ => BTreeMap::new(),
    };
    let mut rows = compare(&tonight, &earlier);
    if let (Some(base), None) = (&options.confirm_base, &options.points) {
        confirm(&mut rows, base, options.rounds)?;
    }
    let shape = format!(
        "{} {} with {} logical CPUs",
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::thread::available_parallelism().map_or(0, std::num::NonZero::get)
    );
    let rounds = if options.points.is_some() {
        0
    } else {
        options.rounds
    };
    let report = render(&rows, rounds, &shape);
    std::fs::write(out.join("report.md"), &report).map_err(|e| e.to_string())?;
    if let Some(summary) = std::env::var_os("GITHUB_STEP_SUMMARY") {
        let mut text = std::fs::read_to_string(&summary).unwrap_or_default();
        text.push_str(&report);
        std::fs::write(&summary, text).map_err(|e| e.to_string())?;
    }
    if rows.iter().any(|row| row.trend == Trend::Alert) {
        Err(report)
    } else {
        Ok(report)
    }
}

#[cfg(test)]
#[expect(
    clippy::indexing_slicing,
    reason = "tests index values of known size; a panic is a test failure"
)]
mod tests {
    use std::collections::BTreeMap;

    use super::{Trend, compare, history, parse, render};

    const DATA_JS: &str = r#"window.BENCHMARK_DATA = {
  "lastUpdate": 1,
  "repoUrl": "https://github.com/nobuddyorg/PeekBuddy",
  "entries": {
    "benches (windows-2025)": [
      {"commit": {}, "date": 1, "tool": "customSmallerIsBetter",
       "benches": [{"name": "a/x", "value": 10, "unit": "ns"}, {"name": "a/y", "value": 5, "unit": "ns"}]},
      {"commit": {}, "date": 2, "tool": "customSmallerIsBetter",
       "benches": [{"name": "a/x", "value": 12, "unit": "ns"}]}
    ],
    "other": [{"benches": [{"name": "a/x", "value": 999, "unit": "ns"}]}]
  }
}"#;

    #[test]
    fn history_reads_one_entry_oldest_first() {
        let earlier = history(DATA_JS, "benches (windows-2025)").unwrap();
        assert_eq!(earlier["a/x"], [10.0, 12.0]);
        assert_eq!(earlier["a/y"], [5.0]);
        assert_eq!(earlier.len(), 2);
        assert!(history(DATA_JS, "missing").unwrap().is_empty());
        assert!(history("", "x").unwrap().is_empty());
        assert!(history("window.BENCHMARK_DATA = {broken", "x").is_err());
        // A trailing semicolon is fine.
        let with_semicolon = format!("{DATA_JS};\n");
        assert_eq!(history(&with_semicolon, "other").unwrap()["a/x"], [999.0]);
    }

    fn tonight(list: &[(&str, f64)]) -> BTreeMap<String, f64> {
        list.iter().map(|(n, v)| ((*n).to_owned(), *v)).collect()
    }

    fn earlier(list: &[(&str, &[f64])]) -> BTreeMap<String, Vec<f64>> {
        list.iter()
            .map(|(n, v)| ((*n).to_owned(), v.to_vec()))
            .collect()
    }

    #[test]
    fn alerts_use_the_median_of_the_last_seven_points() {
        // The first two points are outside the window: the median of the
        // last seven is 10, not lowered by the old 1s.
        let points: &[f64] = &[1.0, 1.0, 10.0, 10.0, 9.0, 11.0, 10.0, 10.0, 10.0];
        let rows = compare(
            &tonight(&[("a/ok", 12.5), ("a/hot", 12.6)]),
            &earlier(&[("a/ok", points), ("a/hot", points)]),
        );
        assert_eq!(rows[0].name, "a/hot");
        assert_eq!(rows[0].baseline, Some(10.0));
        // One night over is only watched; exactly +25 % is fine.
        assert_eq!(rows[0].trend, Trend::Watch);
        assert_eq!(rows[1].trend, Trend::Ok);
    }

    #[test]
    fn two_nights_in_a_row_over_the_median_alert() {
        // 13 is over +25 % of the median of the seven 10s before it; tonight
        // is over +25 % of the median of the last seven, which is still 10.
        let points: &[f64] = &[10.0, 10.0, 10.0, 10.0, 10.0, 10.0, 10.0, 13.0];
        let rows = compare(
            &tonight(&[("a/hot", 13.0), ("a/cool", 10.0)]),
            &earlier(&[("a/hot", points), ("a/cool", points)]),
        );
        assert_eq!(rows[0].name, "a/cool");
        assert_eq!(rows[0].trend, Trend::Ok);
        assert_eq!(rows[1].trend, Trend::Alert);
    }

    #[test]
    fn a_slow_night_between_fast_ones_does_not_alert() {
        // The hosted runners alternate between two speeds (#403): 6.4 after
        // 2.2 and 2.1 is watched, not an alert.
        let points: &[f64] = &[6.0, 6.0, 4.3, 2.1, 2.2, 6.4, 2.2];
        let rows = compare(&tonight(&[("a/x", 6.4)]), &earlier(&[("a/x", points)]));
        assert_eq!(rows[0].trend, Trend::Watch);
    }

    #[test]
    fn short_histories_have_no_baseline() {
        let rows = compare(
            &tonight(&[("a/new", 100.0), ("a/two", 100.0), ("a/three", 100.0)]),
            &earlier(&[("a/two", &[1.0, 1.0]), ("a/three", &[1.0, 1.0, 1.0])]),
        );
        let trends: Vec<(&str, Trend)> = rows.iter().map(|r| (r.name.as_str(), r.trend)).collect();
        assert_eq!(
            trends,
            [
                ("a/new", Trend::NoBaseline),
                ("a/three", Trend::Watch),
                ("a/two", Trend::NoBaseline)
            ]
        );
    }

    #[test]
    fn the_report_marks_alerts() {
        let rows = compare(
            &tonight(&[("a/x", 2_000.0), ("a/y", 5.0)]),
            &earlier(&[("a/x", &[1_000.0, 1_000.0, 1_000.0, 1_000.0, 2_000.0])]),
        );
        let report = render(&rows, 10, "test");
        assert!(
            report.contains(
                "| `a/x` | 2.00 us | 1.00 us | +100.0 % | **ALERT** (> +25 %, two nights running) |"
            ),
            "{report}"
        );
        let watched = render(
            &compare(
                &tonight(&[("a/w", 2_000.0)]),
                &earlier(&[("a/w", &[1_000.0; 4])]),
            ),
            10,
            "test",
        );
        assert!(
            watched.contains("| high tonight (alerts if tomorrow is too) |"),
            "{watched}"
        );
        assert!(
            report.contains("| `a/y` | 5.00 ns | - | - | no baseline yet |"),
            "{report}"
        );
    }

    #[test]
    fn points_come_from_a_custom_smaller_is_better_file() {
        let dir = std::env::temp_dir().join(format!("pb-trend-points-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("output.json");
        std::fs::write(
            &file,
            r#"[{"name":"q/a","unit":"ns","value":5.5},{"name":"q/b","unit":"ns","value":7},{"unit":"ns"}]"#,
        )
        .unwrap();
        let read = super::read_points(&file).unwrap();
        let missing = super::read_points(&dir.join("missing.json"));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(read.len(), 2);
        assert_eq!(read["q/a"], 5.5);
        assert_eq!(read["q/b"], 7.0);
        assert!(missing.is_err());
    }

    #[test]
    fn an_alert_the_paired_run_does_not_confirm_is_named_so() {
        let mut rows = compare(
            &tonight(&[("a/x", 2_000.0)]),
            &earlier(&[("a/x", &[1_000.0, 1_000.0, 1_000.0, 1_000.0, 2_000.0])]),
        );
        assert_eq!(rows[0].trend, Trend::Alert);
        rows[0].trend = Trend::NotConfirmed;
        rows[0].ab_ratio = Some(1.02);
        let report = render(&rows, 10, "test");
        assert!(
            report.contains(
                "high on the history, not slower in the same-runner A/B (+2.0 % vs the base)"
            ),
            "{report}"
        );
    }

    #[test]
    fn a_report_without_rounds_names_the_source() {
        let text = render(&[], 0, "shape");
        assert!(
            text.contains("medians as the bench job reported them"),
            "{text}"
        );
        assert!(render(&[], 10, "shape").contains("10 rounds"));
    }

    #[test]
    fn options() {
        let args = |list: &[&str]| list.iter().map(ToString::to_string).collect::<Vec<_>>();
        let defaults = parse(&[]).unwrap();
        assert_eq!(defaults.rounds, 10);
        assert_eq!(defaults.entry, "benches (windows-2025)");
        let set = parse(&args(&[
            "--rounds",
            "3",
            "--history",
            "d.js",
            "--entry",
            "e",
        ]))
        .unwrap();
        assert_eq!(set.rounds, 3);
        assert_eq!(set.history.unwrap().to_str(), Some("d.js"));
        assert_eq!(set.entry, "e");
        assert_eq!(set.points, None);
        assert_eq!(set.confirm_base, None);
        let confirm = parse(&args(&["--confirm-base", "abc123"])).unwrap();
        assert_eq!(confirm.confirm_base.as_deref(), Some("abc123"));
        let points = parse(&args(&["--points", "o.json"])).unwrap();
        assert_eq!(points.points.unwrap().to_str(), Some("o.json"));
        for bad in [&["--rounds", "0"][..], &["--rounds"], &["--x"]] {
            assert!(parse(&args(bad)).is_err(), "{bad:?}");
        }
    }
}
