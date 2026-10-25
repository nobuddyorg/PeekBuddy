//! Spike #33: viewer renderer and footprint.
//!
//! One scenario per process so the memory numbers are clean:
//!
//! `viewer-bench --scenario first-frame|idle|scroll|tail|wiggle|engine
//!  [--secs N] [--latency-ms N] [--label X] [--out FILE]`
//!
//! The renderer is chosen at build time (`--features wgpu` is the default,
//! `--no-default-features --features glow` builds the glow variant). Input is
//! injected into egui's raw input (no SendInput), so the scripted scenarios
//! work on runners without a foreground window.
//!
//! Result: one JSON line on stdout (and appended to `--out`).

use std::collections::HashMap;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use eframe::egui;
use egui_table::{CellInfo, Column, HeaderCellInfo, HeaderRow, PrefetchInfo, Table, TableDelegate};
use serde_json::{Value, json};

const ROWS: u64 = 10_000_000;
const PAGE: u64 = 200;
const MAX_PAGES: usize = 16;

// ------------------------------------------------------------- platform

#[cfg(windows)]
mod sys {
    use std::mem::zeroed;
    use windows_sys::Win32::Foundation::{FILETIME, HANDLE};
    use windows_sys::Win32::System::ProcessStatus::{
        EnumProcessModules, GetModuleFileNameExW, GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    use windows_sys::Win32::System::WindowsProgramming::QueryProcessCycleTime;

    fn ft(f: FILETIME) -> u64 {
        (u64::from(f.dwHighDateTime) << 32) | u64::from(f.dwLowDateTime)
    }

    pub fn memory() -> (u64, u64) {
        // SAFETY: plain data; `cb` set to the structure size as the API requires.
        let mut c: PROCESS_MEMORY_COUNTERS_EX = unsafe { zeroed() };
        c.cb = size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
        // SAFETY: valid pseudo handle and a buffer of the size passed.
        let ok = unsafe { GetProcessMemoryInfo(GetCurrentProcess(), (&mut c as *mut PROCESS_MEMORY_COUNTERS_EX).cast(), c.cb) };
        if ok == 0 { (0, 0) } else { (c.PrivateUsage as u64, c.WorkingSetSize as u64) }
    }

    /// (creation time, kernel+user time) in 100 ns ticks since 1601, 100 ns.
    pub fn times() -> (u64, u64) {
        // SAFETY: plain data.
        let (mut c, mut e, mut k, mut u): (FILETIME, FILETIME, FILETIME, FILETIME) = unsafe { zeroed() };
        // SAFETY: valid pseudo handle and out pointers.
        unsafe { GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) };
        (ft(c), ft(k) + ft(u))
    }

    pub fn now_filetime() -> u64 {
        // SAFETY: plain data and valid out pointer.
        let mut f: FILETIME = unsafe { zeroed() };
        unsafe { windows_sys::Win32::System::SystemInformation::GetSystemTimeAsFileTime(&mut f) };
        ft(f)
    }

    pub fn cycles() -> u64 {
        let mut c = 0u64;
        // SAFETY: valid pseudo handle and out pointer.
        unsafe { QueryProcessCycleTime(GetCurrentProcess(), &mut c) };
        c
    }

    pub fn modules() -> Vec<String> {
        let mut mods = vec![std::ptr::null_mut(); 1024];
        let mut needed = 0u32;
        // SAFETY: the buffer is valid for the byte length passed.
        let ok = unsafe {
            EnumProcessModules(GetCurrentProcess(), mods.as_mut_ptr(), (mods.len() * size_of::<HANDLE>()) as u32, &mut needed)
        };
        if ok == 0 {
            return Vec::new();
        }
        let n = (needed as usize / size_of::<HANDLE>()).min(mods.len());
        let mut out = Vec::new();
        for m in mods.iter().take(n) {
            let mut buf = [0u16; 520];
            // SAFETY: the buffer is valid for 520 units.
            let len = unsafe { GetModuleFileNameExW(GetCurrentProcess(), *m, buf.as_mut_ptr(), 520) } as usize;
            let path = String::from_utf16_lossy(buf.get(..len).unwrap_or(&[]));
            out.push(path.rsplit('\\').next().unwrap_or("").to_lowercase());
        }
        out
    }
}

