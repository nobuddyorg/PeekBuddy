//! The PeekBuddy engine role: tray, watches, reconciliation and the writer.
//!
//! The engine never depends on a GPU or UI stack (ADR 0001); `cargo xtask
//! check-deps` enforces that. This crate holds the process skeleton (#59):
//! single instance, data directory, hidden window and message loop, crash
//! log and the one shutdown path. Tray (#60), pipe (#61), settings (#62) and
//! capture hang off it.
#![forbid(unsafe_code)]

pub mod crash;
pub mod diagnostics;
pub mod portable;
pub mod probe;
pub mod shutdown;
pub mod tray_model;

#[cfg(windows)]
pub mod datadir;
#[cfg(windows)]
pub mod diag_collect;
#[cfg(windows)]
mod sample;
#[cfg(windows)]
pub mod settings;
#[cfg(windows)]
pub mod tray_win;

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
    /// Show the tray although `headless` (tests of the tray itself).
    pub tray: bool,
    /// Data directory override, accepted only together with `headless`.
    pub data_dir: Option<PathBuf>,
    /// `--perf-probe`: stay up for a while, then print the JSON report (#24).
    pub probe: Option<ProbeOptions>,
    /// Panic right after "ready", to test the crash log.
    #[cfg(feature = "test-panic")]
    pub panic_after_ready: bool,
}

/// `--perf-probe <seconds> [--perf-probe-out <file>]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeOptions {
    /// Seconds to stay up after "ready"; 0 exits right after it.
    pub seconds: u64,
    /// Write the report here instead of standard output.
    pub out: Option<PathBuf>,
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
        /// How `settings.toml` was found (Windows only).
        settings: Option<String>,
    },
}

/// Keeps the ETW provider registered. `main` registers it before the first
/// milestone, so `ProcessStart` is not written to an unregistered provider,
/// and hands it to the engine, whose shutdown path releases it.
#[cfg(windows)]
pub type Provider = peekbuddy_win::perf::etw::Registration;

/// Placeholder where there is no ETW.
#[cfg(not(windows))]
#[derive(Debug)]
pub struct Provider;

/// Registers the ETW provider (a no-op off Windows).
#[must_use]
pub fn register_provider() -> Provider {
    #[cfg(windows)]
    {
        peekbuddy_win::perf::etw::register()
    }
    #[cfg(not(windows))]
    {
        Provider
    }
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
    _provider: Provider,
) -> std::io::Result<Exit> {
    Ok(Exit::Ran {
        abnormal_termination: None,
        settings: None,
    })
}

#[cfg(windows)]
mod run {
    use std::cell::RefCell;
    use std::rc::Rc;

    use peekbuddy_core::perf::{Milestone, MilestoneTable, Role};
    use peekbuddy_win::console::{attach_output, close_window_on_ctrl_c};
    use peekbuddy_win::metrics::ContextSwitches;
    use peekbuddy_win::osinfo;
    use peekbuddy_win::perf::{record, since_process_create_us};
    use peekbuddy_win::single_instance::SingleInstance;
    use peekbuddy_win::window::{HiddenWindow, WindowEvent};

    use crate::crash::{self, BuildInfo};
    use crate::datadir::{self, DataDir};
    use crate::portable::Decision;
    use crate::probe::{self, Report, Sample};
    use crate::shutdown::{Reason, Shutdown, Stage};
    use crate::tray_model::{Health, TrayState, WatchEntry};
    use crate::tray_win::{SharedTray, TrayController, label_for};
    use crate::{Exit, Names, Options, Provider};

    fn mark(table: &MilestoneTable, milestone: Milestone) {
        record(table, milestone, Role::Engine, since_process_create_us());
    }

