//! `peekbuddy.exe`: one exe with two roles (ADR 0001).
//!
//! Without arguments it runs the engine; with `--viewer` it runs the viewer.
//! The exe uses the GUI subsystem: there is no console, and a refused command
//! line is reported through the exit code (2).
#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]
#![forbid(unsafe_code)]

use std::process::ExitCode;

use peekbuddy_core::perf::MilestoneTable;
use peekbuddy_engine::Provider;
use peekbuddy_engine::diagnostics::Facts;

mod cli;
#[cfg(feature = "profile-tracy")]
mod tracy;

use cli::{Invocation, Role};

/// Mutex of the viewer role; the engine's is in `peekbuddy_engine`.
#[cfg(windows)]
const VIEWER_MUTEX: &str = r"Local\PeekBuddy.{5e7d2b90-3a14-4c68-8f0b-d1a9c6e4f723}";

/// Records `ProcessStart` for `role` (F7: measured from process creation).
#[cfg(windows)]
fn process_start(role: &Role, table: &MilestoneTable) {
    use peekbuddy_core::perf::{Milestone, Role as PerfRole};
    use peekbuddy_win::perf::{record, since_process_create_us};

    let role = match role {
        Role::Engine | Role::Diagnostics => PerfRole::Engine,
        Role::Viewer => PerfRole::Viewer,
    };
    record(
        table,
        Milestone::ProcessStart,
        role,
        since_process_create_us(),
    );
}

#[cfg(not(windows))]
fn process_start(_role: &Role, _table: &MilestoneTable) {}

/// Runs the viewer unless another viewer already runs in this session.
#[cfg(windows)]
fn run_viewer() -> std::io::Result<()> {
    use peekbuddy_win::single_instance::SingleInstance;

    if let Some(_instance) = SingleInstance::acquire(VIEWER_MUTEX)? {
        peekbuddy_viewer::run();
    }
    Ok(())
}

#[cfg(not(windows))]
fn run_viewer() -> std::io::Result<()> {
    peekbuddy_viewer::run();
    Ok(())
}

/// What the build script recorded: version, git SHA, compiler, key
/// dependency versions.
fn facts() -> Facts {
    Facts {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        git_sha: option_env!("PEEKBUDDY_GIT_SHA")
            .unwrap_or("unknown")
            .to_owned(),
        rustc: option_env!("PEEKBUDDY_RUSTC")
            .unwrap_or("unknown")
            .to_owned(),
        dependencies: Facts::parse_dependencies(option_env!("PEEKBUDDY_DEPS").unwrap_or("")),
    }
}

/// Prints the diagnostics text to standard output.
#[cfg(windows)]
fn run_diagnostics(full: bool) -> std::io::Result<()> {
    use std::io::Write as _;

    peekbuddy_win::console::attach_output();
    let text = peekbuddy_engine::diag_collect::collect(facts(), full).render();
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(text.as_bytes())?;
    stdout.flush()
}

#[cfg(not(windows))]
fn run_diagnostics(_full: bool) -> std::io::Result<()> {
    Ok(())
}

/// Runs the invoked role to completion; in profiling builds inside one Tracy
/// zone.
fn run(invocation: &Invocation, table: &MilestoneTable, provider: Provider) -> std::io::Result<()> {
    #[cfg(feature = "profile-tracy")]
    let _zone = tracy::start(&invocation.role);
    match invocation.role {
        Role::Engine => peekbuddy_engine::run(&invocation.engine, table, provider).map(drop),
        Role::Viewer => run_viewer(),
        Role::Diagnostics => run_diagnostics(invocation.full),
    }
}

fn main() -> ExitCode {
    let table = MilestoneTable::new();
    // Registered first, so the very first milestone reaches ETW listeners.
    let provider = peekbuddy_engine::register_provider();
    let Ok(invocation) = cli::parse(std::env::args().skip(1)) else {
        return ExitCode::from(2);
    };
    process_start(&invocation.role, &table);
    match run(&invocation, &table, provider) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

#[cfg(test)]
mod tests {
    use peekbuddy_core::perf::MilestoneTable;

    use super::{Invocation, Role, run};

    #[test]
    fn the_viewer_role_runs_to_completion() {
        let invocation = Invocation {
            role: Role::Viewer,
            engine: peekbuddy_engine::Options::default(),
            full: false,
        };
        run(
            &invocation,
            &MilestoneTable::new(),
            peekbuddy_engine::register_provider(),
        )
        .unwrap();
        #[cfg(feature = "profile-tracy")]
        assert!(tracy_client::Client::is_running());
    }
}
