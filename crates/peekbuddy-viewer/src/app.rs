//! The eframe application (#65). Windows only: the window and the renderer
//! come from eframe, the frame and the state it shows from [`crate::shell`]
//! and [`crate::link`], which build and test on every host.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use eframe::egui;
use peekbuddy_core::ipc::Command;
use peekbuddy_core::perf::{Milestone, Role as PerfRole};
use peekbuddy_core::settings::{self, Context, Loaded, Settings};
use peekbuddy_win::chrome;
use peekbuddy_win::perf::{record, since_process_create_us};
use peekbuddy_win::signal::Signal;
use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};

use crate::filterspec::FilterSpec;
use crate::geometry::{self, PxRect};
use crate::gpu::{self, Adapter};
use crate::link::{Link, LinkEvent, LinkState, Timing};
use crate::link_win::PipeConnector;
use crate::onboarding::{Onboarding, OnboardingAction};
use crate::settings_ui::{SettingsAction, SettingsScreen};
use crate::shell::{Shell, ShellAction};
use crate::shellpath;
use crate::source::StoreSource;
use crate::view::{LogView, ViewEvent};
use crate::watches::{Tab, WatchAction, WatchPanel};
use crate::{Options, Probe, theme};

/// How long the close handler waits for the engine to take the settings.
const SAVE_WAIT: Duration = Duration::from_secs(1);

/// How often the watch list is asked for while the Watches tab is open.
const WATCH_REFRESH: Duration = Duration::from_secs(2);

/// How long after the engine went away the restart starts a new one: the
/// old process needs a moment to release its mutex. A second try follows if
/// the engine does not come back.
const RESTART_DELAY: Duration = Duration::from_millis(1500);

/// Which screen asked for a watch command, so the answer goes back to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Origin {
    Tab,
    Onboarding,
}

/// The size of the files in `dir`, not looking further down.
fn directory_size(dir: &Path) -> u64 {
    std::fs::read_dir(dir).map_or(0, |entries| {
        entries
            .filter_map(Result::ok)
            .filter_map(|entry| entry.metadata().ok())
            .filter(std::fs::Metadata::is_file)
            .map(|metadata| metadata.len())
            .sum()
    })
}

/// Opens `dir` in Explorer.
fn open_folder(dir: &Path) {
    let parameters = format!("\"{}\"", dir.display());
    std::thread::spawn(move || {
        let _ = peekbuddy_win::shell::explorer(&parameters);
    });
}

/// The log view of this run: the filter of the last run (text that no longer
/// parses is dropped), the columns and the details pane as they were left.
fn log_view(
    options: &Options,
    settings: &Settings,
    waker: &Arc<dyn Fn() + Send + Sync>,
) -> std::io::Result<LogView> {
    let spec = FilterSpec::parse(&settings.viewer.filter).unwrap_or_default();
    let roots = settings
        .watches
        .iter()
        .map(|watch| watch.path.clone())
        .collect();
    let mut view = LogView::new(
        Arc::new(StoreSource::new(&options.segments)),
        Arc::new(peekbuddy_win::localtime::utc_offset_seconds),
        Arc::clone(waker),
        spec,
        roots,
        &settings.view.columns,
    )?;
    view.set_details(!settings.viewer.details_hidden);
    view.set_export_dirs(options.segments.clone(), options.data_dir.join("exports"));
    Ok(view)
}

/// The settings screen, and the first-run screens when nothing is watched
/// yet and the user was never onboarded. A measuring run (the probe) goes
/// straight to the log.
fn first_screens(options: &Options, settings: &Settings) -> (SettingsScreen, Option<Onboarding>) {
    let screen = SettingsScreen::new(
        options.data_dir.display().to_string(),
        options.about.clone(),
    );
    let onboarding =
        (settings.watches.is_empty() && settings.general.first_run && options.probe.is_none())
            .then(Onboarding::default);
    (screen, onboarding)
}

/// The name in messages. The window title itself is `Options::title`;
/// Windows draws it, so it cannot carry the wordmark colours (#88).
const TITLE: &str = "PeekBuddy";

/// What the probe measures; filled by the probe thread and the app, written
/// when the window closes.
#[derive(Debug, Default)]
struct ProbeReport {
    first_frame_us: Option<u64>,
    idle_seconds: u64,
    idle_frames: Option<u64>,
    adapter: Option<Adapter>,
    /// Who asked for the frames painted in the idle window, with counts
    /// (`file:line reason`): when a frame is painted that should not be, this
    /// says which call to look at.
    causes: BTreeMap<String, u32>,
}