    /// Runs the engine role until it is asked to exit.
    ///
    /// # Errors
    ///
    /// Returns the error that stopped the engine from starting.
    pub fn run(
        options: &Options,
        milestones: &MilestoneTable,
        provider: Provider,
    ) -> std::io::Result<Exit> {
        // Held in a cell so the shutdown path can release it explicitly: on
        // `WM_ENDSESSION` the OS may end the process before `run` returns.
        let provider = Rc::new(RefCell::new(Some(provider)));
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
        let loaded_settings = crate::settings::load_or_create(&data);
        mark(milestones, Milestone::SettingsLoaded);
        crash::install_panic_hook(
            data.logs.clone(),
            data.root.clone(),
            BuildInfo {
                build: env!("CARGO_PKG_VERSION").to_owned(),
                os_build: osinfo::os_build(),
            },
        );

        // The writer (#52) feeds both; until it exists they stay empty.
        let counters = peekbuddy_core::perf::Counters::new();
        let commits = probe::CommitLog::new();
        let shutdown = Rc::new(RefCell::new(Shutdown::new()));
        shutdown
            .borrow_mut()
            .register(Stage::ReleaseProvider, release_provider(&provider));
        let handler = Rc::clone(&shutdown);
        // The probe's closing sample is taken when its timer fires, before
        // the shutdown starts: teardown wakeups are not idle wakeups.
        let switches = Rc::new(RefCell::new(ContextSwitches::new()));
        let closing: Rc<RefCell<Option<Sample>>> = Rc::new(RefCell::new(None));
        let (timer_switches, timer_sample) = (Rc::clone(&switches), Rc::clone(&closing));
        let tray_cell: SharedTray = Rc::new(RefCell::new(None));
        let tray_events = Rc::clone(&tray_cell);
        let hidden = HiddenWindow::create(&names.window_class, move |event, window| match event {
            WindowEvent::TraySelect
            | WindowEvent::TrayMenu
            | WindowEvent::Command(_)
            | WindowEvent::TaskbarCreated
            | WindowEvent::ThemeChanged => {
                if let Some(tray) = tray_events.borrow_mut().as_mut() {
                    tray.handle(event, window);
                }
            }
            WindowEvent::Close => handler.borrow_mut().run(Reason::Quit),
            // The probe's one-shot exit timer.
            WindowEvent::Timer => {
                *timer_sample.borrow_mut() =
                    Some(crate::sample::take(&mut timer_switches.borrow_mut()));
                let _ = window.post_close();
            }
            WindowEvent::EndSession { ending: true } => {
                handler.borrow_mut().run(Reason::EndSession);
            }
            WindowEvent::QueryEndSession | WindowEvent::EndSession { ending: false } => {}
        })?;
        if options.headless || options.probe.is_some() {
            close_window_on_ctrl_c(hidden.handle())?;
        }

        if !options.headless || options.tray {
            start_tray(
                &hidden,
                &tray_cell,
                &shutdown,
                &data,
                &loaded_settings,
                abnormal_termination.is_some(),
                milestones,
            )?;
        }

        // F7: ready is "tray added AND all watchers armed". There are no
        // watchers yet (#45), and a failed tray is retried later, so ready is
        // the point where the tray attempt is done.
        mark(milestones, Milestone::EngineReady);
        #[cfg(feature = "test-panic")]
        if options.panic_after_ready {
            test_panic();
        }
        let start = crate::sample::take(&mut switches.borrow_mut());
        if let Some(probe) = &options.probe {
            arm_probe(&hidden, probe.seconds)?;
        }
        let _ = hidden.run_message_loop();
        if let Some(probe) = &options.probe {
            let end = closing
                .borrow_mut()
                .take()
                .unwrap_or_else(|| crate::sample::take(&mut switches.borrow_mut()));
            let report = Report {
                version: env!("CARGO_PKG_VERSION").to_owned(),
                arch: arch_name(),
                requested_seconds: probe.seconds,
                milestones: probe::milestone_rows(milestones),
                start,
                end,
                counters: counters.snapshot(),
                batch_commits: commits.to_vec(),
            };
            emit(&report, probe.out.as_deref())?;
        }
        Ok(Exit::Ran {
            abnormal_termination,
            settings: Some(loaded_settings.state.describe()),
        })
    }

    /// `--perf-probe`: exit right away for 0 seconds, else after the timer.
    fn arm_probe(hidden: &HiddenWindow, seconds: u64) -> std::io::Result<()> {
        if seconds == 0 {
            return hidden.handle().post_close();
        }
        let millis = u32::try_from(seconds.saturating_mul(1_000)).unwrap_or(u32::MAX);
        hidden.set_one_shot_timer(millis)
    }

    const fn arch_name() -> &'static str {
        if cfg!(target_arch = "aarch64") {
            "aarch64"
        } else {
            "x86_64"
        }
    }

    /// Creates the tray, shows it and registers its removal for shutdown.
    fn start_tray(
        hidden: &HiddenWindow,
        cell: &SharedTray,
        shutdown: &Rc<RefCell<Shutdown>>,
        data: &DataDir,
        loaded: &crate::settings::LoadedSettings,
        crashed: bool,
        milestones: &MilestoneTable,
    ) -> std::io::Result<()> {
        let health = if loaded.state.is_problem() || crashed {
            Health::Problem
        } else {
            Health::Live
        };
        let watches = loaded
            .settings
            .watches
            .iter()
            .map(|watch| WatchEntry {
                label: label_for(&watch.path),
                paused: !watch.enabled,
            })
            .collect();
        let mut controller = TrayController::new(
            hidden.handle(),
            std::env::current_exe()?,
            data.root.clone(),
            TrayState {
                health,
                rate: 0.0,
                watches,
            },
        );
        // Not fatal when it fails: the engine keeps capturing and the icon is
        // added when the taskbar appears (`TaskbarCreated`).
        if controller.show() {
            mark(milestones, Milestone::TrayShown);
            if loaded.settings.general.first_run && !loaded.state.is_problem() {
                controller.first_run_balloon();
                let _ = crate::settings::mark_first_run_done(data, loaded);
            }
        }
        *cell.borrow_mut() = Some(controller);
        let removal = Rc::clone(cell);
        shutdown.borrow_mut().register(Stage::RemoveTray, move |_| {
            if let Some(tray) = removal.borrow_mut().as_mut() {
                tray.remove();
            }
        });
        Ok(())
    }

    /// The shutdown step that unregisters the ETW provider.
    pub(crate) fn release_provider(
        provider: &Rc<RefCell<Option<Provider>>>,
    ) -> impl FnMut(Reason) + 'static {
        let provider = Rc::clone(provider);
        move |_| drop(provider.borrow_mut().take())
    }

    /// Writes the report to `out`, or to standard output.
    fn emit(report: &Report, out: Option<&std::path::Path>) -> std::io::Result<()> {
        let json = report.to_json();
        if let Some(path) = out {
            return std::fs::write(path, json);
        }
        attach_output();
        let mut stdout = std::io::stdout().lock();
        std::io::Write::write_all(&mut stdout, json.as_bytes())?;
        std::io::Write::flush(&mut stdout)
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

#[cfg(all(test, windows))]
mod windows_tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use peekbuddy_win::perf::etw;

    use crate::shutdown::{Reason, Shutdown, Stage};

    #[test]
    fn the_shutdown_path_unregisters_the_provider() {
        let provider = Rc::new(RefCell::new(Some(etw::register())));
        let mut shutdown = Shutdown::new();
        shutdown.register(
            Stage::ReleaseProvider,
            super::run::release_provider(&provider),
        );
        assert!(etw::is_registered());
        shutdown.run(Reason::EndSession);
        assert!(!etw::is_registered());
        assert!(provider.borrow().is_none());
    }
}
