//! `peekbuddy.exe`: one exe with two roles (ADR 0001).
//!
//! Without arguments it runs the engine; with `--viewer` it runs the viewer.
//! The exe uses the GUI subsystem: there is no console, and a refused command
//! line is reported through the exit code (2).
#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]
#![forbid(unsafe_code)]

use std::process::ExitCode;

use peekbuddy_core::perf::MilestoneTable;

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
        Role::Engine => PerfRole::Engine,
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

/// Runs the invoked role to completion; in profiling builds inside one Tracy
/// zone.
fn run(invocation: &Invocation, table: &MilestoneTable) -> std::io::Result<()> {
    #[cfg(feature = "profile-tracy")]
    let _zone = tracy::start(&invocation.role);
    match invocation.role {
        Role::Engine => peekbuddy_engine::run(&invocation.engine, table).map(drop),
        Role::Viewer => run_viewer(),
    }
}

fn main() -> ExitCode {
    let table = MilestoneTable::new();
    let Ok(invocation) = cli::parse(std::env::args().skip(1)) else {
        return ExitCode::from(2);
    };
    process_start(&invocation.role, &table);
    match run(&invocation, &table) {
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
        };
        run(&invocation, &MilestoneTable::new()).unwrap();
        #[cfg(feature = "profile-tracy")]
        assert!(tracy_client::Client::is_running());
    }
}