fn json_string(text: &str) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

impl ProbeReport {
    fn to_json(&self, frames: u64) -> String {
        let number = |value: Option<u64>| value.map_or(String::from("null"), |v| v.to_string());
        let causes = format!(
            "{{{}}}",
            self.causes
                .iter()
                .map(|(cause, count)| format!("{}:{count}", json_string(cause)))
                .collect::<Vec<_>>()
                .join(",")
        );
        let (name, backend, device) = self.adapter.as_ref().map_or(
            (
                String::from("null"),
                String::from("null"),
                String::from("null"),
            ),
            |a| {
                (
                    json_string(&a.name),
                    json_string(&a.backend),
                    json_string(&a.device_type),
                )
            },
        );
        format!(
            "{{\"role\":\"viewer\",\"frames\":{frames},\"first_frame_us\":{},\
             \"idle_seconds\":{},\"idle_frames\":{},\"adapter\":{name},\
             \"backend\":{backend},\"device_type\":{device},\"causes\":{causes}}}\n",
            number(self.first_frame_us),
            self.idle_seconds,
            self.idle_frames
                .map_or(String::from("null"), |frames| frames.to_string()),
        )
    }
}

/// The settings of the data directory, or the defaults when the file is
/// missing, invalid or newer than this build (browsing still works).
fn load_settings(data_dir: &Path) -> Option<Settings> {
    let text = std::fs::read_to_string(data_dir.join("settings.toml")).ok()?;
    let context = Context {
        data_dir: data_dir.to_string_lossy().into_owned(),
        export_dir: None,
    };
    match settings::load(&text, &context) {
        Loaded::Ok { settings, .. } => Some(*settings),
        _ => None,
    }
}

fn window_handle(frame: &eframe::Frame) -> Option<isize> {
    match frame.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
        _ => None,
    }
}

fn append_log(data_dir: &Path, line: &str) {
    use std::io::Write as _;

    let logs = data_dir.join("logs");
    let path = logs.join("viewer.log");
    // A log that never grows past 64 KiB: start over when it is full.
    if std::fs::metadata(&path).is_ok_and(|meta| meta.len() > 64 * 1024) {
        let _ = std::fs::remove_file(&path);
    }
    if std::fs::create_dir_all(&logs).is_err() {
        return;
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{line}");
    }
}

struct ViewerApp {
    options: Options,
    shell: Shell,
    link: Link,
    view: LogView,
    tab: Tab,
    panel: WatchPanel,
    settings_screen: SettingsScreen,
    /// The first-run screens, while they are shown.
    onboarding: Option<Onboarding>,
    /// The tag of the settings save the engine has not answered yet.
    settings_tag: Option<u64>,
    /// A theme to switch to on the next frame (after a save).
    theme_pending: Option<settings::Theme>,
    /// The diagnostics text, while the exe collects it.
    diagnostics: Option<Receiver<String>>,
    /// Which screen opened the folder picker.
    picker_origin: Origin,
    /// Tags of the commands whose answer belongs on a watch screen.
    watch_tags: HashMap<u64, Origin>,
    next_refresh: Instant,
    /// When to start the engine again after a requested restart.
    restart_due: Option<Instant>,
    /// The folder picker's thread, while its dialog is open.
    picker: Option<Receiver<std::io::Result<Vec<String>>>>,
    /// Wakes the UI from another thread.
    waker: Arc<dyn Fn() + Send + Sync>,
    settings: Settings,
    /// Set by the thread that waits for a second launch.
    show_requested: Arc<AtomicBool>,
    frames: Arc<AtomicU64>,
    /// True while the probe counts idle frames.
    idle_window: Arc<AtomicBool>,
    hwnd: Option<isize>,
    placed: bool,
    /// The last rectangle the window had while neither maximized nor
    /// minimized: what is remembered when it closes.
    normal: Option<PxRect>,
    title_dark: Option<bool>,
    first_frame_done: bool,
    report: Arc<Mutex<ProbeReport>>,
}