#[cfg(not(windows))]
mod sys {
    pub fn memory() -> (u64, u64) {
        (0, 0)
    }
    pub fn times() -> (u64, u64) {
        (0, 0)
    }
    pub fn now_filetime() -> u64 {
        0
    }
    pub fn cycles() -> u64 {
        0
    }
    pub fn modules() -> Vec<String> {
        Vec::new()
    }
}

// ----------------------------------------------------------------- data

#[derive(Clone)]
struct Row {
    id: u64,
    ts: u64,
    kind: &'static str,
    root: u64,
    path: String,
}

const KINDS: [&str; 6] = ["Created", "Modified", "Deleted", "RenamedFrom", "RenamedTo", "Attrib"];

fn splitmix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

fn make_row(id: u64) -> Row {
    let h = splitmix(id);
    Row {
        id,
        ts: 133_000_000_000_000_000 + id * 25_000,
        kind: KINDS[(h % 6) as usize],
        root: (h >> 8) % 10,
        path: format!("C:\\Users\\user\\project-{}\\src\\module-{}\\file-{}.rs", (h >> 16) % 50, (h >> 24) % 200, (h >> 32) % 5000),
    }
}

fn spawn_worker(ctx: egui::Context, latency: Duration) -> (mpsc::Sender<u64>, mpsc::Receiver<(u64, Vec<Row>)>) {
    let (req_tx, req_rx) = mpsc::channel::<u64>();
    let (res_tx, res_rx) = mpsc::channel();
    std::thread::spawn(move || {
        while let Ok(page) = req_rx.recv() {
            std::thread::sleep(latency);
            let rows = (page * PAGE..(page + 1) * PAGE).map(make_row).collect();
            if res_tx.send((page, rows)).is_err() {
                break;
            }
            ctx.request_repaint();
        }
    });
    (req_tx, res_rx)
}

// ------------------------------------------------------------------ app

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Scenario {
    FirstFrame,
    Idle,
    Scroll,
    Tail,
    Wiggle,
}

struct Delegate {
    pages: HashMap<u64, Vec<Row>>,
    order: Vec<u64>,
    requested: std::collections::HashSet<u64>,
    req: mpsc::Sender<u64>,
    res: mpsc::Receiver<(u64, Vec<Row>)>,
    first_data: bool,
    allocs_hint: u64,
}

impl TableDelegate for Delegate {
    fn prepare(&mut self, info: &PrefetchInfo) {
        while let Ok((page, rows)) = self.res.try_recv() {
            self.requested.remove(&page);
            self.pages.insert(page, rows);
            self.order.push(page);
            if self.order.len() > MAX_PAGES {
                let old = self.order.remove(0);
                self.pages.remove(&old);
            }
        }
        let first = info.visible_rows.start / PAGE;
        let last = info.visible_rows.end.saturating_sub(1) / PAGE;
        for page in first..=last {
            if !self.pages.contains_key(&page) && self.requested.insert(page) {
                let _ = self.req.send(page);
            }
        }
    }

    fn header_cell_ui(&mut self, ui: &mut egui::Ui, cell: &HeaderCellInfo) {
        let names = ["Id", "Time", "Kind", "Root", "Path"];
        ui.label(names.get(cell.col_range.start).copied().unwrap_or(""));
    }

    fn cell_ui(&mut self, ui: &mut egui::Ui, cell: &CellInfo) {
        let page = cell.row_nr / PAGE;
        let Some(rows) = self.pages.get(&page) else {
            ui.label("...");
            return;
        };
        let Some(row) = rows.get((cell.row_nr % PAGE) as usize) else { return };
        self.first_data = true;
        self.allocs_hint += 1;
        match cell.col_nr {
            0 => ui.label(row.id.to_string()),
            1 => ui.label(row.ts.to_string()),
            2 => ui.label(row.kind),
            3 => ui.label(row.root.to_string()),
            _ => ui.label(&row.path),
        };
    }

    fn default_row_height(&self) -> f32 {
        20.0
    }
}

struct Bench {
    scenario: Scenario,
    label: String,
    renderer: &'static str,
    out: Option<String>,
    secs: f64,
    delegate: Delegate,
    started: Instant,
    first_frame_ms: Option<f64>,
    first_data_ms: Option<f64>,
    adapter: String,
    ui_us: Vec<f32>,
    interval_us: Vec<f32>,
    last_logic: Option<Instant>,
    frames: u64,
    logic_calls: u64,
    mem_start: (u64, u64),
    cpu_start: u64,
    cyc_start: u64,
    phase: usize,
    phase_marks: Vec<Value>,
    phase_start: Option<(Instant, u64, u64, u64, u64)>,
    jump: Option<u64>,
    jump_clock: f64,
    tail_rows: u64,
    done: bool,
}

