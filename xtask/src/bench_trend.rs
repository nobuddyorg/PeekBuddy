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
    /// More than +25 % above the rolling median.
    Alert,
    /// Fewer than [`MIN_POINTS`] earlier points.
    NoBaseline,
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
/// [`WINDOW`] earlier points.
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
                Some(base) if value > base * ALERT_RATIO => Trend::Alert,
                Some(_) => Trend::Ok,
            };
            Row {
                name: name.clone(),
                value,
                baseline,
                trend,
            }
        })
        .collect()
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
    let _ = writeln!(
        out,
        "{rounds} rounds, median of per-round medians; alert above +25 % of the \
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
            Trend::Alert => "**ALERT** (> +25 %)",
            Trend::NoBaseline => "no baseline yet",
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
            other => {
                return Err(format!(
                    "unknown option {other}; usage: cargo xtask bench-trend [--rounds <n>] \
                     [--history <data.js>] [--entry <name>]"
                ));
            }
        }
    }
    if options.rounds < 1 {
        return Err("--rounds must be at least 1".to_owned());
    }
    Ok(options)
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
    let exes = bench_ab::build(&here, &out.join("target"))?;
    let mut samples: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for index in 0..options.rounds {
        let home = out.join("rounds").join(index.to_string());
        let _ = std::fs::remove_dir_all(&home);
        for (id, median) in bench_ab::round(&exes, &here, &home, None)? {
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

    let earlier = match &options.history {
        Some(path) if path.is_file() => history(
            &std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?,
            &options.entry,
        )?,
        _ => BTreeMap::new(),
    };
    let rows = compare(&tonight, &earlier);
    let shape = format!(
        "{} {} with {} logical CPUs",
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::thread::available_parallelism().map_or(0, std::num::NonZero::get)
    );
    let report = render(&rows, options.rounds, &shape);
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
        assert_eq!(rows[0].trend, Trend::Alert);
        // Exactly +25 % is not an alert.
        assert_eq!(rows[1].trend, Trend::Ok);
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
                ("a/three", Trend::Alert),
                ("a/two", Trend::NoBaseline)
            ]
        );
    }

    #[test]
    fn the_report_marks_alerts() {
        let rows = compare(
            &tonight(&[("a/x", 2_000.0), ("a/y", 5.0)]),
            &earlier(&[("a/x", &[1_000.0; 3])]),
        );
        let report = render(&rows, 10, "test");
        assert!(
            report.contains("| `a/x` | 2.00 us | 1.00 us | +100.0 % | **ALERT** (> +25 %) |"),
            "{report}"
        );
        assert!(
            report.contains("| `a/y` | 5.00 ns | - | - | no baseline yet |"),
            "{report}"
        );
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
        for bad in [&["--rounds", "0"][..], &["--rounds"], &["--x"]] {
            assert!(parse(&args(bad)).is_err(), "{bad:?}");
        }
    }
}