impl ViewerApp {
    fn place_window(&mut self, hwnd: isize) {
        let areas: Vec<PxRect> = chrome::work_areas()
            .into_iter()
            .map(|r| PxRect {
                left: r.left,
                top: r.top,
                right: r.right,
                bottom: r.bottom,
            })
            .collect();
        let Some(placement) = geometry::restore(&self.settings.viewer, &areas) else {
            return;
        };
        let rect = chrome::Rect {
            left: placement.rect.left,
            top: placement.rect.top,
            right: placement.rect.right,
            bottom: placement.rect.bottom,
        };
        if let Err(error) = chrome::place(hwnd, rect, placement.maximized) {
            append_log(
                &self.options.data_dir,
                &format!("could not place the window: {error}"),
            );
        }
    }

    fn sync_title_bar(&mut self, ctx: &egui::Context, hwnd: isize) {
        let dark = ctx.theme() == egui::Theme::Dark;
        if self.title_dark == Some(dark) {
            return;
        }
        self.title_dark = Some(dark);
        // `Unsupported` (Windows 10 before 19041) is not an error.
        if let Err(error) = chrome::set_dark_title_bar(hwnd, dark) {
            append_log(&self.options.data_dir, &format!("title bar: {error}"));
        }
    }

    fn track_window(&mut self, hwnd: isize) {
        if !chrome::is_maximized(hwnd)
            && !chrome::is_minimized(hwnd)
            && let Some(r) = chrome::window_rect(hwnd)
        {
            self.normal = Some(PxRect {
                left: r.left,
                top: r.top,
                right: r.right,
                bottom: r.bottom,
            });
        }
    }

    fn handle(&mut self, event: LinkEvent) {
        match event {
            LinkEvent::State(state) => {
                match &state {
                    LinkState::Connected { .. } => {
                        self.panel.restarting = false;
                        self.restart_due = None;
                        self.link.request_watches();
                    }
                    LinkState::Gone if self.panel.restarting => {
                        self.restart_due = Some(Instant::now() + RESTART_DELAY);
                    }
                    _ => {}
                }
                self.shell.link = state;
            }
            LinkEvent::Result { tag, ok, message } => {
                if let Some(origin) = self.watch_tags.remove(&tag) {
                    match origin {
                        Origin::Tab => self.panel.on_result(ok, &message),
                        Origin::Onboarding => {
                            if let Some(onboarding) = &mut self.onboarding {
                                onboarding.adder.on_result(ok, &message);
                            }
                        }
                    }
                    self.link.request_watches();
                } else if self.settings_tag == Some(tag) {
                    self.settings_tag = None;
                    let saved = load_settings(&self.options.data_dir);
                    self.settings_screen.on_result(ok, &message, saved.as_ref());
                    if let (true, Some(saved)) = (ok, saved) {
                        self.theme_pending = Some(saved.general.theme);
                        self.settings = saved;
                    }
                }
            }
            LinkEvent::Watches(list) => self.panel.set_watches(list),
            LinkEvent::Pushed(_) => {}
        }
    }

    /// Carries out what the user asked for on a watch screen.
    fn watch_action(&mut self, action: WatchAction, origin: Origin) {
        let command = match action {
            WatchAction::Add { path, confirmed } => Command::AddWatch { path, confirmed },
            WatchAction::Remove(id) => Command::RemoveWatch { id },
            WatchAction::SetPaused { id, paused } => Command::SetWatchPaused { id, paused },
            WatchAction::Browse => {
                self.browse(origin);
                return;
            }
            WatchAction::RestartEngine => {
                self.panel.restarting = true;
                self.link.stop_engine();
                return;
            }
        };
        let tag = self.link.send(command);
        self.watch_tags.insert(tag, origin);
    }

    /// Opens the folder picker on a thread of its own.
    fn browse(&mut self, origin: Origin) {
        if self.picker.is_some() {
            return;
        }
        self.picker_origin = origin;
        let owner = self.hwnd.unwrap_or(0);
        let (sender, receiver) = std::sync::mpsc::channel();
        let waker = self.waker.clone();
        let started = std::thread::Builder::new()
            .name(String::from("folder-picker"))
            .spawn(move || {
                let _ = sender.send(peekbuddy_win::picker::pick_folders(
                    owner,
                    "Choose the folders to watch",
                ));
                waker();
            });
        if started.is_ok() {
            self.picker = Some(receiver);
        }
    }