impl Bench {
    fn elapsed(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    fn mark_phase(&mut self, name: &str) {
        let now = (Instant::now(), sys::times().1, sys::cycles(), self.frames, self.logic_calls);
        if let Some(prev) = self.phase_start {
            let secs = now.0.duration_since(prev.0).as_secs_f64();
            self.phase_marks.push(json!({
                "phase": name,
                "secs": secs,
                "cpu_ms": (now.1 - prev.1) as f64 / 10_000.0,
                "cycles": now.2 - prev.2,
                "frames": now.3 - prev.3,
                "logic_calls": now.4 - prev.4,
                "frames_per_s": (now.3 - prev.3) as f64 / secs,
            }));
        }
        self.phase_start = Some(now);
    }

    fn finish(&mut self, ctx: &egui::Context) {
        if self.done {
            return;
        }
        self.done = true;
        let result = self.report();
        println!("{result}");
        if let Some(path) = &self.out {
            use std::io::Write as _;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = writeln!(f, "{result}");
            }
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn report(&mut self) -> Value {
        let mem = sys::memory();
        let cpu = sys::times().1 - self.cpu_start;
        let stats = |v: &mut Vec<f32>| -> Value {
            v.sort_by(|a, b| a.total_cmp(b));
            let p = |q: f64| v.get(((v.len().max(1) - 1) as f64 * q).round() as usize).copied().unwrap_or(f32::NAN) / 1000.0;
            json!({ "n": v.len(), "p50_ms": p(0.5), "p95_ms": p(0.95), "p99_ms": p(0.99), "max_ms": p(1.0),
                    "over_16ms": v.iter().filter(|x| **x > 16_000.0).count(), "over_33ms": v.iter().filter(|x| **x > 33_000.0).count() })
        };
        let mut ui = std::mem::take(&mut self.ui_us);
        let mut iv = std::mem::take(&mut self.interval_us);
        json!({
            "experiment": "viewer", "scenario": format!("{:?}", self.scenario), "renderer": self.renderer, "label": self.label,
            "adapter": self.adapter,
            "exe_bytes": std::env::current_exe().ok().and_then(|p| std::fs::metadata(p).ok()).map_or(0, |m| m.len()),
            "first_frame_ms": self.first_frame_ms, "first_data_frame_ms": self.first_data_ms,
            "private_bytes_start": self.mem_start.0, "private_bytes_end": mem.0,
            "working_set_start": self.mem_start.1, "working_set_end": mem.1,
            "cpu_ms": cpu as f64 / 10_000.0, "cycles": sys::cycles() - self.cyc_start,
            "frames": self.frames, "logic_calls": self.logic_calls,
            "ui_time": stats(&mut ui), "frame_interval": stats(&mut iv),
            "cells_drawn": self.delegate.allocs_hint,
            "phases": self.phase_marks,
            "modules": sys::modules(),
        })
    }
}

impl eframe::App for Bench {
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        let t = self.elapsed();
        let center = egui::pos2(400.0, 300.0);
        match self.scenario {
            Scenario::Scroll => {
                raw.events.push(egui::Event::PointerMoved(center));
                // 2,500 rows/s fling, i.e. 50,000 points/s at 20 points per row.
                let dt = 1.0 / 60.0;
                raw.events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, -(50_000.0 * dt) as f32),
                    phase: egui::TouchPhase::Move,
                    source: egui::MouseWheelSource::Unknown,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            Scenario::Wiggle => {
                let x = 300.0 + 150.0 * (t * 6.0).sin() as f32;
                let y = 300.0 + 150.0 * (t * 4.0).cos() as f32;
                raw.events.push(egui::Event::PointerMoved(egui::pos2(x, y)));
            }
            _ => {}
        }
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.logic_calls += 1;
        let now = Instant::now();
        if let Some(last) = self.last_logic.replace(now) {
            self.interval_us.push(now.duration_since(last).as_micros() as f32);
        }
        let t = self.elapsed();
        match self.scenario {
            Scenario::Idle => {
                // Three phases: open, minimized, hidden.
                let third = self.secs / 3.0;
                let want = (t / third) as usize;
                if want != self.phase && want < 3 {
                    let names = ["open", "minimized", "hidden"];
                    self.mark_phase(names.get(self.phase).copied().unwrap_or(""));
                    self.phase = want;
                    match want {
                        1 => ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)),
                        2 => {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                        }
                        _ => {}
                    }
                }
                if t >= self.secs {
                    self.mark_phase("hidden");
                    self.finish(ctx);
                }
            }
            Scenario::Scroll => {
                ctx.request_repaint();
                if t >= self.jump_clock + 2.0 {
                    self.jump_clock = t;
                    self.jump = Some(splitmix(t.to_bits()) % (ROWS - 100));
                }
                if t >= self.secs {
                    self.finish(ctx);
                }
            }
            Scenario::Tail => {
                self.tail_rows = ROWS + (t * 5000.0) as u64;
                ctx.request_repaint_after(Duration::from_millis(16));
                if t >= self.secs {
                    self.finish(ctx);
                }
            }
            Scenario::Wiggle => {
                ctx.request_repaint_after(Duration::from_millis(8));
                if t >= self.secs {
                    self.finish(ctx);
                }
            }
            Scenario::FirstFrame => {
                if self.first_data_ms.is_some() && t > 2.0 {
                    self.finish(ctx);
                }
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let t0 = Instant::now();
        self.frames += 1;
        FRAMES_SEEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if self.first_frame_ms.is_none() {
            self.adapter = adapter_name(frame);
        }
        let rows = if self.scenario == Scenario::Tail { self.tail_rows.max(ROWS) } else { ROWS };
        let mut table = Table::new()
            .id_salt("events")
            .num_rows(rows)
            .columns(vec![Column::new(90.0), Column::new(170.0), Column::new(110.0), Column::new(60.0), Column::new(420.0)])
            .headers(vec![HeaderRow::new(22.0)]);
        if self.scenario == Scenario::Tail {
            table = table.stick_to_bottom(true);
        }
        if let Some(row) = self.jump.take() {
            table = table.scroll_to_row(row, Some(egui::Align::TOP));
        }
        table.show(ui, &mut self.delegate);
        let spent = t0.elapsed();
        self.ui_us.push(spent.as_micros() as f32);
        if self.first_frame_ms.is_none() {
            self.first_frame_ms = Some(process_age_ms());
        }
        if self.first_data_ms.is_none() && self.delegate.first_data {
            self.first_data_ms = Some(process_age_ms());
        }
    }
}

static FRAMES_SEEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A window that never gets a frame (no desktop on the runner) would hang the
/// run; the watchdog turns that into a result row and a non-zero exit.
fn spawn_watchdog(limit: Duration, scenario: String, label: String, out: Option<String>) {
    std::thread::spawn(move || {
        std::thread::sleep(limit);
        let v = json!({
            "experiment": "viewer", "scenario": scenario, "label": label, "watchdog": true,
            "after_s": limit.as_secs_f64(),
            "frames": FRAMES_SEEN.load(std::sync::atomic::Ordering::Relaxed),
            "process_age_ms": process_age_ms(),
        });
        println!("{v}");
        if let Some(p) = out {
            use std::io::Write as _;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
                let _ = writeln!(f, "{v}");
            }
        }
        std::process::exit(3);
    });
}

