//! `peekbuddy.exe`: one exe with two roles (ADR 0001).
//!
//! Without arguments it runs the engine; with `--viewer` it runs the viewer.
//! The exe uses the GUI subsystem: there is no console, and a refused command
//! line is reported through the exit code (2).
#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]
#![forbid(unsafe_code)]

#[cfg(not(windows))]
use std::path::PathBuf;
use std::process::ExitCode;

use std::sync::Arc;

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

/// The event a second viewer launch raises to bring the first one forward.
#[cfg(windows)]
const VIEWER_EVENT: &str = r"Local\PeekBuddy.Viewer.Show.{5e7d2b90-3a14-4c68-8f0b-d1a9c6e4f723}";

/// Records `ProcessStart` for `role` (F7: measured from process creation).
#[cfg(windows)]
fn process_start(role: &Role, table: &MilestoneTable) {
    use peekbuddy_core::perf::{Milestone, Role as PerfRole};
    use peekbuddy_win::perf::{record, since_process_create_us};

    let role = match role {
        Role::Engine | Role::Diagnostics | Role::Ping => PerfRole::Engine,
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

/// Runs the viewer unless another viewer already runs in this session; that
/// one is asked to come to the front instead.
///
/// With `--data-dir` (tests, the perf harness) the viewer belongs to the
/// isolated engine of that directory: its pipe, mutex and event carry the
/// same hash, so it never meets a real viewer.
#[cfg(windows)]
fn run_viewer(invocation: &Invocation, table: &Arc<MilestoneTable>) -> std::io::Result<()> {
    use peekbuddy_engine::{datadir, ipc_server};
    use peekbuddy_win::single_instance::SingleInstance;

    let exe = std::env::current_exe()?;
    let (root, pipe_name, engine_args, suffix, title) = match invocation.engine.data_dir.as_deref()
    {
        Some(dir) => {
            let hash = peekbuddy_win::pipe::stable_hash(&dir.to_string_lossy().to_lowercase());
            (
                dir.to_path_buf(),
                ipc_server::pipe_name(Some(dir))?,
                vec!["--headless".into(), "--data-dir".into(), dir.into()],
                format!(".{hash:016x}"),
                peekbuddy_viewer::isolated_title(hash),
            )
        }
        None => (
            datadir::locate(&exe)?.0,
            ipc_server::pipe_name(None)?,
            Vec::new(),
            String::new(),
            String::from("PeekBuddy"),
        ),
    };
    let show_event = format!("{VIEWER_EVENT}{suffix}");
    let Some(_instance) = SingleInstance::acquire(&format!("{VIEWER_MUTEX}{suffix}"))? else {
        // A viewer is open: let it take the foreground, then leave. Failing
        // to reach it is not worth an error; the user clicked twice.
        peekbuddy_win::chrome::allow_any_foreground();
        let _ = peekbuddy_win::signal::Signal::open(&show_event).and_then(|event| event.raise());
        return Ok(());
    };
    peekbuddy_viewer::run(peekbuddy_viewer::Options {
        segments: root.join("segments"),
        data_dir: root,
        pipe_name,
        exe,
        engine_args,
        show_event,
        title,
        table: Arc::clone(table),
        probe: invocation.viewer_probe.clone(),
    })
}

#[cfg(not(windows))]
fn run_viewer(invocation: &Invocation, table: &Arc<MilestoneTable>) -> std::io::Result<()> {
    peekbuddy_viewer::run(peekbuddy_viewer::Options {
        data_dir: PathBuf::new(),
        segments: PathBuf::new(),
        pipe_name: String::new(),
        exe: PathBuf::new(),
        engine_args: Vec::new(),
        show_event: String::new(),
        title: String::new(),
        table: Arc::clone(table),
        probe: invocation.viewer_probe.clone(),
    })
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

/// Connects to the engine pipe: the exit code is 0 or the Windows error.
#[cfg(windows)]
fn run_ping(options: &peekbuddy_engine::Options) -> std::io::Result<()> {
    let name = peekbuddy_engine::ipc_server::pipe_name(options.data_dir.as_deref())?;
    peekbuddy_engine::ipc_server::ping(&name, &std::env::current_exe()?)
}

#[cfg(not(windows))]
fn run_ping(_options: &peekbuddy_engine::Options) -> std::io::Result<()> {
    Ok(())
}

/// The exit code for a failed run: the Windows error code (clamped to a
/// byte) so `--ipc-ping` can say "access denied" (5), otherwise 1.
fn failure_code(error: &std::io::Error) -> u8 {
    error
        .raw_os_error()
        .and_then(|code| u8::try_from(code).ok())
        .filter(|code| *code != 0)
        .unwrap_or(1)
}

/// Runs the invoked role to completion; in profiling builds inside one Tracy
/// zone.
fn run(
    invocation: &Invocation,
    table: &Arc<MilestoneTable>,
    provider: Provider,
) -> std::io::Result<()> {
    #[cfg(feature = "profile-tracy")]
    let _zone = tracy::start(&invocation.role);
    match invocation.role {
        Role::Engine => peekbuddy_engine::run(&invocation.engine, table, provider).map(drop),
        Role::Viewer => run_viewer(invocation, table),
        Role::Diagnostics => run_diagnostics(invocation.full),
        Role::Ping => run_ping(&invocation.engine),
    }
}

fn main() -> ExitCode {
    let table = Arc::new(MilestoneTable::new());
    // Registered first, so the very first milestone reaches ETW listeners.
    let provider = peekbuddy_engine::register_provider();
    let Ok(invocation) = cli::parse(std::env::args().skip(1)) else {
        return ExitCode::from(2);
    };
    process_start(&invocation.role, &table);
    match run(&invocation, &table, provider) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => ExitCode::from(failure_code(&error)),
    }
}

// Off Windows the viewer role returns at once. On Windows it opens a window,
// which winit only allows on the main thread: `tests/viewer.rs` runs the real
// exe instead.
#[cfg(all(test, not(windows)))]
mod tests {
    use peekbuddy_core::perf::MilestoneTable;

    use super::{Invocation, Role, run};

    #[test]
    fn the_viewer_role_runs_to_completion() {
        let invocation = Invocation {
            role: Role::Viewer,
            engine: peekbuddy_engine::Options::default(),
            full: false,
            viewer_probe: None,
        };
        run(
            &invocation,
            &std::sync::Arc::new(MilestoneTable::new()),
            peekbuddy_engine::register_provider(),
        )
        .unwrap();
        #[cfg(feature = "profile-tracy")]
        assert!(tracy_client::Client::is_running());
    }
}
