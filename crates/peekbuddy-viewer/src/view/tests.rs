use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use peekbuddy_core::query::{CancelFlag, Filter, Page, PageRequest};

use super::{Col, LogView, Phase, ViewEvent, columns_from, group_digits};
use crate::source::PageSource;
use crate::source::fake::FakeLog;
use crate::timeline::{Extent, MAX_ROWS, PAGE};

struct App {
    view: LogView,
    event: Option<ViewEvent>,
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(ToString::to_string).collect()
}

fn app(source: Arc<dyn PageSource>, filter: Filter, columns: &[&str]) -> Harness<'static, App> {
    let view = LogView::new(
        source,
        Arc::new(|_| 0),
        Arc::new(|| {}),
        filter,
        &names(columns),
    )
    .unwrap();
    Harness::builder()
        .with_size(egui::vec2(1100.0, 520.0))
        .build_ui_state(
            |ui, app: &mut App| {
                app.view.logic();
                if let Some(event) = app.view.show(ui) {
                    app.event = Some(event);
                }
            },
            App { view, event: None },
        )
}

fn log_app(log: &Arc<FakeLog>) -> Harness<'static, App> {
    app(
        Arc::clone(log) as Arc<dyn PageSource>,
        Filter::default(),
        &["time", "kind", "path"],
    )
}

/// Steps frames until `done`, giving the worker thread time to answer.
#[track_caller]
fn until(harness: &mut Harness<'static, App>, what: &str, done: impl Fn(&LogView) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        harness.step();
        if done(&harness.state().view) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn the_first_page_appears_and_only_it_is_held() {
    let log = FakeLog::new(10_000);
    let mut harness = log_app(&log);
    until(&mut harness, "the first page", |v| v.held_rows() == PAGE);
    harness.step();
    harness.get_by_label("f10000.txt");
    harness.get_by_label("Time");
    harness.get_by_label("10 000 events");
    assert!(harness.state().view.first_page_ready());
    assert_eq!(harness.state().view.phase(), &Phase::Ready);
}

#[test]
fn an_empty_log_says_so_and_the_first_rows_arrive_through_the_tail() {
    let log = FakeLog::new(0);
    let mut harness = log_app(&log);
    until(&mut harness, "the empty state", LogView::first_page_ready);
    harness.step();
    harness.get_by_label_contains("No events recorded yet");
    log.grow(3);
    until(&mut harness, "the tail", |v| v.held_rows() == 3);
    harness.step();
    harness.get_by_label("f3.txt");
    assert!(
        harness
            .query_by_label_contains("No events recorded yet")
            .is_none()
    );
}

#[test]
fn new_rows_appear_at_the_top_while_following() {
    let log = FakeLog::new(1_000);
    let mut harness = log_app(&log);
    until(&mut harness, "the first page", |v| v.held_rows() == PAGE);
    log.grow(2);
    until(&mut harness, "the tail", |v| v.held_rows() == PAGE + 2);
    harness.step();
    harness.get_by_label("f1002.txt");
    assert!(harness.state().view.follow.is_following());
    assert!(harness.query_by_label_contains("new event").is_none());
}

#[test]
fn scrolled_away_the_grid_counts_and_the_pill_goes_back() {
    let log = FakeLog::new(5_000);
    let mut harness = log_app(&log);
    until(&mut harness, "the first page", |v| v.held_rows() == PAGE);
    // The user scrolls far away.
    harness.state_mut().view.scroll_to = Some((2_500, egui::Align::TOP));
    until(&mut harness, "the pause", |v| !v.follow.is_following());
    until(&mut harness, "the island at the target", |v| {
        v.timeline.row_at(2_500).is_some()
    });
    let id_at = |harness: &Harness<'static, App>| {
        harness
            .state()
            .view
            .timeline
            .row_at(2_500)
            .map(|row| row.id)
    };
    let before = id_at(&harness);
    assert_eq!(before, Some(5_000 - 2_500));
    log.grow(5);
    until(&mut harness, "five counted rows", |v| {
        v.follow.new_rows() == 5
    });
    harness.step();
    harness.get_by_label("5 new events");
    // The island may extend at its edges, but the rows stay where they are.
    assert_eq!(id_at(&harness), before, "the rows did not move");

    harness.get_by_label("5 new events").click();
    until(&mut harness, "following again", |v| {
        v.follow.is_following() && v.timeline.row_at(0).is_some_and(|r| r.id == 5_005)
    });
    assert!(harness.query_by_label_contains("new event").is_none());
}

#[test]
fn the_keyboard_moves_the_selection_and_the_view_follows_it() {
    let log = FakeLog::new(100_000);
    let mut harness = log_app(&log);
    until(&mut harness, "the first page", |v| v.held_rows() == PAGE);
    harness.key_press(egui::Key::ArrowDown);
    harness.step();
    assert_eq!(harness.state().view.selection.cursor(), Some(0));
    harness.key_press(egui::Key::ArrowDown);
    harness.step();
    assert_eq!(harness.state().view.selection.cursor(), Some(1));
    harness.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::ArrowDown);
    harness.step();
    assert_eq!(harness.state().view.selection.range(), Some(1..=2));

    harness.key_press(egui::Key::End);
    harness.step();
    assert_eq!(harness.state().view.selection.cursor(), Some(99_999));
    until(&mut harness, "the oldest rows", |v| {
        v.timeline.row_at(99_999).is_some()
    });
    assert!(!harness.state().view.follow.is_following());

    harness.key_press(egui::Key::Home);
    harness.step();
    assert_eq!(harness.state().view.selection.cursor(), Some(0));
    until(&mut harness, "the newest rows again", |v| {
        v.timeline.row_at(0).is_some_and(|r| r.id == 100_000)
    });
}

