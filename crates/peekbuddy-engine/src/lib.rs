//! The PeekBuddy engine role: tray, watches, reconciliation and the writer.
//!
//! The engine never depends on a GPU or UI stack (ADR 0001); `cargo xtask
//! check-deps` enforces that. This crate holds the process skeleton (#59):
//! single instance, data directory, hidden window and message loop, crash
//! log and the one shutdown path. Tray (#60), pipe (#61), settings (#62) and
//! capture hang off it.
#![forbid(unsafe_code)]

pub mod crash;
pub mod portable;
pub mod shutdown;

#[cfg(windows)]
pub mod datadir;

use std::path::PathBuf;

/// Mutex of the engine role. The GUID is fixed; the viewer has its own.
pub const MUTEX_NAME: &str = r"Local\PeekBuddy.{8a3c1f4e-6d52-4b0e-9f17-2c5a7e90d4b1}";

/// Window class of the engine's hidden window.
pub const WINDOW_CLASS: &str = "PeekBuddy.Engine";

/// The names that make one engine instance unique in a session.
///
/// A normal engine uses the fixed names. A headless engine with an explicit
/// data directory (tests, the perf harness) gets a suffix derived from that
/// directory, so isolated instances can run side by side while two engines
/// on the same directory still collide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Names {
    /// Name of the single-instance mutex.
    pub mutex: String,
    /// Class of the hidden window.
    pub window_class: String,
}

impl Names {
    /// The names for an engine started with `data_dir`.
    #[must_use]
    pub fn new(data_dir: Option<&std::path::Path>) -> Self {
        let Some(dir) = data_dir else {
            return Self {
                mutex: MUTEX_NAME.to_owned(),
                window_class: WINDOW_CLASS.to_owned(),
            };
        };
        // FNV-1a over the lower-cased path: stable across runs and builds.
        let hash = dir
            .to_string_lossy()
            .to_lowercase()
            .bytes()
            .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
                (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
            });
        Self {
            mutex: format!("{MUTEX_NAME}.{hash:016x}"),
            window_class: format!("{WINDOW_CLASS}.{hash:016x}"),
        }
    }
}

/// What the exe was asked to do as the engine.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Options {
    /// No tray; for tests and the perf harness.
    pub headless: bool,
    /// Data directory override, accepted only together with `headless`.
    pub data_dir: Option<PathBuf>,
    /// Panic right after "ready", to test the crash log.
    #[cfg(feature = "test-panic")]
    pub panic_after_ready: bool,
}

/// How an engine run ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exit {
    /// Another engine of this session holds the mutex; this process only
    /// forwards "show viewer" to it (#61) and exits 0.
    AlreadyRunning,
    /// The engine ran and shut down.
    Ran {
        /// The previous run ended abnormally; names its crash log.
        abnormal_termination: Option<String>,
    },
}

#[cfg(windows)]
pub use run::run;

/// Runs the engine role until it is asked to exit.
///
/// # Errors
///
/// Returns the error that stopped the engine from starting.
#[cfg(not(windows))]
pub fn run(
    _options: &Options,
    _milestones: &peekbuddy_core::perf::MilestoneTable,
) -> std::io::Result<Exit> {
    Ok(Exit::Ran {
        abnormal_termination: None,
    })
}

#[cfg(windows)]
mod run {
    use std::cell::RefCell;
    use std::rc::Rc;

    use peekbuddy_core::perf::{Milestone, MilestoneTable, Role};
    use peekbuddy_win::console::close_window_on_ctrl_c;
    use peekbuddy_win::osinfo;
    use peekbuddy_win::perf::{etw, record, since_process_create_us};
    use peekbuddy_win::single_instance::SingleInstance;
    use peekbuddy_win::window::{HiddenWindow, WindowEvent};

    use crate::crash::{self, BuildInfo};
    use crate::datadir::{self, DataDir};
    use crate::portable::Decision;
    use crate::shutdown::{Reason, Shutdown};
    use crate::{Exit, Names, Options};

    fn mark(table: &MilestoneTable, milestone: Milestone) {
        record(table, milestone, Role::Engine, since_process_create_us());
    }

    /// Runs the engine role until it is asked to exit.
    ///
    /// # Errors
    ///
    /// Returns the error that stopped the engine from starting.
    pub fn run(options: &Options, milestones: &MilestoneTable) -> std::io::Result<Exit> {
        let _provider = etw::register();
        let names = Names::new(options.data_dir.as_deref());
        let Some(_instance) = SingleInstance::acquire(&names.mutex)? else {
            return Ok(Exit::AlreadyRunning);
        };
        // The placeholder policy itself is owned by #49.
        let _ = osinfo::expose_placeholders();

        let (root, decision) = match &options.data_dir {
            Some(root) => (root.clone(), Decision::Installed(None)),
            None => datadir::locate(&std::env::current_exe()?)?,
        };
        let data = DataDir::open(&root, decision)?;
        let abnormal_termination = crash::take_marker(&data.root);
        crash::install_panic_hook(
            data.logs.clone(),
            data.root.clone(),
            BuildInfo {
                build: env!("CARGO_PKG_VERSION").to_owned(),
                os_build: osinfo::os_build(),
            },
        );

        let shutdown = Rc::new(RefCell::new(Shutdown::new()));
        let handler = Rc::clone(&shutdown);
        let hidden = HiddenWindow::create(&names.window_class, move |event| match event {
            WindowEvent::Close => handler.borrow_mut().run(Reason::Quit),
            WindowEvent::EndSession { ending: true } => {
                handler.borrow_mut().run(Reason::EndSession);
            }
            WindowEvent::QueryEndSession | WindowEvent::EndSession { ending: false } => {}
        })?;
        if options.headless {
            close_window_on_ctrl_c(hidden.handle())?;
        }

        // F7: ready is "tray added AND all watchers armed". Neither exists
        // yet (#60, #45), so for now it is the point where the window is up.
        mark(milestones, Milestone::EngineReady);
        #[cfg(feature = "test-panic")]
        if options.panic_after_ready {
            test_panic();
        }
        let _ = hidden.run_message_loop();
        Ok(Exit::Ran {
            abnormal_termination,
        })
    }

    #[cfg(feature = "test-panic")]
    #[expect(clippy::panic, reason = "test-only crash trigger (#59)")]
    fn test_panic() {
        panic!("test panic requested with --test-panic");
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{MUTEX_NAME, Names, WINDOW_CLASS};

    #[test]
    fn without_a_data_dir_the_names_are_the_fixed_ones() {
        let names = Names::new(None);
        assert_eq!(names.mutex, MUTEX_NAME);
        assert_eq!(names.window_class, WINDOW_CLASS);
    }

    #[test]
    fn a_data_dir_makes_the_names_unique_and_stable() {
        let a = Names::new(Some(Path::new(r"C:\t\a")));
        let b = Names::new(Some(Path::new(r"C:\t\b")));
        assert_ne!(a, b);
        assert_ne!(a, Names::new(None));
        // Case does not matter on Windows paths.
        assert_eq!(a, Names::new(Some(Path::new(r"c:\T\A"))));
    }
}
