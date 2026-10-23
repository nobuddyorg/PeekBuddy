//! Repository automation for PeekBuddy, run as `cargo xtask <command>`.
#![forbid(unsafe_code)]

mod bench_ab;
mod budgets;
mod check;
mod deps;
mod dist;
mod metadata;
mod size;
mod tracy;
mod unsafe_code;

use std::io::Write as _;
use std::process::ExitCode;

const USAGE: &str = "usage: cargo xtask <command>

commands:
  bench-ab       same-runner A/B of the criterion benches [--base rev] [--rounds n] [--filter re]
  budgets        check (or --write) the PERFORMANCE.md table against perf/budgets.toml
  check          the local loop: fmt, prek, Linux tests, Windows clippy
  check-unsafe   every crate forbids unsafe code except the allow-list
  check-deps     engine crates have no GPU or UI crates; nothing uses PDH
  check-release  no Tracy in the release feature set [--features f] [--exe path]
  dist-extras    third-party licences and CycloneDX SBOM for the release
  size           binary-size gate for release exes against perf/budgets.toml
  profile        --tracy [--alloc] [--role engine|viewer]: Tracy profiling build";

fn main() -> ExitCode {
    let command = std::env::args().nth(1);
    let rest: Vec<String> = std::env::args().skip(2).collect();
    let option = rest.first().cloned();
    let result = match command.as_deref() {
        Some("bench-ab") => bench_ab::run(&rest),
        Some("budgets") => budgets::run(option.as_deref()),
        Some("check") => check::run(),
        Some("check-unsafe") => unsafe_code::run(),
        Some("check-deps") => deps::run(),
        Some("check-release") => tracy::check_release(&rest),
        Some("dist-extras") => dist::run(),
        Some("profile") => tracy::profile(&rest),
        Some("size") => size::run(&rest),
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
