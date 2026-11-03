//! The eframe application (#65). Windows only: the window and the renderer
//! come from eframe, the frame and the state it shows from [`crate::shell`]
//! and [`crate::link`], which build and test on every host.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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

use crate::geometry::{self, PxRect};
use crate::gpu::{self, Adapter};
use crate::link::{Link, LinkEvent, Timing};
use crate::link_win::PipeConnector;
use crate::shell::{Shell, ShellAction};
use crate::{Options, Probe, theme};

/// How long the close handler waits for the engine to take the settings.
const SAVE_WAIT: Duration = Duration::from_secs(1);

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
            LinkEvent::State(state) => self.shell.link = state,
            LinkEvent::Result { .. } | LinkEvent::Pushed(_) => {}
        }
    }

    fn start_engine(&self) {
        // A failure shows as the banner staying where it is.
        let _ = std::process::Command::new(&self.options.exe)
            .args(&self.options.engine_args)
            .spawn();
        self.link.reconnect();
    }

    /// Remembers the window in `settings.toml`: through the engine when it
    /// runs (it owns the file), else directly.
    fn save_window(&mut self) {
        let Some(rect) = self.normal else { return };
        let maximized = self.hwnd.is_some_and(chrome::is_maximized);
        let Some(mut current) = load_settings(&self.options.data_dir) else {
            return;
        };
        let remembered = geometry::remember(current.viewer.clone(), rect, maximized);
        if remembered == current.viewer {
            return;
        }
        current.viewer = remembered;
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
                &format!("could not remember the window: {error}"),
            );
        }
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
        if self.show_requested.swap(false, Ordering::AcqRel) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.frames.fetch_add(1, Ordering::Relaxed);
        if self.idle_window.load(Ordering::Relaxed) {
            self.note_causes(ui.ctx());
        }
        let action = self.shell.show(ui, |ui| {
            ui.centered_and_justified(|ui| {
                ui.label("The log appears here.");
            });
        });
        match action {
            Some(ShellAction::StartEngine) => self.start_engine(),
            Some(ShellAction::Reconnect) => self.link.reconnect(),
            None => {}
        }
        if !self.first_frame_done {
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

    fn on_exit(&mut self) {
        self.save_window();
        self.write_report();
    }
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
        .with_icon(egui::IconData::default())
}

/// Runs the viewer window until it closes.
///
/// # Errors
///
/// The renderer or the window could not start: the user has been told in a
/// message box and the error is in `logs\viewer.log`.
pub fn run(options: Options) -> std::io::Result<()> {
    let settings = load_settings(&options.data_dir).unwrap_or_default();
    let show_signal = Signal::open(&options.show_event)?;
    let data_dir = options.data_dir.clone();
    let native = gpu::native_options(viewport(&options.title));
    let result = eframe::run_native(
        TITLE,
        native,
        Box::new(move |cc| {
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
            let waker = {
                let ctx = cc.egui_ctx.clone();
                Arc::new(move || ctx.request_repaint())
            };
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
        }),
    );
    match result {
        Ok(()) => Ok(()),
        Err(error) => {
            let text = format!("PeekBuddy could not open its window:\n\n{error}");
            append_log(&data_dir, &format!("viewer failed to start: {error}"));
            chrome::error_box(TITLE, &text);
            Err(std::io::Error::other(error.to_string()))
        }
    }
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
