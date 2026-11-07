//! The screens as layout snapshots (#71): the top bar, the welcome screen, a
//! spoofed name (#69) and the compact grid, each in light and dark at 1.0 and
//! 1.5 pixels per point.
//!
//! Text, not images; [`crate::paint_dump`] says why. To review a change run
//! `cargo insta review` (or set `INSTA_UPDATE=always`) and read the diff of
//! the `.snap` files like any other diff.

use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Rect, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use peekbuddy_core::query::Row;

use crate::filterspec::FilterSpec;
use crate::link::LinkState;
use crate::onboarding::Onboarding;
use crate::paint_dump::{Look, dump};
use crate::shell::Shell;
use crate::source::PageSource;
use crate::source::fake::FakeLog;
use crate::timeline::PAGE;
use crate::view::LogView;
use crate::watches::{Tab, tabs};

const SIZE: egui::Vec2 = vec2(1100.0, 520.0);

struct Screen {
    shell: Shell,
    tab: Tab,
    view: LogView,
}

fn screen(log: &Arc<FakeLog>, look: Look) -> Harness<'static, Screen> {
    let columns: Vec<String> = ["time", "kind", "path"]
        .iter()
        .map(ToString::to_string)
        .collect();
    let view = LogView::new(
        Arc::clone(log) as Arc<dyn PageSource>,
        Arc::new(|_| 0),
        Arc::new(|| {}),
        FilterSpec::default(),
        vec![String::from("C:\\w")],
        &columns,
    )
    .unwrap();
    let state = Screen {
        shell: Shell {
            link: LinkState::Connected {
                build: String::from("peekbuddy 0.0.0"),
            },
        },
        tab: Tab::Log,
        view,
    };
    let mut harness = Harness::builder()
        .with_size(SIZE)
        .with_pixels_per_point(look.ppp)
        .with_theme(look.theme)
        .build_ui_state(
            |ui, screen: &mut Screen| {
                screen.view.logic();
                let Screen { shell, tab, view } = screen;
                shell.show(ui, |ui| {
                    tabs(ui, tab, 1, 0);
                    view.show(ui);
                });
            },
            state,
        );
    let deadline = Instant::now() + Duration::from_secs(20);
    while harness.state().view.held_rows() < PAGE.min(log_len(log)) {
        harness.step();
        assert!(Instant::now() < deadline, "the first page never came");
        std::thread::sleep(Duration::from_millis(2));
    }
    harness
}

fn log_len(log: &Arc<FakeLog>) -> usize {
    let extent = log.extent().unwrap().unwrap();
    usize::try_from(extent.span()).unwrap_or(usize::MAX)
}

/// Lets the worker answer what is outstanding (the hidden count, the extent)
/// so that the status line is final.
fn settle<S>(harness: &mut Harness<'static, S>) {
    let end = Instant::now() + Duration::from_millis(400);
    while Instant::now() < end {
        harness.step();
        std::thread::sleep(Duration::from_millis(5));
    }
    harness.run_steps(3);
}

fn check(scene: &str, look: Look, text: &str) {
    insta::assert_snapshot!(format!("{scene}_{}", look.tag()), text);
}

fn spoofed(row: &mut Row) {
    row.path = "C:\\w\\invoice\u{202E}fdp.exe"
        .encode_utf16()
        .collect::<Vec<u16>>();
}

#[test]
fn the_top_bar_in_every_look() {
    for look in Look::ALL {
        let log = FakeLog::new(1_000);
        let mut harness = screen(&log, look);
        settle(&mut harness);
        let region = Rect::from_min_max(pos2(0.0, 0.0), pos2(SIZE.x, 160.0));
        check("top_bar", look, &dump(&harness, "top bar", look, region));
    }
}

#[test]
fn the_welcome_screen_in_every_look() {
    for look in Look::ALL {
        let mut harness = Harness::builder()
            .with_size(vec2(800.0, 900.0))
            .with_pixels_per_point(look.ppp)
            .with_theme(look.theme)
            .build_ui_state(
                |ui, screens: &mut Onboarding| {
                    screens.show(ui, &[]);
                },
                Onboarding::default(),
            );
        harness.run();
        let region = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 900.0));
        check(
            "welcome",
            look,
            &dump(&harness, "welcome screen", look, region),
        );
    }
}

#[test]
fn a_spoofed_name_is_marked_in_the_grid_and_in_the_details_in_every_look() {
    for look in Look::ALL {
        let log = FakeLog::new(20);
        log.shape_rows(spoofed);
        let mut harness = screen(&log, look);
        settle(&mut harness);
        harness.key_press(egui::Key::ArrowDown);
        settle(&mut harness);
        let region = Rect::from_min_size(pos2(0.0, 0.0), SIZE);
        let text = dump(&harness, "spoofed name", look, region);
        assert!(text.contains("u{202E}"), "{text}");
        check("spoofed_name", look, &text);
    }
}

#[test]
fn the_compact_grid_in_every_look() {
    for look in Look::ALL {
        let log = FakeLog::new(60);
        log.bursts(4);
        let mut harness = screen(&log, look);
        settle(&mut harness);
        harness.get_by_label("Compact").click();
        settle(&mut harness);
        let region = Rect::from_min_size(pos2(0.0, 0.0), SIZE);
        let text = dump(&harness, "compact grid", look, region);
        assert!(text.contains("folded into groups"), "{text}");
        check("compact_grid", look, &text);
    }
}
