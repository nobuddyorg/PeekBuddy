//! `cargo xtask check`: the local loop an agent or contributor runs on Linux
//! before every push (#86). Windows CI stays the test oracle.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

/// Windows targets that clippy checks from Linux.
pub const WINDOWS_TARGETS: &[&str] = &["x86_64-pc-windows-msvc", "aarch64-pc-windows-msvc"];

/// One step of the loop: a program and its arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// Short name shown in the summary.
    pub name: String,
    /// Program to run.
    pub program: String,
    /// Arguments.
    pub args: Vec<String>,
    /// Extra environment variables.
    pub env: Vec<(String, String)>,
}

impl Step {
    fn new(name: &str, program: &str, args: &[&str]) -> Self {
        Self {
            name: name.to_owned(),
            program: program.to_owned(),
            args: args.iter().map(|&arg| arg.to_owned()).collect(),
            env: Vec::new(),
        }
    }

    fn with_env(mut self, key: &str, value: &str) -> Self {
        self.env.push((key.to_owned(), value.to_owned()));
        self
    }
}

/// What the host has installed, so the plan can be computed without running
/// anything.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent tool-availability flags"
)]
pub struct Host {
    /// `cargo nextest` is installed.
    pub nextest: bool,
    /// `cargo xwin` should be used for the Windows targets.
    pub xwin: bool,
    /// `.pre-commit-config.yaml` exists.
    pub prek_config: bool,
    /// `typos` is installed.
    pub typos: bool,
}

/// Returns the steps `cargo xtask check` runs on `host`, in order.
pub fn plan(host: Host) -> Vec<Step> {
    let mut steps = vec![Step::new("fmt", "cargo", &["fmt", "--all", "--check"])];
    if host.prek_config {
        // Clippy runs below per Windows target; the branch check is for commits.
        steps.push(
            Step::new("prek", "prek", &["run", "--all-files"])
                .with_env("SKIP", "no-commit-to-branch,cargo-clippy"),
        );
    } else if host.typos {
        steps.push(Step::new("typos", "typos", &[]));
    }
    steps.push(if host.nextest {
        Step::new(
            "test (linux)",
            "cargo",
            &[
                "nextest",
                "run",
                "--workspace",
                "--locked",
                "--no-tests=pass",
            ],
        )
    } else {
        Step::new(
            "test (linux)",
            "cargo",
            &["test", "--workspace", "--locked"],
        )
    });
    steps.push(Step::new(
        "doctest (linux)",
        "cargo",
        &["test", "--workspace", "--locked", "--doc"],
    ));
    steps.push(Step::new(
        "check-unsafe",
        "cargo",
        &["xtask", "check-unsafe"],
    ));
    steps.push(Step::new("check-deps", "cargo", &["xtask", "check-deps"]));
    steps.push(Step::new(
        "budgets",
        "cargo",
        &["xtask", "budgets", "--check"],
    ));
    for target in WINDOWS_TARGETS {
        let mut args = Vec::new();
        if host.xwin {
            args.push("xwin");
        }
        args.extend([
            "clippy",
            "--workspace",
            "--all-targets",
            "--all-features",
            "--locked",
            "--target",
            target,
        ]);
        steps.push(
            Step::new(&format!("clippy ({target})"), "cargo", &args)
                .with_env("CARGO_BUILD_WARNINGS", "deny"),
        );
    }
    steps
}

fn installed(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .output()
        .is_ok_and(|output| output.status.success())
}

fn detect_host() -> Host {
    Host {
        nextest: installed("cargo", &["nextest", "--version"]),
        xwin: std::env::var_os("PEEKBUDDY_XWIN").is_some_and(|value| value == "1"),
        prek_config: Path::new(".pre-commit-config.yaml").exists(),
        typos: installed("typos", &["--version"]),
    }
}

pub fn run() -> Result<String, String> {
    let host = detect_host();
    let mut stdout = std::io::stdout();
    let mut timings: Vec<(String, Duration)> = Vec::new();
    let started = Instant::now();
    for step in plan(host) {
        let _ = writeln!(stdout, "==> {}", step.name);
        let step_started = Instant::now();
        let status = Command::new(&step.program)
            .args(&step.args)
            .envs(step.env.iter().map(|(key, value)| (key, value)))
            .status()
            .map_err(|error| format!("{}: cannot run {}: {error}", step.name, step.program))?;
        timings.push((step.name.clone(), step_started.elapsed()));
        if !status.success() {
            return Err(format!("check failed at step `{}`", step.name));
        }
    }
    Ok(summary(&timings, started.elapsed()))
}

fn summary(timings: &[(String, Duration)], total: Duration) -> String {
    let mut text = String::from("check: ok\n");
    for (name, duration) in timings {
        let _ = writeln!(text, "  {name:<40} {:>7.1} s", duration.as_secs_f64());
    }
    let _ = write!(text, "  {:<40} {:>7.1} s", "total", total.as_secs_f64());
    text
}

#[cfg(test)]
mod tests {
    use super::{Host, WINDOWS_TARGETS, plan};

    fn names(host: Host) -> Vec<String> {
        plan(host).into_iter().map(|step| step.name).collect()
    }

    #[test]
    fn clippy_runs_for_both_windows_targets_with_warnings_denied() {
        let steps = plan(Host::default());
        for target in WINDOWS_TARGETS {
            let step = steps
                .iter()
                .find(|step| step.args.iter().any(|arg| arg == target))
                .unwrap();
            assert_eq!(step.args[0], "clippy");
            assert!(
                step.env
                    .contains(&("CARGO_BUILD_WARNINGS".to_owned(), "deny".to_owned()))
            );
        }
    }

    #[test]
    fn xwin_wraps_clippy_when_enabled() {
        let host = Host {
            xwin: true,
            ..Host::default()
        };
        let steps = plan(host);
        let clippy: Vec<_> = steps
            .iter()
            .filter(|step| step.name.starts_with("clippy"))
            .collect();
        assert_eq!(clippy.len(), 2);
        assert!(clippy.iter().all(|step| step.args[0] == "xwin"));
    }

    #[test]
    fn prek_replaces_standalone_typos() {
        let host = Host {
            prek_config: true,
            typos: true,
            ..Host::default()
        };
        let steps = names(host);
        assert!(steps.contains(&"prek".to_owned()));
        assert!(!steps.contains(&"typos".to_owned()));
    }

    #[test]
    fn nextest_is_used_when_installed() {
        let host = Host {
            nextest: true,
            ..Host::default()
        };
        let test = plan(host)
            .into_iter()
            .find(|step| step.name == "test (linux)")
            .unwrap();
        assert_eq!(test.args[0], "nextest");
    }

    #[test]
    fn fmt_runs_first() {
        assert_eq!(names(Host::default())[0], "fmt");
    }
}