fn process_age_ms() -> f64 {
    let (created, _) = sys::times();
    if created == 0 {
        return f64::NAN;
    }
    (sys::now_filetime().saturating_sub(created)) as f64 / 10_000.0
}

#[cfg(feature = "wgpu")]
fn adapter_name(frame: &eframe::Frame) -> String {
    frame
        .wgpu_render_state()
        .map(|s| {
            let i = s.adapter.get_info();
            format!("{} / {:?} / {:?}", i.name, i.backend, i.device_type)
        })
        .unwrap_or_default()
}

#[cfg(not(feature = "wgpu"))]
fn adapter_name(frame: &eframe::Frame) -> String {
    frame.gl().map(|gl| {
        use eframe::glow::HasContext as _;
        // SAFETY: querying a string of the current GL context.
        unsafe { gl.get_parameter_string(eframe::glow::RENDERER) }
    }).unwrap_or_default()
}

// ----------------------------------------------------------------- main

fn arg(args: &[String], key: &str) -> Option<String> {
    args.iter().position(|a| a == key).and_then(|i| args.get(i + 1)).cloned()
}

fn engine_role(secs: f64, label: &str, out: Option<&str>) {
    let start = sys::memory();
    let t = Instant::now();
    std::thread::sleep(Duration::from_secs_f64(secs));
    let end = sys::memory();
    let v = json!({
        "experiment": "viewer", "scenario": "engine-role", "label": label,
        "private_bytes_start": start.0, "private_bytes_end": end.0, "working_set_end": end.1,
        "secs": t.elapsed().as_secs_f64(),
        "exe_bytes": std::env::current_exe().ok().and_then(|p| std::fs::metadata(p).ok()).map_or(0, |m| m.len()),
        "modules": sys::modules(),
    });
    println!("{v}");
    if let Some(p) = out {
        use std::io::Write as _;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
            let _ = writeln!(f, "{v}");
        }
    }
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().collect();
    let scenario = arg(&args, "--scenario").unwrap_or_else(|| "first-frame".into());
    let secs: f64 = arg(&args, "--secs").and_then(|s| s.parse().ok()).unwrap_or(20.0);
    let latency: u64 = arg(&args, "--latency-ms").and_then(|s| s.parse().ok()).unwrap_or(1);
    let label = arg(&args, "--label").unwrap_or_default();
    let out = arg(&args, "--out");
    if scenario == "engine" {
        engine_role(secs, &label, out.as_deref());
        return Ok(());
    }
    let sc = match scenario.as_str() {
        "idle" => Scenario::Idle,
        "scroll" => Scenario::Scroll,
        "tail" => Scenario::Tail,
        "wiggle" => Scenario::Wiggle,
        _ => Scenario::FirstFrame,
    };

    spawn_watchdog(Duration::from_secs_f64(secs.max(10.0) + 45.0), scenario.clone(), label.clone(), out.clone());
    let mut options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([900.0, 600.0]).with_title("viewer-bench"),
        ..Default::default()
    };
    #[cfg(feature = "wgpu")]
    {
        use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetup, WgpuSetupCreateNew, wgpu};
        options.renderer = eframe::Renderer::Wgpu;
        let mut create = WgpuSetupCreateNew::without_display_handle();
        create.instance_descriptor.backends = wgpu::Backends::DX12;
        create.instance_descriptor.flags = wgpu::InstanceFlags::empty();
        options.wgpu_options = WgpuConfiguration { wgpu_setup: WgpuSetup::CreateNew(create), ..Default::default() };
    }
    #[cfg(not(feature = "wgpu"))]
    {
        options.renderer = eframe::Renderer::Glow;
    }
    let renderer = if cfg!(feature = "wgpu") { "wgpu-dx12" } else { "glow" };

    eframe::run_native(
        "viewer-bench",
        options,
        Box::new(move |cc| {
            let (req, res) = spawn_worker(cc.egui_ctx.clone(), Duration::from_millis(latency));
            // eframe repaints only on events. These two scenarios request no
            // repaints themselves (idle must stay idle), so a helper thread
            // wakes the loop at the phase boundaries and the end; without it
            // they never reached their finish condition (quick run, 25 Oct).
            if matches!(sc, Scenario::Idle | Scenario::FirstFrame) {
                let ctx = cc.egui_ctx.clone();
                let (wakes, gap) = if sc == Scenario::Idle { (3, Duration::from_secs_f64(secs / 3.0 + 0.05)) } else { (20, Duration::from_millis(500)) };
                std::thread::spawn(move || {
                    for _ in 0..wakes {
                        std::thread::sleep(gap);
                        ctx.request_repaint();
                    }
                });
            }
            let mem_start = sys::memory();
            Ok(Box::new(Bench {
                scenario: sc,
                label,
                renderer,
                out,
                secs,
                delegate: Delegate { pages: HashMap::new(), order: Vec::new(), requested: Default::default(), req, res, first_data: false, allocs_hint: 0 },
                started: Instant::now(),
                first_frame_ms: None,
                first_data_ms: None,
                adapter: String::new(),
                ui_us: Vec::new(),
                interval_us: Vec::new(),
                last_logic: None,
                frames: 0,
                logic_calls: 0,
                mem_start,
                cpu_start: sys::times().1,
                cyc_start: sys::cycles(),
                phase: 0,
                phase_marks: Vec::new(),
                phase_start: Some((Instant::now(), sys::times().1, sys::cycles(), 0, 0)),
                jump: None,
                jump_clock: 0.0,
                tail_rows: ROWS,
                done: false,
            }))
        }),
    )
}