#[test]
fn a_click_selects_the_row() {
    let log = FakeLog::new(1_000);
    let mut harness = log_app(&log);
    until(&mut harness, "the first page", |v| v.held_rows() == PAGE);
    harness.step();
    harness.get_by_label("f998.txt").click();
    harness.step();
    assert_eq!(harness.state().view.selection.range(), Some(2..=2));
    assert_eq!(
        harness
            .state()
            .view
            .selected_rows()
            .map(|r| r.id)
            .collect::<Vec<_>>(),
        vec![998]
    );
    harness.step();
    harness.get_by_label_contains("1 selected");
}

#[test]
fn memory_stays_bounded_across_a_log_of_ten_million_rows() {
    let log = FakeLog::new(10_000_000);
    let mut harness = log_app(&log);
    until(&mut harness, "the first page", |v| v.held_rows() == PAGE);
    harness.get_by_label("10 000 000 events");
    for target in [5_000_000_u64, 9_999_999, 123_456, 0] {
        harness.state_mut().view.scroll_to = Some((target, egui::Align::TOP));
        until(&mut harness, "the island at the target", |v| {
            v.timeline.row_at(target).is_some()
        });
        let view = &harness.state().view;
        assert!(view.held_rows() <= MAX_ROWS);
        // Exact ids: row `i` of an unfiltered log is `newest - i`.
        assert_eq!(
            view.timeline.row_at(target).map(|r| r.id),
            Some(10_000_000 - i64::try_from(target).unwrap())
        );
    }
}

struct Broken;

impl PageSource for Broken {
    fn page(&self, _: &PageRequest, _: &CancelFlag) -> Result<Page, String> {
        Err(String::from("disk on fire"))
    }

    fn extent(&self) -> Result<Option<Extent>, String> {
        Err(String::from("catalog.db is damaged"))
    }

    fn hidden(&self, _: &Filter, _: &CancelFlag) -> Result<Option<u64>, String> {
        Err(String::from("no"))
    }
}

#[test]
fn a_log_that_cannot_be_read_says_why() {
    let mut harness = app(Arc::new(Broken), Filter::default(), &["path"]);
    until(&mut harness, "the failure", |v| {
        matches!(v.phase(), Phase::Failed(_))
    });
    harness.step();
    harness.get_by_label_contains("The log could not be read: catalog.db is damaged");
    assert!(
        harness.state().view.first_page_ready(),
        "the viewer still opens"
    );
}

#[test]
fn the_column_chooser_reports_the_new_list_and_keeps_one_column() {
    let log = FakeLog::new(1_000);
    let mut harness = app(
        Arc::clone(&log) as Arc<dyn PageSource>,
        Filter::default(),
        &["kind", "path"],
    );
    until(&mut harness, "the first page", |v| v.held_rows() == PAGE);
    harness.get_by_label("Columns").click();
    harness.run();
    harness.get_by_label("Size").click();
    harness.run();
    assert_eq!(
        harness.state().event,
        Some(ViewEvent::ColumnsChanged(names(&["kind", "path", "size"])))
    );
    assert_eq!(
        harness.state().view.columns(),
        &[Col::Kind, Col::Path, Col::Size]
    );
    until(&mut harness, "the size cells", |_| true);
    harness.get_by_label("Size");
    assert!(
        harness.get_all_by_label("100 B").count() > 3,
        "every visible row has a size"
    );
}

#[test]
fn the_default_view_filter_does_not_break_the_grid() {
    // The default view hides cloud churn: the id span is only an estimate.
    let log = FakeLog::new(2_000);
    log.set_hidden(50);
    let mut harness = app(
        Arc::clone(&log) as Arc<dyn PageSource>,
        Filter::default_view(),
        &["path"],
    );
    until(&mut harness, "the first page", |v| v.held_rows() == PAGE);
    until(&mut harness, "the hidden count", |v| {
        v.timeline.total() == 1_950
    });
    harness.step();
    harness.get_by_label("about 1 950 events");
}

#[test]
fn columns_come_from_the_settings_names_and_never_end_up_empty() {
    assert_eq!(
        columns_from(&names(&["path", "time", "path", "process", "volume"])),
        vec![Col::Path, Col::Time]
    );
    assert_eq!(
        columns_from(&names(&["process"])),
        vec![Col::Time, Col::Kind, Col::Path]
    );
    for col in Col::ALL {
        assert_eq!(Col::from_name(col.name()), Some(col));
        assert!(!col.title().is_empty() && !col.tip().is_empty());
    }
}

#[test]
fn numbers_are_grouped_by_thousands() {
    assert_eq!(group_digits(0), "0");
    assert_eq!(group_digits(999), "999");
    assert_eq!(group_digits(1_000), "1 000");
    assert_eq!(group_digits(1_234_567), "1 234 567");
    assert_eq!(group_digits(10_000_000), "10 000 000");
}