    /// Takes the picker's answer, if it is there: each folder goes to the
    /// engine, which decides; cancelling changes nothing.
    fn picked(&mut self) {
        let Some(receiver) = &self.picker else {
            return;
        };
        let folders = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Ok(Vec::new()),
        };
        self.picker = None;
        match folders {
            Ok(folders) => {
                let origin = self.picker_origin;
                for folder in folders {
                    let action = match (origin, &mut self.onboarding) {
                        (Origin::Onboarding, Some(onboarding)) => onboarding.adder.picked(folder),
                        _ => self.panel.picked(folder),
                    };
                    self.watch_action(action, origin);
                }
            }
            Err(error) => append_log(
                &self.options.data_dir,
                &format!("the folder picker failed: {error}"),
            ),
        }
    }

    /// Saves a change of the settings: through the engine when it runs (it
    /// validates and owns the file), else after the same validation here.
    fn save_settings(&mut self, change: impl FnOnce(&mut Settings)) {
        let mut current = load_settings(&self.options.data_dir).unwrap_or_default();
        change(&mut current);
        let Ok(text) = settings::to_toml(&current) else {
            return;
        };
        if self.shell.link.is_connected() {
            let tag = self.link.send(Command::ApplySettings { toml: text });
            self.settings_tag = Some(tag);
            return;
        }
        let context = Context {
            data_dir: self.options.data_dir.to_string_lossy().into_owned(),
            export_dir: None,
        };
        match settings::load(&text, &context) {
            Loaded::Ok { .. } => {
                let path = self.options.data_dir.join("settings.toml");
                match peekbuddy_win::atomic::write_atomic(&path, text.as_bytes()) {
                    Ok(()) => {
                        self.settings_screen.on_result(
                            true,
                            "the engine is not running; it reads the file when it starts",
                            Some(&current),
                        );
                        self.theme_pending = Some(current.general.theme);
                        self.settings = current;
                    }
                    Err(error) => self.settings_screen.on_result(
                        false,
                        &format!("could not write settings.toml: {error}"),
                        None,
                    ),
                }
            }
            Loaded::TooNew { version, .. } => self.settings_screen.on_result(
                false,
                &format!("version {version} is newer than this build reads"),
                None,
            ),
            Loaded::Invalid(problems) => {
                let text = problems
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("\n");
                self.settings_screen.on_result(false, &text, None);
            }
        }
    }

    /// Fills the settings screen when its tab opens.
    fn open_settings(&mut self) {
        let settings = load_settings(&self.options.data_dir).unwrap_or_default();
        self.settings_screen.load(&settings);
        self.settings_screen
            .set_usage(directory_size(&self.options.segments));
    }

    fn settings_action(&mut self, action: SettingsAction) {
        match action {
            SettingsAction::Apply(edit) => self.save_settings(|settings| edit.apply_to(settings)),
            SettingsAction::OpenDataFolder => open_folder(&self.options.data_dir),
            SettingsAction::ShowDiagnostics => self.collect_diagnostics(),
            SettingsAction::ShowOnboarding => {
                self.onboarding = Some(Onboarding::reopened(
                    self.settings.general.autostart,
                    &self.settings.filters.presets,
                ));
            }
        }
    }

    fn onboarding_action(&mut self, action: OnboardingAction) {
        match action {
            OnboardingAction::Watch(action) => self.watch_action(action, Origin::Onboarding),
            OnboardingAction::Finish { autostart, presets } => {
                self.save_settings(|settings| {
                    settings.general.first_run = false;
                    settings.general.autostart = autostart;
                    settings.filters.presets = presets;
                });
                self.onboarding = None;
                self.tab = Tab::Log;
            }
        }
    }

    /// Runs `peekbuddy --diagnostics` (the text support asks for) on a thread
    /// and hands the text to the preview.
    fn collect_diagnostics(&mut self) {
        let (sender, receiver) = std::sync::mpsc::channel();
        let exe = self.options.exe.clone();
        let mut args = vec![std::ffi::OsString::from("--diagnostics")];
        if let Some(position) = self
            .options
            .engine_args
            .iter()
            .position(|arg| arg == "--data-dir")
        {
            args.extend(
                self.options
                    .engine_args
                    .iter()
                    .skip(position)
                    .take(2)
                    .cloned(),
            );
        }
        let waker = self.waker.clone();
        let started = std::thread::Builder::new()
            .name(String::from("diagnostics"))
            .spawn(move || {
                let text = std::process::Command::new(exe)
                    .args(args)
                    .output()
                    .map_or_else(
                        |error| format!("could not run the diagnostics: {error}"),
                        |output| String::from_utf8_lossy(&output.stdout).into_owned(),
                    );
                let _ = sender.send(text);
                waker();
            });
        if started.is_ok() {
            self.diagnostics = Some(receiver);
        } else {
            self.settings_screen
                .set_diagnostics(Some(String::from("could not start the diagnostics")));
        }
    }

    /// Takes the diagnostics text when it is ready.
    fn diagnostics_ready(&mut self) {
        let Some(receiver) = &self.diagnostics else {
            return;
        };
        match receiver.try_recv() {
            Ok(text) => {
                self.diagnostics = None;
                self.settings_screen.set_diagnostics(Some(text));
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.diagnostics = None;
                self.settings_screen.set_diagnostics(None);
            }
        }
    }

    /// Starts the engine again once a requested restart is due.
    fn restart_step(&mut self) {
        let Some(due) = self.restart_due else {
            return;
        };
        if Instant::now() >= due && matches!(self.shell.link, LinkState::Gone) {
            self.start_engine();
            self.restart_due = Some(Instant::now() + 2 * RESTART_DELAY);
        }
    }

    fn start_engine(&self) {
        // A failure shows as the banner staying where it is.
        let _ = std::process::Command::new(&self.options.exe)
            .args(&self.options.engine_args)
            .spawn();
        self.link.reconnect();
    }

    /// Changes `settings.toml` with `change` and saves it: through the engine
    /// when it runs (it owns the file), else directly. `what` names the change
    /// in the log when it fails.
    fn update_settings(&mut self, what: &str, change: impl FnOnce(&mut Settings)) {
        let Some(mut current) = load_settings(&self.options.data_dir) else {
            return;
        };
        let before = current.clone();
        change(&mut current);
        if current == before {
            return;
        }
        let Ok(text) = settings::to_toml(&current) else {
            return;
        };
        if self.shell.link.is_connected() {
            let tag = self
                .link
                .send(Command::ApplySettings { toml: text.clone() });
            if let Some((true, _)) = self.link.wait_result(tag, SAVE_WAIT) {
                return;
            }
        }
        let path = self.options.data_dir.join("settings.toml");
        if let Err(error) = peekbuddy_win::atomic::write_atomic(&path, text.as_bytes()) {
            append_log(
                &self.options.data_dir,
                &format!("could not remember {what}: {error}"),
            );
        }
    }

    /// Remembers the window.
    fn save_window(&mut self) {
        let rect = self.normal;
        let maximized = self.hwnd.is_some_and(chrome::is_maximized);
        let filter = self.view.filter_text();
        self.update_settings("the window", |settings| {
            if let Some(rect) = rect {
                settings.viewer = geometry::remember(settings.viewer.clone(), rect, maximized);
            }
            settings.viewer.filter = filter;
        });
    }

    /// Records who asked for this frame (the probe's idle window).
    fn note_causes(&self, ctx: &egui::Context) {
        let mut report = self.report.lock().unwrap_or_else(PoisonError::into_inner);
        for cause in ctx.repaint_causes() {
            if report.causes.len() < 32 {
                let key = format!("{}:{} {}", cause.file, cause.line, cause.reason);
                *report.causes.entry(key).or_insert(0) += 1;
            }
        }
        // egui repaints at once when it has input events: say which.
        ctx.input(|input| {
            for event in &input.events {
                let text = format!("{event:?}");
                let name = text.split(['(', ' ', '{']).next().unwrap_or_default();
                if report.causes.len() < 32 {
                    *report.causes.entry(format!("event {name}")).or_insert(0) += 1;
                }
            }
        });
    }

    fn write_report(&self) {
        let Some(probe) = &self.options.probe else {
            return;
        };
        let json = self
            .report
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .to_json(self.frames.load(Ordering::Relaxed));
        if let Some(path) = &probe.out {
            let _ = std::fs::write(path, json);
        } else {
            use std::io::Write as _;
            let _ = std::io::stdout().write_all(json.as_bytes());
        }
    }
}

