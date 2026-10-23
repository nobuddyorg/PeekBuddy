//! Repository automation for PeekBuddy, run as `cargo xtask <command>`.
#![forbid(unsafe_code)]

mod budgets;
mod check;
mod deps;
mod dist;
mod metadata;
mod unsafe_code;

use std::io::Write as _;
use std::process::ExitCode;

const USAGE: &str = "usage: cargo xtask <command>

commands:
  budgets        check (or --write) the PERFORMANCE.md table against perf/budgets.toml
  check          the local loop: fmt, prek, Linux tests, Windows clippy
  check-unsafe   every crate forbids unsafe code except the allow-list
  check-deps     engine crates do not depend on GPU or UI crates
  dist-extras    third-party licences and CycloneDX SBOM for the release";

fn main() -> ExitCode {
    let command = std::env::args().nth(1);
    let option = std::env::args().nth(2);
    let result = match command.as_deref() {
        Some("budgets") => budgets::run(option.as_deref()),
        Some("check") => check::run(),
        Some("check-unsafe") => unsafe_code::run(),
        Some("check-deps") => deps::run(),
        Some("dist-extras") => dist::run(),
        _ => Err(USAGE.to_owned()),
    };
    match result {
        Ok(message) => {
            let _ = writeln!(std::io::stdout(), "{message}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            let _ = writeln!(std::io::stderr(), "{message}");
            ExitCode::FAILURE
        }
    }
}
