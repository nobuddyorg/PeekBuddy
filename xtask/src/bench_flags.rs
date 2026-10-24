//! `cargo xtask bench-flags --flags "<rustflags>" [--rounds n] [--filter re]`:
//! the same-runner A/B of one source tree built with and without extra
//! `RUSTFLAGS` (#28): the x86-64-v2 CPU baseline and the PGO experiment.
//!
//! Reuses the `bench-ab` machinery. The report is informational: whether a
//! flag is adopted is a decision recorded in `PERFORMANCE.md`, not a gate.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::bench_ab::{build, build_with, compare, render, round};
use crate::budgets;

/// Options of `bench-flags`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// `RUSTFLAGS` of the "head" side; the "base" side uses none.
    pub flags: String,
    /// Rounds per side.
    pub rounds: usize,
    /// Criterion filter.
    pub filter: Option<String>,
}

/// Parses `--flags <rustflags> [--rounds <n>] [--filter <regex>]`.
///
/// # Errors
///
/// Returns a usage message for unknown options, bad values or missing flags.
pub fn parse(args: &[String]) -> Result<Options, String> {
    let usage =
        "usage: cargo xtask bench-flags --flags <rustflags> [--rounds <n>] [--filter <regex>]";
    let mut flags = None;
    let mut rounds = 10;
    let mut filter = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next()
                .cloned()
                .ok_or_else(|| format!("{arg} needs a value; {usage}"))
        };
        match arg.as_str() {
            "--flags" => flags = Some(value()?),
            "--rounds" => rounds = value()?.parse().map_err(|e| format!("--rounds: {e}"))?,
            "--filter" => filter = Some(value()?),
            other => return Err(format!("unknown option {other}; {usage}")),
        }
    }
    if rounds < 1 {
        return Err("--rounds must be at least 1".to_owned());
    }
    Ok(Options {
        flags: flags.ok_or_else(|| usage.to_owned())?,
        rounds,
        filter,
    })
}

/// `cargo xtask bench-flags`.
///
/// # Errors
///
/// Returns the error of a failed step.
pub fn run(args: &[String]) -> Result<String, String> {
    let options = parse(args)?;
    let here = std::env::current_dir().map_err(|e| e.to_string())?;
    let out = here.join("target").join("bench-flags");
    let plain = build(&here, &out.join("plain-target"))?;
    let flagged = build_with(&here, &out.join("flags-target"), Some(&options.flags))?;
    let mut base: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    let mut head: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for index in 0..options.rounds {
        let order = if index % 2 == 0 {
            [false, true]
        } else {
            [true, false]
        };
        for is_head in order {
            let (exes, side, samples): (&Vec<PathBuf>, _, _) = if is_head {
                (&flagged, "flags", &mut head)
            } else {
                (&plain, "plain", &mut base)
            };
            let home = out.join("rounds").join(side).join(index.to_string());
            let _ = std::fs::remove_dir_all(&home);
            for (id, median) in round(exes, &here, &home, options.filter.as_deref())? {
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
        "{} {}, base = default flags, head = `{}`",
        std::env::consts::OS,
        std::env::consts::ARCH,
        options.flags
    );
    let report = render(&rows, options.rounds, &shape).replacen(
        "## Bench A/B (same runner)",
        "## Bench A/B of build flags (informational, no gate)",
        1,
    );
    std::fs::write(out.join("report.md"), &report).map_err(|e| e.to_string())?;
    if let Some(summary) = std::env::var_os("GITHUB_STEP_SUMMARY") {
        let mut text = std::fs::read_to_string(&summary).unwrap_or_default();
        text.push_str(&report);
        std::fs::write(&summary, text).map_err(|e| e.to_string())?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::{Options, parse};

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn flags_are_required() {
        assert!(parse(&args(&[])).is_err());
        assert!(parse(&args(&["--rounds", "3"])).is_err());
    }

    #[test]
    fn options_are_parsed() {
        let options = parse(&args(&[
            "--flags",
            "-Ctarget-cpu=x86-64-v2",
            "--rounds",
            "4",
            "--filter",
            "parse",
        ]))
        .unwrap();
        assert_eq!(
            options,
            Options {
                flags: "-Ctarget-cpu=x86-64-v2".to_owned(),
                rounds: 4,
                filter: Some("parse".to_owned()),
            }
        );
        assert!(parse(&args(&["--flags", "x", "--rounds", "0"])).is_err());
        assert!(parse(&args(&["--bogus"])).is_err());
    }
}