impl eframe::App for ViewerApp {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if self.hwnd.is_none() {
            self.hwnd = window_handle(frame);
        }
        if let Some(hwnd) = self.hwnd {
            if !self.placed {
                self.placed = true;
                self.place_window(hwnd);
            }
            self.sync_title_bar(ctx, hwnd);
            self.track_window(hwnd);
        }
        while let Some(event) = self.link.poll() {
            self.handle(event);
        }
        self.view.logic();
        self.picked();
        self.diagnostics_ready();
        self.restart_step();
        if let Some(theme) = self.theme_pending.take() {
            ctx.set_theme(theme::preference(theme));
        }
        if self.show_requested.swap(false, Ordering::AcqRel) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.frames.fetch_add(1, Ordering::Relaxed);
        #[cfg(feature = "tracy")]
        if let Some(client) = tracy_client::Client::running() {
            client.frame_mark();
        }
        if self.idle_window.load(Ordering::Relaxed) {
            self.note_causes(ui.ctx());
        }
        let mut view_events = Vec::new();
        let mut watch_actions = Vec::new();
        let mut settings_actions = Vec::new();
        let mut onboarding_actions = Vec::new();
        let before = self.tab;
        let action = self.shell.show(ui, |ui| {
            if let Some(onboarding) = &mut self.onboarding {
                onboarding_actions = onboarding.show(ui, &self.panel.watches);
                return;
            }
            crate::watches::tabs(
                ui,
                &mut self.tab,
                self.panel.watches.len(),
                self.panel.attention(),
            );
            match self.tab {
                Tab::Log => view_events = self.view.show(ui),
                Tab::Watches => watch_actions = self.panel.show(ui),
                Tab::Settings => settings_actions = self.settings_screen.show(ui),
            }
        });
        if before != Tab::Settings && self.tab == Tab::Settings {
            self.open_settings();
        }
        if self.tab == Tab::Watches && self.shell.link.is_connected() {
            let now = Instant::now();
            if now >= self.next_refresh {
                self.next_refresh = now + WATCH_REFRESH;
                self.link.request_watches();
            }
            ui.ctx().request_repaint_after(WATCH_REFRESH);
        }
        for action in watch_actions {
            self.watch_action(action, Origin::Tab);
        }
        for action in settings_actions {
            self.settings_action(action);
        }
        for action in onboarding_actions {
            self.onboarding_action(action);
        }
        for event in view_events {
            match event {
                ViewEvent::ColumnsChanged(names) => {
                    self.update_settings("the columns", |settings| settings.view.columns = names);
                }
                ViewEvent::DetailsToggled(open) => {
                    self.update_settings("the details pane", |settings| {
                        settings.viewer.details_hidden = !open;
                    });
                }
                ViewEvent::Open { path, select } => open_in_explorer(path, select),
            }
        }
        match action {
            Some(ShellAction::StartEngine) => self.start_engine(),
            Some(ShellAction::Reconnect) => self.link.reconnect(),
            None => {}
        }
        // F7: ready is the first presented frame that already shows the
        // first page of rows (or knows there are none), not the first frame.
        if !self.first_frame_done && self.view.first_page_ready() {
            self.first_frame_done = true;
            let since = since_process_create_us();
            record(
                &self.options.table,
                Milestone::ViewerFirstFrame,
                PerfRole::Viewer,
                since,
            );
            let adapter = gpu::adapter(frame);
            if let Some(adapter) = &adapter {
                self.settings_screen.set_adapter(format!(
                    "{} / {} / {}",
                    adapter.name, adapter.backend, adapter.device_type
                ));
                let warning = if adapter.is_software() {
                    " (software rendering: WARP)"
                } else {
                    ""
                };
                append_log(
                    &self.options.data_dir,
                    &format!(
                        "viewer: adapter {} / {} / {}{warning}",
                        adapter.name, adapter.backend, adapter.device_type
                    ),
                );
            }
            let mut report = self.report.lock().unwrap_or_else(PoisonError::into_inner);
            report.first_frame_us = Some(since);
            report.adapter = adapter;
        }
    }

    fn on_exit(&mut self, #[cfg(target_arch = "x86_64")] _gl: Option<&eframe::glow::Context>) {
        // A running export is cancelled and its partial file removed.
        self.view.cancel_export();
        self.save_window();
        self.write_report();
    }
}

