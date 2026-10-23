//! `cargo xtask size <exe>...`: binary-size gate (#28, M14).
//!
//! Each exe is checked against the `binary.*` rows of `perf/budgets.toml`:
//! a `pr-strict` row fails the run when exceeded, a `report-only` row is
//! reported. Sizes are in MB of 1,000,000 bytes, as the rows state.

use std::fmt::Write as _;

use crate::budgets::{self, Budget};

/// One exe checked against one budget row.
#[derive(Debug, Clone, PartialEq)]
pub struct Check {
    /// Exe path as given.
    pub exe: String,
    /// Budget id.
    pub id: String,
    /// Size in MB.
    pub mb: f64,
    /// Limit in MB.
    pub limit: f64,
    /// Whether exceeding the limit fails the run.
    pub strict: bool,
}

impl Check {
    /// `true` when the size is within the limit.
    #[must_use]
    pub fn within(&self) -> bool {
        self.mb <= self.limit
    }
}

/// Bytes to MB (1,000,000 bytes).
#[must_use]
pub fn megabytes(bytes: u64) -> f64 {
    // u64 -> f64 is exact below 2^53 bytes; no exe gets near that.
    #[expect(clippy::cast_precision_loss, reason = "exact below 2^53 bytes")]
    let bytes = bytes as f64;
    bytes / 1_000_000.0
}

/// The checks of an exe of `bytes` against the `binary.*` rows with a limit.
#[must_use]
pub fn checks(exe: &str, bytes: u64, budgets: &[Budget]) -> Vec<Check> {
    budgets
        .iter()
        .filter(|row| row.id.starts_with("binary.") && row.unit == "MB")
        .filter_map(|row| {
            Some(Check {
                exe: exe.to_owned(),
                id: row.id.clone(),
                mb: megabytes(bytes),
                limit: row.limit?,
                strict: row.gate == "pr-strict",
            })
        })
        .collect()
}

/// The Markdown table of `checks`.
#[must_use]
pub fn render(checks: &[Check]) -> String {
    let mut out = String::from(
        "## Binary size\n\n| exe | budget | size | limit | result |\n| --- | --- | --- | --- | --- |\n",
    );
    for check in checks {
        let result = match (check.within(), check.strict) {
            (true, _) => "ok",
            (false, true) => "**FAIL**",
            (false, false) => "over (report only)",
        };
        let _ = writeln!(
            out,
            "| `{}` | `{}` | {:.2} MB | {} MB | {result} |",
            check.exe, check.id, check.mb, check.limit
        );
    }
    out
}

/// `cargo xtask size <exe>...`.
///
/// # Errors
///
/// Returns the report when a strict limit is exceeded, or a usage or I/O
/// error.
pub fn run(args: &[String]) -> Result<String, String> {
    if args.is_empty() {
        return Err("usage: cargo xtask size <exe>...".to_owned());
    }
    let budgets = budgets::load()?;
    let mut all = Vec::new();
    for exe in args {
        let bytes = std::fs::metadata(exe)
            .map_err(|e| format!("{exe}: {e}"))?
            .len();
        all.extend(checks(exe, bytes, &budgets));
    }
    let report = render(&all);
    if let Some(summary) = std::env::var_os("GITHUB_STEP_SUMMARY") {
        let mut text = std::fs::read_to_string(&summary).unwrap_or_default();
        text.push_str(&report);
        std::fs::write(&summary, text).map_err(|e| e.to_string())?;
    }
    if all.iter().any(|check| check.strict && !check.within()) {
        Err(report)
    } else {
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::{Check, checks, megabytes, render};
    use crate::budgets::Budget;

    fn row(id: &str, unit: &str, limit: Option<f64>, gate: &str) -> Budget {
        Budget {
            id: id.to_owned(),
            metric: String::new(),
            unit: unit.to_owned(),
            bound: "max".to_owned(),
            limit,
            class: "deterministic".to_owned(),
            gate: gate.to_owned(),
            status: "provisional".to_owned(),
            source: String::new(),
            workload: String::new(),
            how: String::new(),
            arch: Vec::new(),
        }
    }

    fn rows() -> Vec<Budget> {
        vec![
            row("binary.size", "MB", Some(5.0), "report-only"),
            row("binary.size.cap", "MB", Some(8.0), "pr-strict"),
            row("binary.other", "MB", None, "pr-strict"),
            row("engine.ready.warm", "ms", Some(100.0), "pr-ab"),
        ]
    }

    #[test]
    fn megabytes_are_decimal() {
        assert!((megabytes(5_000_000) - 5.0).abs() < f64::EPSILON);
        assert!((megabytes(0)).abs() < f64::EPSILON);
    }

    #[test]
    fn checks_use_binary_rows_with_limits() {
        let found = checks("a.exe", 6_000_000, &rows());
        let ids: Vec<(&str, bool, bool)> = found
            .iter()
            .map(|c| (c.id.as_str(), c.strict, c.within()))
            .collect();
        assert_eq!(
            ids,
            [
                ("binary.size", false, false),
                ("binary.size.cap", true, true)
            ]
        );
    }

    #[test]
    fn limits_are_inclusive() {
        let at_cap = checks("a.exe", 8_000_000, &rows());
        assert!(at_cap.iter().all(|c| !c.strict || c.within()));
        let over = checks("a.exe", 8_000_001, &rows());
        assert!(over.iter().any(|c| c.strict && !c.within()));
    }

    #[test]
    fn the_report_marks_failures() {
        let report = render(&[
            Check {
                exe: "a.exe".into(),
                id: "binary.size.cap".into(),
                mb: 9.5,
                limit: 8.0,
                strict: true,
            },
            Check {
                exe: "a.exe".into(),
                id: "binary.size".into(),
                mb: 9.5,
                limit: 5.0,
                strict: false,
            },
            Check {
                exe: "b.exe".into(),
                id: "binary.size".into(),
                mb: 1.25,
                limit: 5.0,
                strict: false,
            },
        ]);
        assert!(
            report.contains("| `a.exe` | `binary.size.cap` | 9.50 MB | 8 MB | **FAIL** |"),
            "{report}"
        );
        assert!(
            report.contains("| `a.exe` | `binary.size` | 9.50 MB | 5 MB | over (report only) |"),
            "{report}"
        );
        assert!(
            report.contains("| `b.exe` | `binary.size` | 1.25 MB | 5 MB | ok |"),
            "{report}"
        );
    }
}
