//! Frame time of the log grid under a scripted scroll (#347, F9, F11).
//!
//! The UI pass of the log screen runs frame after frame on a 10M-row log
//! while a scripted mouse wheel sweeps the grid down and back up, pages
//! arriving from the worker thread as the island moves. Criterion measures
//! one number per bench, so the two benches below measure a percentile of
//! the frame time: each sample runs `iters` frames and reports the 95th
//! (or 99th) percentile of them as the time of one iteration. The A/B gate
//! of `cargo xtask bench-ab` compares them like any other bench.
//!
//! The frame is the UI pass only, with the textures dropped: the hosted
//! runners have no GPU, so the rasteriser is not part of the number (F12).

use std::sync::Arc;
use std::time::{Duration, Instant};

use criterion::{Criterion, criterion_group, criterion_main};
use peekbuddy_viewer::filterspec::FilterSpec;
use peekbuddy_viewer::source::fake::FakeLog;
use peekbuddy_viewer::view::LogView;

/// Frames of one sweep in one direction.
const SWEEP: u64 = 400;

struct Screen {
    ctx: egui::Context,
    view: LogView,
    time: f64,
    frames: u64,
}

impl Screen {
    fn new() -> Option<Self> {
        let view = LogView::new(
            FakeLog::new(10_000_000),
            Arc::new(|_| 0),
            Arc::new(|| {}),
            FilterSpec::default(),
            vec![String::from(r"C:\w")],
            &["time", "kind", "path"].map(String::from),
        )
        .ok()?;
        let mut screen = Self {
            ctx: egui::Context::default(),
            view,
            time: 0.0,
            frames: 0,
        };
        // The pointer over the grid and the first page in.
        screen.frame(vec![egui::Event::PointerMoved(egui::pos2(500.0, 250.0))]);
        for _ in 0..50 {
            screen.frame(Vec::new());
            std::thread::sleep(Duration::from_millis(2));
        }
        Some(screen)
    }

    fn frame(&mut self, events: Vec<egui::Event>) {
        self.time += 1.0 / 60.0;
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1100.0, 520.0),
            )),
            time: Some(self.time),
            events,
            ..egui::RawInput::default()
        };
        let view = &mut self.view;
        let mut output = self.ctx.run_ui(raw, |ui| {
            view.logic();
            let _ = view.show(ui);
        });
        // The renderer would upload the textures; here nobody does.
        output.textures_delta.clear();
        self.frames += 1;
    }

    /// One frame of the script: three lines of wheel per frame, down for
    /// `SWEEP` frames, then up again.
    fn scroll_frame(&mut self) -> Duration {
        let down = (self.frames / SWEEP).is_multiple_of(2);
        let lines = if down { -3.0 } else { 3.0 };
        let events = vec![egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, lines),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
            modifiers: egui::Modifiers::NONE,
        }];
        let start = Instant::now();
        self.frame(events);
        start.elapsed()
    }
}

/// The time at `permille` thousandths of the sorted frame times.
fn percentile(sorted: &[Duration], permille: usize) -> Duration {
    let last = sorted.len().saturating_sub(1);
    sorted
        .get((last * permille).div_ceil(1000).min(last))
        .copied()
        .unwrap_or_default()
}

fn scroll(c: &mut Criterion) {
    let Some(mut screen) = Screen::new() else {
        return;
    };
    let mut group = c.benchmark_group("viewer");
    for (name, permille) in [("scroll_frame_p95", 950), ("scroll_frame_p99", 990)] {
        group.bench_function(name, |b| {
            b.iter_custom(|iters| {
                let mut frames: Vec<Duration> = (0..iters).map(|_| screen.scroll_frame()).collect();
                frames.sort();
                let per_frame = percentile(&frames, permille);
                per_frame * u32::try_from(iters).unwrap_or(u32::MAX)
            });
        });
    }
    group.finish();
}

criterion_group!(benches, scroll);
criterion_main!(benches);