/// Shows a path in Explorer on a thread of its own (a slow share must not
/// stall the window): the item selected, or its folder. A deleted item opens
/// the nearest folder that still exists. Only attributes are looked at; the
/// item is never opened.
fn open_in_explorer(path: String, select: bool) {
    std::thread::spawn(move || {
        let exists = |candidate: &Path| std::fs::symlink_metadata(candidate).is_ok();
        let item = shellpath::plain(&path);
        let parameters = if select && exists(Path::new(&item)) {
            format!("/select,\"{item}\"")
        } else {
            let folder = if select {
                item.clone()
            } else {
                shellpath::parent(&item).map_or_else(|| item.clone(), str::to_owned)
            };
            match shellpath::nearest_existing(&folder, exists) {
                Some(found) => format!("\"{}\"", found.display()),
                None => return,
            }
        };
        let _ = peekbuddy_win::shell::explorer(&parameters);
    });
}

/// What the probe thread does: wait for the first frame, let the window
/// settle, count the frames painted while nothing happens, then close.
fn spawn_probe(
    ctx: egui::Context,
    probe: &Probe,
    (frames, idle_window): (Arc<AtomicU64>, Arc<AtomicBool>),
    report: Arc<Mutex<ProbeReport>>,
) {
    let seconds = probe.seconds;
    std::thread::spawn(move || {
        let give_up = Instant::now() + Duration::from_secs(seconds + 60);
        while frames.load(Ordering::Relaxed) == 0 {
            if Instant::now() > give_up {
                // No window ever painted (no desktop): fail instead of hang.
                std::process::exit(3);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        // The first frames run layout, AccessKit start-up and the first
        // page of whatever there is; the idle window starts after them.
        std::thread::sleep(Duration::from_millis(1500));
        let before = frames.load(Ordering::Relaxed);
        idle_window.store(true, Ordering::Relaxed);
        std::thread::sleep(Duration::from_secs(seconds));
        idle_window.store(false, Ordering::Relaxed);
        let after = frames.load(Ordering::Relaxed);
        {
            let mut report = report.lock().unwrap_or_else(PoisonError::into_inner);
            report.idle_seconds = seconds;
            report.idle_frames = Some(after.saturating_sub(before));
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    });
}

/// Waits for a second launch of the viewer and asks the window to come to
/// the front. Blocks in the kernel: an idle viewer has no wakeups from it.
fn spawn_show_listener(
    ctx: egui::Context,
    signal: Signal,
    requested: Arc<AtomicBool>,
) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name(String::from("show-viewer"))
        .spawn(move || {
            while let Ok(true) = signal.wait(None) {
                requested.store(true, Ordering::Release);
                ctx.request_repaint();
            }
        })
        .map(drop)
}

fn viewport(title: &str) -> egui::ViewportBuilder {
    // The real size and place are restored in `logic`, in pixels, because a
    // logical size here would be converted with the wrong monitor's scale.
    egui::ViewportBuilder::default()
        .with_title(title)
        .with_inner_size([1100.0, 700.0])
        .with_min_inner_size([640.0, 400.0])
        .with_icon(icon())
}

/// The window and taskbar icon: `assets/icon/peek.svg` of nobuddy.org, 64 px
/// as raw RGBA.
fn icon() -> egui::IconData {
    const SIZE: u32 = 64;
    egui::IconData {
        rgba: include_bytes!("../../../assets/icon/window-64.rgba").to_vec(),
        width: SIZE,
        height: SIZE,
    }
}

/// Runs the viewer window until it closes.
///
/// # Errors
///
/// The renderer or the window could not start: the user has been told in a
/// message box and the error is in `logs\viewer.log`.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the viewer owns its options for as long as it runs, as the callers pass them"
)]
pub fn run(options: Options) -> std::io::Result<()> {
    let settings = load_settings(&options.data_dir).unwrap_or_default();
    let data_dir = options.data_dir.clone();
    let mut failures = Vec::new();
    // The renderers in order of preference (ADR 0005): on x64 glow, then
    // wgpu when this machine has no OpenGL 2.0 driver (Remote Desktop without
    // GPU acceleration, a bare VM). A renderer that fails before the app is
    // created has not touched anything, so the next one gets a clean start.
    for &renderer in gpu::renderers() {
        let created = Arc::new(AtomicBool::new(false));
        let result = eframe::run_native(
            TITLE,
            gpu::native_options(renderer, viewport(&options.title)),
            creator(
                options.clone(),
                settings.clone(),
                Signal::open(&options.show_event)?,
                Arc::clone(&created),
            ),
        );
        match result {
            Ok(()) => return Ok(()),
            Err(error) if !created.load(Ordering::Acquire) => {
                let line = format!("{renderer} could not start: {error}");
                append_log(&data_dir, &format!("viewer: {line}"));
                failures.push(line);
            }
            Err(error) => return fail(&data_dir, &error.to_string()),
        }
    }
    fail(&data_dir, &failures.join("\n"))
}

