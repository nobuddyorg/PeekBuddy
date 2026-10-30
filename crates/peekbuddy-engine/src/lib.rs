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
pub mod autostart_ctl;
#[cfg(windows)]
pub mod capture_host;
#[cfg(windows)]
pub mod datadir;
#[cfg(windows)]
pub mod diag_collect;
#[cfg(windows)]
pub mod instrument;
#[cfg(windows)]
pub mod ipc_server;
#[cfg(windows)]
pub mod pause_ctl;
#[cfg(windows)]
pub mod reconciler;
#[cfg(windows)]
mod sample;
#[cfg(windows)]
pub mod settings;
#[cfg(windows)]
pub mod store_win;
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
    /// Push this many synthetic events for the first watch right after
    /// "ready", to test the store end to end.
    #[cfg(feature = "test-panic")]
    pub synthetic_events: u32,
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
    _milestones: &std::sync::Arc<peekbuddy_core::perf::MilestoneTable>,
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
    use std::sync::{Arc, Mutex};

    use peekbuddy_core::perf::{Milestone, MilestoneTable};
    use peekbuddy_win::console::{attach_output, close_window_on_ctrl_c};
    use peekbuddy_win::metrics::ContextSwitches;
    use peekbuddy_win::osinfo;
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

    /// The running capture, where the shutdown path finds it.
    type SharedHost = Rc<RefCell<Option<crate::capture_host::Host>>>;

    /// The autostart control the pipe handler shares.
    pub(crate) type SharedAutostart =
        Arc<crate::autostart_ctl::AutostartCtl<peekbuddy_win::autostart::RunKeys>>;

    use crate::instrument::mark;

    /// Runs the engine role until it is asked to exit.
    ///
    /// # Errors
    ///
    /// Returns the error that stopped the engine from starting.
    pub fn run(
        options: &Options,
        milestones: &Arc<MilestoneTable>,
        provider: Provider,
    ) -> std::io::Result<Exit> {
        // Held in a cell so the shutdown path can release it explicitly: on
        // `WM_ENDSESSION` the OS may end the process before `run` returns.
        let provider = Rc::new(RefCell::new(Some(provider)));
        let names = Names::new(options.data_dir.as_deref());
        let Some(_instance) = SingleInstance::acquire(&names.mutex)? else {
            forward_to_running_engine(options);
            return Ok(Exit::AlreadyRunning);
        };
        // The placeholder policy itself is owned by #49.
        let _ = osinfo::expose_placeholders();

        let (data, abnormal_termination) = open_data(options)?;
        init_instrumentation(&data, milestones);
        let loaded_settings = crate::settings::load_or_create(&data);
        mark(milestones, Milestone::SettingsLoaded);

        // The writer feeds both on every commit.
        let counters = Arc::new(peekbuddy_core::perf::Counters::new());
        let commits = Arc::new(Mutex::new(probe::CommitLog::new()));
        let shutdown = Rc::new(RefCell::new(Shutdown::new()));
        let store = start_store(&data, &loaded_settings, &counters, &commits, &shutdown)?;
        mark(milestones, Milestone::StoreOpened);
        if let Some(log) = &abnormal_termination {
            record_abnormal_exit(&store, log);
        }
        // Queued before the window exists, so a test that sees the window
        // knows the events are pending.
        #[cfg(feature = "test-panic")]
        if options.synthetic_events > 0 {
            push_synthetic(&store, &counters, options.synthetic_events);
        }
        // F7: ready is "tray attempt done AND every watch armed (or failed)
        // AND setup done"; the last of them records `EngineReady`.
        let tray_wanted = !options.headless || options.tray;
        let instruments = instruments(
            milestones,
            loaded_settings.settings.watches.len() + usize::from(tray_wanted) + 1,
        );
        let gate = Arc::clone(&instruments.gate);
        let (pause, capture) = start_pause_and_capture(
            &data,
            &loaded_settings,
            &store,
            &counters,
            &shutdown,
            &instruments,
        )?;
        let autostart = start_autostart(options, &loaded_settings)?;
        shutdown
            .borrow_mut()
            .register(Stage::ReleaseProvider, release_provider(&provider));
        // The probe's closing sample is taken when its timer fires, before
        // the shutdown starts: teardown wakeups are not idle wakeups.
        let switches = Rc::new(RefCell::new(ContextSwitches::new()));
        let closing: Rc<RefCell<Option<Sample>>> = Rc::new(RefCell::new(None));
        let (timer_switches, timer_sample) = (Rc::clone(&switches), Rc::clone(&closing));
        let tray_cell: SharedTray = Rc::new(RefCell::new(None));
        let hidden = create_window(
            &names.window_class,
            Rc::clone(&shutdown),
            Rc::clone(&tray_cell),
            (timer_switches, timer_sample),
        )?;
        if options.headless || options.probe.is_some() {
            close_window_on_ctrl_c(hidden.handle())?;
        }
        start_ipc(options, &data, &hidden, &shutdown, (&pause, autostart))?;

        if tray_wanted {
            start_tray(
                &hidden,
                &tray_cell,
                &shutdown,
                &data,
                (&loaded_settings, &pause),
                abnormal_termination.is_some(),
                milestones,
            )?;
            // A failed tray is retried later; the attempt is what counts.
            gate.done();
        }
        // Setup is done.
        gate.done();
        #[cfg(feature = "test-panic")]
        if options.panic_after_ready {
            test_panic();
        }
        drop(store);
        let _capture = capture;
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
            let report = probe_report(probe, milestones, (&counters, &commits), (start, end));
            emit(&report, probe.out.as_deref())?;
        }
        Ok(Exit::Ran {
            abnormal_termination,
            settings: Some(loaded_settings.state.describe()),
        })
    }

    /// The document `--perf-probe` prints.
    fn probe_report(
        probe: &crate::ProbeOptions,
        milestones: &MilestoneTable,
        (counters, commits): (
            &Arc<peekbuddy_core::perf::Counters>,
            &Arc<Mutex<probe::CommitLog>>,
        ),
        (start, end): (Sample, Sample),
    ) -> Report {
        Report {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            arch: arch_name(),
            requested_seconds: probe.seconds,
            milestones: probe::milestone_rows(milestones),
            start,
            end,
            counters: counters.snapshot(),
            batch_commits: commits
                .lock()
                .map_or_else(|_| Vec::new(), |log| log.to_vec()),
        }
    }

    /// The hidden window and the dispatch of its events: tray, shutdown and
    /// the probe's closing sample.
    fn create_window(
        class: &str,
        shutdown: Rc<RefCell<Shutdown>>,
        tray: SharedTray,
        (switches, sample): (Rc<RefCell<ContextSwitches>>, Rc<RefCell<Option<Sample>>>),
    ) -> std::io::Result<HiddenWindow> {
        HiddenWindow::create(class, move |event, window| match event {
            WindowEvent::TraySelect
            | WindowEvent::TrayMenu
            | WindowEvent::Command(_)
            | WindowEvent::TaskbarCreated
            | WindowEvent::ThemeChanged => {
                if let Some(tray) = tray.borrow_mut().as_mut() {
                    tray.handle(event, window);
                }
            }
            WindowEvent::Close => shutdown.borrow_mut().run(Reason::Quit),
            // The probe's one-shot exit timer.
            WindowEvent::Timer => {
                *sample.borrow_mut() = Some(crate::sample::take(&mut switches.borrow_mut()));
                let _ = window.post_close();
            }
            WindowEvent::EndSession { ending: true } => {
                shutdown.borrow_mut().run(Reason::EndSession);
            }
            WindowEvent::QueryEndSession | WindowEvent::EndSession { ending: false } => {}
        })
    }

    /// Opens today's segment and starts the writer. The shutdown path drains
    /// the queue, commits, checkpoints and closes the database, in that order.
    /// Returns the handle capture pushes batches through.
    fn start_store(
        data: &DataDir,
        loaded: &crate::settings::LoadedSettings,
        counters: &Arc<peekbuddy_core::perf::Counters>,
        commits: &Arc<Mutex<probe::CommitLog>>,
        shutdown: &Rc<RefCell<Shutdown>>,
    ) -> std::io::Result<peekbuddy_store::pipeline::Producer> {
        let store = crate::store_win::Store::start(data, &loaded.settings, counters, commits)?;
        let producer = store.producer();
        let store = Rc::new(RefCell::new(store));
        let mut steps = shutdown.borrow_mut();
        let cell = Rc::clone(&store);
        // Everything queued reaches the disk within the deadline (F19); what
        // does not stays in the spill file for the next start.
        steps.register(Stage::DrainRing, move |_| {
            let _ = cell.borrow().flush();
        });
        let cell = Rc::clone(&store);
        steps.register(Stage::FinalCommit, move |_| cell.borrow_mut().close());
        let cell = Rc::clone(&store);
        steps.register(Stage::PersistCheckpoints, move |_| {
            cell.borrow().checkpoint();
        });
        steps.register(Stage::CloseDatabase, move |_| store.borrow_mut().release());
        Ok(producer)
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

    /// Second launch: ask the running engine to show the viewer. If that
    /// fails there is nothing more to do; the second launch still exits 0.
    fn forward_to_running_engine(options: &Options) {
        if let (Ok(pipe), Ok(exe)) = (
            crate::ipc_server::pipe_name(options.data_dir.as_deref()),
            std::env::current_exe(),
        ) {
            let _ = crate::ipc_server::forward_show_viewer(&pipe, &exe);
        }
    }

    /// Opens the data directory, takes the crash marker of a previous run
    /// and installs the panic hook that writes the next one.
    fn open_data(options: &Options) -> std::io::Result<(DataDir, Option<String>)> {
        let (root, decision) = match &options.data_dir {
            Some(root) => (root.clone(), Decision::Installed(None)),
            None => datadir::locate(&std::env::current_exe()?)?,
        };
        let data = DataDir::open(&root, decision)?;
        let abnormal = crash::take_marker(&data.root);
        crash::install_panic_hook(
            data.logs.clone(),
            data.root.clone(),
            BuildInfo {
                build: env!("CARGO_PKG_VERSION").to_owned(),
                os_build: osinfo::os_build(),
            },
        );
        Ok((data, abnormal))
    }

    /// Writes the one row that says the previous run ended abnormally; its
    /// name is the crash log. The tray problem state and the start reconcile
    /// follow from the same marker and from the normal start.
    fn record_abnormal_exit(producer: &peekbuddy_store::pipeline::Producer, crash_log: &str) {
        use peekbuddy_core::event::{
            Event, Flags, Kind, NameKind, NameRef, RootIdx, Source, VolIdx,
        };

        let (ts, qpc) = crate::pause_ctl::now_ticks();
        let event = Event {
            ts,
            qpc,
            src_ts: 0,
            source: Source::System,
            kind: Kind::EngineAbnormalExit,
            raw_mask: 0,
            source_info: 0,
            is_dir: false,
            size: 0,
            attrs: 0,
            reparse_tag: 0,
            file_id: 0,
            parent_file_id: 0,
            usn: 0,
            vol: VolIdx(0),
            root: RootIdx::NONE,
            name: NameRef::EMPTY,
            name_kind: NameKind::Unknown,
            flags: Flags::NONE,
        };
        let name: Vec<u16> = crash_log.trim().encode_utf16().collect();
        let mut batch = producer.new_batch();
        if batch.push(event, &name).is_ok() {
            let _ = producer.push(batch);
        }
    }

    /// What the capture side needs to report milestones.
    struct Instruments {
        milestones: Arc<MilestoneTable>,
        gate: Arc<crate::instrument::ReadyGate>,
    }

    /// The log file and the `ViewerSpawn` milestone.
    fn init_instrumentation(data: &DataDir, milestones: &Arc<MilestoneTable>) {
        crate::instrument::init_log(&data.logs);
        let table = Arc::clone(milestones);
        crate::instrument::on_viewer_spawn(move || mark(&table, Milestone::ViewerSpawn));
    }

    /// The milestone table and a gate that records `EngineReady` after
    /// `count` calls of `done`.
    fn instruments(milestones: &Arc<MilestoneTable>, count: usize) -> Instruments {
        let table = Arc::clone(milestones);
        Instruments {
            milestones: Arc::clone(milestones),
            gate: crate::instrument::ReadyGate::new(count, move || {
                mark(&table, Milestone::EngineReady);
            }),
        }
    }

    /// Pause control and capture, which need each other: pausing is a signal
    /// to the lifecycle machines, and the machines read the pause switches.
    fn start_pause_and_capture(
        data: &DataDir,
        loaded: &crate::settings::LoadedSettings,
        producer: &peekbuddy_store::pipeline::Producer,
        counters: &Arc<peekbuddy_core::perf::Counters>,
        shutdown: &Rc<RefCell<Shutdown>>,
        instruments: &Instruments,
    ) -> std::io::Result<(Arc<crate::pause_ctl::PauseCtl>, Option<SharedHost>)> {
        let (pause_rows, bind) = crate::capture_host::PauseSignals::new();
        let pause = crate::pause_ctl::PauseCtl::new(
            loaded.settings.clone(),
            data.settings.clone(),
            !matches!(
                loaded.state,
                crate::settings::State::TooNew { .. } | crate::settings::State::Invalid { .. }
            ),
            Box::new(pause_rows),
            crate::pause_ctl::now_ticks,
        );
        let capture = start_capture(
            data,
            loaded,
            (producer, counters),
            (&pause, &bind),
            (shutdown, instruments),
        )?;
        pause.announce_start();
        Ok((pause, capture))
    }

    /// Starts capture: the watches of the settings, their lifecycle and the
    /// reconcile worker. Without watches nothing is started. The host stops
    /// in the `StopCapture` step, before the queue is drained.
    fn start_capture(
        data: &DataDir,
        loaded: &crate::settings::LoadedSettings,
        (producer, counters): (
            &peekbuddy_store::pipeline::Producer,
            &Arc<peekbuddy_core::perf::Counters>,
        ),
        (pause, bind): (
            &Arc<crate::pause_ctl::PauseCtl>,
            &Arc<std::sync::OnceLock<crate::capture_host::Signaller>>,
        ),
        (shutdown, instruments): (&Rc<RefCell<Shutdown>>, &Instruments),
    ) -> std::io::Result<Option<SharedHost>> {
        use crate::capture_host::{
            Host, HostParts, Services, StoreSink, filters_from_settings, real_probe,
            watches_from_settings,
        };
        use crate::reconciler::{Reconciler, RootSpec, StoreRows};

        let watches = watches_from_settings(&loaded.settings);
        if watches.is_empty() {
            return Ok(None);
        }
        let sink = Box::new(StoreSink::new(
            producer.clone(),
            filters_from_settings(&loaded.settings),
            Arc::clone(counters),
        ));
        let specs: Vec<RootSpec> = watches
            .iter()
            .map(|watch| RootSpec {
                root: watch.root,
                vol: watch.vol,
                path: watch.long.clone(),
            })
            .collect();
        // Without a snapshot there is no reconcile; the watches still run.
        let reconciler =
            peekbuddy_store::snapshot::SnapshotDb::open(&data.root.join("snapshot.db"))
                .ok()
                .and_then(|snapshot| {
                    Reconciler::start(
                        snapshot,
                        StoreRows::new(producer.clone()),
                        specs,
                        crate::pause_ctl::now_ticks,
                        Host::observer(bind),
                    )
                    .ok()
                });
        let flusher = producer.clone();
        let services = Services {
            rows: Box::new(crate::pause_ctl::ProducerRows(producer.clone())),
            flush: Box::new(move || {
                let _ = flusher.flush(std::time::Duration::from_secs(2));
            }),
            probe: real_probe(),
            clock: crate::pause_ctl::now_ticks,
            notifier: Some(crate::capture_host::notifier_factory(bind)),
            milestone: {
                let table = Arc::clone(&instruments.milestones);
                Arc::new(move |milestone| mark(&table, milestone))
            },
            ready: Some(Arc::clone(&instruments.gate)),
        };
        let host = Host::start(
            HostParts {
                watches,
                sink,
                pause: pause.pause(),
                reconciler,
                services,
                exclude: crate::capture_host::canonical_dirs(&[data.root.as_path()]),
            },
            bind,
        )?;
        crate::capture_host::watch_system(&host);
        let cell = Rc::new(RefCell::new(Some(host)));
        let stopping = Rc::clone(&cell);
        shutdown
            .borrow_mut()
            .register(Stage::StopCapture, move |_| {
                if let Some(mut host) = stopping.borrow_mut().take() {
                    host.stop();
                }
            });
        Ok(Some(cell))
    }

    /// Brings the Run value in line with the saved intent. Test engines (an
    /// explicit data directory, or headless) never touch the registry.
    fn start_autostart(
        options: &Options,
        loaded: &crate::settings::LoadedSettings,
    ) -> std::io::Result<Option<SharedAutostart>> {
        if options.data_dir.is_some() || options.headless {
            return Ok(None);
        }
        let exe = std::env::current_exe()?.to_string_lossy().into_owned();
        let auto = peekbuddy_core::autostart::Autostart::new(
            peekbuddy_win::autostart::RunKeys::current_user(),
            &exe,
        );
        let (ctl, _status) =
            crate::autostart_ctl::AutostartCtl::start(auto, loaded.settings.general.autostart);
        Ok(Some(Arc::new(ctl)))
    }

    /// Starts the viewer pipe. A pipe that already exists means another engine
    /// of this user (a second session) or a squatter: the engine fails closed
    /// with the reason written to `logs\ipc-error.log`.
    fn start_ipc(
        options: &Options,
        data: &DataDir,
        hidden: &HiddenWindow,
        shutdown: &Rc<RefCell<Shutdown>>,
        (pause, autostart): (&Arc<crate::pause_ctl::PauseCtl>, Option<SharedAutostart>),
    ) -> std::io::Result<()> {
        let name = crate::ipc_server::pipe_name(options.data_dir.as_deref())?;
        let shared = crate::ipc_server::Shared {
            window: hidden.handle(),
            exe: std::env::current_exe()?,
            settings_path: data.settings.clone(),
            data_dir: data.root.to_string_lossy().into_owned(),
            pause: Arc::clone(pause),
            autostart,
        };
        let server = match crate::ipc_server::start(&name, shared) {
            Ok(server) => server,
            Err(error) => {
                let reason = format!(
                    "could not create {name}: {error}
Another PeekBuddy engine of this user owns it, or another program created it first. This engine exits instead of running a second writer.
"
                );
                let _ = std::fs::write(data.logs.join("ipc-error.log"), reason);
                return Err(error);
            }
        };
        let cell = Rc::new(RefCell::new(Some(server)));
        shutdown.borrow_mut().register(Stage::StopIpc, move |_| {
            if let Some(mut server) = cell.borrow_mut().take() {
                server.stop();
            }
        });
        Ok(())
    }

    /// Creates the tray, shows it and registers its removal for shutdown.
    fn start_tray(
        hidden: &HiddenWindow,
        cell: &SharedTray,
        shutdown: &Rc<RefCell<Shutdown>>,
        data: &DataDir,
        (loaded, pause): (
            &crate::settings::LoadedSettings,
            &Arc<crate::pause_ctl::PauseCtl>,
        ),
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
            Arc::clone(pause),
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

    /// Pushes `count` events for the first watch through the store, like
    /// capture will, so tests see rows, commits and counters end to end.
    #[cfg(feature = "test-panic")]
    fn push_synthetic(
        producer: &peekbuddy_store::pipeline::Producer,
        counters: &peekbuddy_core::perf::Counters,
        count: u32,
    ) {
        use peekbuddy_core::event::{
            Event, Flags, Kind, NameKind, NameRef, RootIdx, Source, VolIdx,
        };

        let mut batch = producer.new_batch();
        for index in 0..u64::from(count) {
            let event = Event {
                ts: 133_000_000_000_000_000 + index,
                qpc: index,
                src_ts: 0,
                source: Source::Rdcw,
                kind: Kind::Created,
                raw_mask: 1,
                source_info: 0,
                is_dir: false,
                size: 1,
                attrs: 0x20,
                reparse_tag: 0,
                file_id: u128::from(index) + 1,
                parent_file_id: 5,
                usn: 0,
                vol: VolIdx(0),
                root: RootIdx(0),
                name: NameRef::EMPTY,
                name_kind: NameKind::Long,
                flags: Flags::NONE,
            };
            let name: Vec<u16> = format!(r"synthetic\file{index}.txt")
                .encode_utf16()
                .collect();
            if batch.push(event, &name).is_err() {
                let spare = match producer.push(batch) {
                    peekbuddy_store::pipeline::Push::Queued(spare)
                    | peekbuddy_store::pipeline::Push::Overflow(spare)
                    | peekbuddy_store::pipeline::Push::Closed(spare) => spare,
                };
                batch = spare;
                let _ = batch.push(event, &name);
            }
        }
        let _ = producer.push(batch);
        counters
            .events_received
            .fetch_add(u64::from(count), std::sync::atomic::Ordering::Relaxed);
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
