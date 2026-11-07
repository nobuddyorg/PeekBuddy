//! Private bytes of the viewer with a log of `PEEKBUDDY_QUERY_ROWS` rows open
//! (#347, F8): the log screen reads a real store through the worker, a
//! scripted wheel sweeps the grid down and back up, and the private bytes of
//! this process are written as JSON to `PEEKBUDDY_PERF_OUT`.
//!
//! `cargo xtask perf --scenario viewer-memory` runs it twice, with a 1M-row
//! and a 10M-row store, and fails when the second needs more than 5 MB over
//! the first: what the viewer holds depends on the screen and the cache
//! limits, never on the length of the log. The UI pass only runs, as in
//! `alloc_viewer`; no GPU memory is part of the number.
//! `PEEKBUDDY_QUERY_DIR` keeps the store between runs.
//!
//! Does nothing without the two variables, and off Windows.

#[cfg(windows)]
#[path = "support/store.rs"]
mod fixture;

#[cfg(windows)]
mod run {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use peekbuddy_viewer::filterspec::FilterSpec;
    use peekbuddy_viewer::source::StoreSource;
    use peekbuddy_viewer::view::LogView;
    use peekbuddy_win::metrics;

    /// Frames of one sweep in one direction.
    const SWEEP: u64 = 600;
    /// Lines of wheel per frame: a page every frame or two, so the sweep
    /// pulls in thousands of pages.
    const LINES: f32 = 40.0;

    struct Screen {
        ctx: egui::Context,
        view: LogView,
        time: f64,
    }

    impl Screen {
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
            output.textures_delta.clear();
        }

        fn wheel(&mut self, lines: f32) {
            self.frame(vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                delta: egui::vec2(0.0, lines),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Wheel,
                modifiers: egui::Modifiers::NONE,
            }]);
            std::thread::sleep(Duration::from_millis(1));
        }

        /// Frames until the island holds the same number of rows for a while.
        fn settle(&mut self) {
            let deadline = Instant::now() + Duration::from_secs(30);
            let mut last = 0;
            let mut steady = 0;
            while steady < 50 && Instant::now() < deadline {
                self.frame(Vec::new());
                std::thread::sleep(Duration::from_millis(2));
                let held = self.view.held_rows();
                steady = if held == last { steady + 1 } else { 0 };
                last = held;
            }
        }
    }

    fn private_bytes() -> Option<u64> {
        metrics::memory(metrics::current_process())
            .ok()
            .map(|memory| memory.private_bytes)
    }

    pub fn main() -> Option<()> {
        let rows: u64 = std::env::var("PEEKBUDDY_QUERY_ROWS").ok()?.parse().ok()?;
        let out = std::env::var_os("PEEKBUDDY_PERF_OUT")?;
        let (dir, _guard) = super::fixture::store(rows)?;
        let view = LogView::new(
            Arc::new(StoreSource::new(&dir)),
            Arc::new(|_| 0),
            Arc::new(|| {}),
            FilterSpec::default(),
            vec![String::from(r"C:\Watched")],
            &["time", "kind", "path"].map(String::from),
        )
        .ok()?;
        let mut screen = Screen {
            ctx: egui::Context::default(),
            view,
            time: 0.0,
        };
        screen.frame(vec![egui::Event::PointerMoved(egui::pos2(500.0, 250.0))]);
        screen.settle();
        let before = private_bytes()?;
        // Down through the log and back, twice: the second pass shows
        // whether anything keeps growing.
        let mut peak = before;
        for _ in 0..2 {
            for _ in 0..SWEEP {
                screen.wheel(-LINES);
            }
            screen.settle();
            peak = peak.max(private_bytes()?);
            for _ in 0..SWEEP {
                screen.wheel(LINES);
            }
            screen.settle();
            peak = peak.max(private_bytes()?);
        }
        let after = private_bytes()?;
        let json = format!(
            "{{\"rows\":{rows},\"privateBytesBefore\":{before},\"privateBytesAfter\":{after},\
             \"privateBytesPeak\":{peak},\"heldRows\":{}}}",
            screen.view.held_rows()
        );
        std::fs::write(out, json).ok()
    }
}

#[cfg(windows)]
fn main() {
    if run::main().is_none() && std::env::var_os("PEEKBUDDY_PERF_OUT").is_some() {
        // Unset variables are a no-op for `cargo bench` of the package; a run
        // that was asked for and failed must not look green.
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn main() {}