/// The renderer or the window could not start: tells the user in a message
/// box and puts the error in the log.
fn fail(data_dir: &Path, error: &str) -> std::io::Result<()> {
    let text = format!("PeekBuddy could not open its window:\n\n{error}");
    append_log(data_dir, &format!("viewer failed to start: {error}"));
    chrome::error_box(TITLE, &text);
    Err(std::io::Error::other(error.to_owned()))
}

/// Builds the app once the renderer is up. `created` says that it ran, so a
/// failure before it can be retried with another renderer.
fn creator(
    options: Options,
    settings: Settings,
    show_signal: Signal,
    created: Arc<AtomicBool>,
) -> eframe::AppCreator<'static> {
    Box::new(move |cc| {
        created.store(true, Ordering::Release);
        cc.egui_ctx
            .set_theme(theme::preference(settings.general.theme));
        let frames = Arc::new(AtomicU64::new(0));
        let idle_window = Arc::new(AtomicBool::new(false));
        let report = Arc::new(Mutex::new(ProbeReport {
            idle_seconds: 0,
            ..ProbeReport::default()
        }));
        let show_requested = Arc::new(AtomicBool::new(false));
        spawn_show_listener(
            cc.egui_ctx.clone(),
            show_signal,
            Arc::clone(&show_requested),
        )?;
        if let Some(probe) = &options.probe {
            spawn_probe(
                cc.egui_ctx.clone(),
                probe,
                (Arc::clone(&frames), Arc::clone(&idle_window)),
                Arc::clone(&report),
            );
        }
        let waker: Arc<dyn Fn() + Send + Sync> = {
            let ctx = cc.egui_ctx.clone();
            Arc::new(move || ctx.request_repaint())
        };
        let app_waker = Arc::clone(&waker);
        let view = log_view(&options, &settings, &waker)?;
        let (settings_screen, onboarding) = first_screens(&options, &settings);
        let link = Link::start(
            PipeConnector {
                name: options.pipe_name.clone(),
                exe: options.exe.clone(),
            },
            Timing::default(),
            waker,
            format!("peekbuddy {}", env!("CARGO_PKG_VERSION")),
        )?;
        Ok(Box::new(ViewerApp {
            options,
            shell: Shell::default(),
            link,
            view,
            tab: Tab::default(),
            panel: WatchPanel::default(),
            settings_screen,
            onboarding,
            settings_tag: None,
            theme_pending: None,
            diagnostics: None,
            picker_origin: Origin::Tab,
            watch_tags: HashMap::new(),
            next_refresh: Instant::now(),
            restart_due: None,
            picker: None,
            waker: Arc::clone(&app_waker),
            settings,
            show_requested,
            frames,
            idle_window,
            hwnd: None,
            placed: false,
            normal: None,
            title_dark: None,
            first_frame_done: false,
            report,
        }))
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{ProbeReport, json_string};
    use crate::gpu::Adapter;

    #[test]
    fn a_probe_report_is_one_json_line() {
        let report = ProbeReport {
            first_frame_us: Some(123_456),
            idle_seconds: 10,
            idle_frames: Some(0),
            adapter: Some(Adapter {
                name: String::from("Microsoft \"Basic\" Render Driver"),
                backend: String::from("Dx12"),
                device_type: String::from("Cpu"),
            }),
            causes: BTreeMap::from([(String::from("a.rs:1 \"x\""), 2)]),
        };
        let json = report.to_json(7);
        assert!(json.ends_with("}\n"));
        assert!(json.contains("\"frames\":7"), "{json}");
        assert!(json.contains("\"first_frame_us\":123456"), "{json}");
        assert!(json.contains("\"idle_frames\":0"), "{json}");
        assert!(json.contains("\"backend\":\"Dx12\""), "{json}");
        assert!(json.contains("\\\"Basic\\\""), "{json}");
        assert!(
            json.contains("\"causes\":{\"a.rs:1 \\\"x\\\"\":2}"),
            "{json}"
        );
        assert_eq!(json.lines().count(), 1);
    }

    #[test]
    fn an_empty_report_has_nulls_and_escapes_control_characters() {
        let json = ProbeReport::default().to_json(0);
        assert!(json.contains("\"idle_frames\":null"), "{json}");
        assert!(json.contains("\"adapter\":null"), "{json}");
        assert!(json.contains("\"causes\":{}"), "{json}");
        assert_eq!(json_string("a\tb\\"), "\"a\\u0009b\\\\\"");
    }
}
