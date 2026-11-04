//! Heap allocations of the log screen per frame while scrolling (#66, F10,
//! #268).
//!
//! The UI pass of the log view runs frame after frame on a 10M-row log while
//! a scripted wheel moves the grid up and down inside the rows it holds, and
//! the allocations of this thread are counted. The same frames without the
//! view are counted too and subtracted, so what is left is what the viewer
//! and egui do for the screen, not what the test builds to drive it. Pages
//! arrive on the worker thread and are not part of a steady-state frame: the
//! test waits until the island stops growing before it counts.
//! `harness = false`: libtest itself allocates.

use std::alloc::System;
use std::io::Write as _;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use peekbuddy_alloc::{CountingAlloc, answer_nextest_list, measure};
use peekbuddy_viewer::filterspec::FilterSpec;
use peekbuddy_viewer::source::fake::FakeLog;
use peekbuddy_viewer::view::LogView;

#[global_allocator]
static ALLOC: CountingAlloc<System> = CountingAlloc::new(System);

/// The budget `alloc.viewer_per_frame` in `perf/budgets.toml`.
const BUDGET: u64 = 1100;
const FRAMES: u64 = 200;

struct Screen {
    ctx: egui::Context,
    time: f64,
}

impl Screen {
    fn raw(&mut self, events: Vec<egui::Event>) -> egui::RawInput {
        self.time += 1.0 / 60.0;
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1100.0, 520.0),
            )),
            time: Some(self.time),
            events,
            ..egui::RawInput::default()
        }
    }

    fn wheel(lines: f32) -> Vec<egui::Event> {
        vec![egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, lines),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
            modifiers: egui::Modifiers::NONE,
        }]
    }

    fn frame(&mut self, view: &mut LogView, events: Vec<egui::Event>) {
        let raw = self.raw(events);
        let mut output = self.ctx.run_ui(raw, |ui| {
            view.logic();
            let _ = view.show(ui);
        });
        // The renderer would upload the textures; here nobody does.
        output.textures_delta.clear();
    }

    fn empty_frame(&mut self, events: Vec<egui::Event>) {
        let raw = self.raw(events);
        let mut output = self.ctx.run_ui(raw, |_ui| {});
        output.textures_delta.clear();
    }
}

fn main() -> ExitCode {
    if answer_nextest_list("alloc_viewer") {
        return ExitCode::SUCCESS;
    }
    let log = FakeLog::new(10_000_000);
    let Ok(mut view) = LogView::new(
        log,
        Arc::new(|_| 0),
        Arc::new(|| {}),
        FilterSpec::default(),
        vec![String::from(r"C:\w")],
        &["time", "kind", "path"].map(String::from),
    ) else {
        return ExitCode::FAILURE;
    };
    let mut screen = Screen {
        ctx: egui::Context::default(),
        time: 0.0,
    };
    // Put the pointer over the grid, then scroll down through ten pages and
    // wait until the island stops growing.
    screen.frame(
        &mut view,
        vec![egui::Event::PointerMoved(egui::pos2(500.0, 250.0))],
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut steady = 0;
    let mut last = 0;
    while steady < 30 {
        screen.frame(&mut view, Screen::wheel(-3.0));
        std::thread::sleep(Duration::from_millis(2));
        let held = view.held_rows();
        steady = if held == last && held > 0 {
            steady + 1
        } else {
            0
        };
        last = held;
        if Instant::now() > deadline {
            let _ = writeln!(std::io::stderr(), "alloc_viewer: the island never settled");
            return ExitCode::FAILURE;
        }
        if screen.time > 60.0 * 5.0 {
            break;
        }
    }
    // Back and forth inside the rows that are held: one line down, one up.
    let mut direction = -1.0f32;
    let with_view = measure(30, FRAMES, || {
        direction = -direction;
        screen.frame(&mut view, Screen::wheel(direction));
    });
    let without = measure(30, FRAMES, || {
        direction = -direction;
        screen.empty_frame(Screen::wheel(direction));
    });
    let per_frame = with_view.allocs.saturating_sub(without.allocs) / FRAMES;
    let _ = writeln!(
        std::io::stderr(),
        "alloc_viewer: {} allocations per frame with the log screen, {} without (the test's own); \
         {per_frame} for the screen (budget {BUDGET}); {} rows held",
        with_view.allocs / FRAMES,
        without.allocs / FRAMES,
        view.held_rows()
    );
    if per_frame > BUDGET {
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
