use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use peekbuddy_core::query::{CancelFlag, Filter, Page, PageRequest};

use super::{Col, LogView, Phase, ViewEvent, columns_from, group_digits};
use crate::filterspec::{FilterSpec, PatternKind};
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

fn app(source: Arc<dyn PageSource>, spec: FilterSpec, columns: &[&str]) -> Harness<'static, App> {
    let view = LogView::new(
        source,
        Arc::new(|_| 0),
        Arc::new(|| {}),
        spec,
        vec![String::from("C:\\w")],
        &names(columns),
    )
    .unwrap();
    Harness::builder()
        .with_size(egui::vec2(1100.0, 520.0))
        .build_ui_state(
            |ui, app: &mut App| {
                app.view.logic();
                for event in app.view.show(ui) {
                    app.event = Some(event);
                }
            },
            App { view, event: None },
        )
}

fn log_app(log: &Arc<FakeLog>) -> Harness<'static, App> {
    app(
        Arc::clone(log) as Arc<dyn PageSource>,
        FilterSpec::default(),
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

/// Right-clicks the row `row` until the item `item` of its menu is on
/// screen. A menu can close when rows arrive from the worker thread between
/// two frames, so one click is not always enough on a slow runner.
#[track_caller]
fn open_menu(harness: &mut Harness<'static, App>, row: &str, item: &str) {
    for _ in 0..10 {
        harness.get_by_label(row).click_secondary();
        harness.run_steps(4);
        if harness.query_by_label(item).is_some() {
            return;
        }
        harness.run_steps(2);
    }
    // Fails with the tree of the screen, which shows why there is no menu.
    harness.get_by_label(item);
}

/// Lets the worker thread answer everything that is outstanding (the hidden
/// count, the extent) before a test clicks: an answer that arrives between
/// the lookup of a row and the click can move the row under the pointer.
fn settle(harness: &mut Harness<'static, App>) {
    let end = Instant::now() + Duration::from_millis(400);
    while Instant::now() < end {
        harness.step();
        std::thread::sleep(Duration::from_millis(5));
    }
    harness.step();
}

#[test]
fn the_first_page_appears_and_only_it_is_held() {
    let log = FakeLog::new(10_000);
    let mut harness = log_app(&log);
    until(&mut harness, "the first page", |v| v.held_rows() == PAGE);
    harness.step();
    harness.get_by_label("f10000.txt");
    harness.get_by_label("Time");
    harness.get_by_label("about 10 000 events");
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
    settle(&mut harness);
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
    harness.get_by_label("about 10 000 000 events");
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
    let mut harness = app(Arc::new(Broken), FilterSpec::default(), &["path"]);
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
        FilterSpec::default(),
        &["kind", "path"],
    );
    until(&mut harness, "the first page", |v| v.held_rows() == PAGE);
    settle(&mut harness);
    harness.get_by_label("Columns").click();
    harness.run_steps(12);
    harness.get_by_label("Size").click();
    harness.run_steps(12);
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
        FilterSpec::default(),
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

#[test]
fn typing_quickly_asks_the_database_once() {
    let log = FakeLog::new(5_000);
    let mut harness = log_app(&log);
    until(&mut harness, "the first page", |v| v.held_rows() == PAGE);
    let before = log.extents();
    // Ten keystrokes in 200 ms.
    for key in 0..10 {
        let view = &mut harness.state_mut().view;
        view.spec.name.push('a');
        view.spec_changed();
        harness.step();
        std::thread::sleep(Duration::from_millis(20));
        let _ = key;
    }
    until(&mut harness, "the pause", |v| !v.is_applying());
    until(&mut harness, "the new view", |v| v.held_rows() == PAGE);
    assert_eq!(log.extents() - before, 1, "one query for ten keystrokes");
    assert_eq!(
        harness.state().view.filter.name.as_deref(),
        Some("aaaaaaaaaa")
    );
}

#[test]
fn an_invalid_regex_says_where_and_keeps_the_results() {
    let log = FakeLog::new(5_000);
    let mut harness = log_app(&log);
    until(&mut harness, "the first page", |v| v.held_rows() == PAGE);
    harness.state_mut().view.more = true;
    {
        let view = &mut harness.state_mut().view;
        view.spec.pattern = Some((PatternKind::Regex, String::from("(unclosed")));
        view.spec_changed();
    }
    until(&mut harness, "the error", |v| v.spec_error.is_some());
    harness.step();
    harness.get_by_label_contains("x rule 0");
    // The previous results are still there and the filter did not change.
    assert_eq!(harness.state().view.held_rows(), PAGE);
    assert!(harness.state().view.filter.pattern.is_none());
    harness.get_by_label("f5000.txt");
}

#[test]
fn the_hidden_counter_counts_and_the_button_shows_them() {
    let log = FakeLog::new(2_000);
    log.set_hidden(7);
    let mut harness = log_app(&log);
    until(&mut harness, "the count", |v| v.hidden == Some(7));
    harness.step();
    harness.get_by_label("7 hidden by the view.");
    harness.get_by_label("Show").click();
    until(&mut harness, "the pause", |v| !v.is_applying());
    assert_eq!(
        harness.state().view.filter.hide,
        peekbuddy_core::event::Flags::NONE
    );
    assert!(harness.state().view.spec.show_hidden);
}

#[test]
fn ctrl_f_finds_the_search_box_and_escape_clears_it() {
    let log = FakeLog::new(1_000);
    let mut harness = log_app(&log);
    until(&mut harness, "the first page", |v| v.held_rows() == PAGE);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::F);
    harness.run_steps(12);
    assert!(!harness.state().view.focus_search, "the request was served");
    harness.get_by_label("Search").type_text("rep");
    harness.run_steps(12);
    assert_eq!(harness.state().view.spec.name, "rep");
    harness.key_press(egui::Key::Escape);
    harness.run_steps(12);
    assert_eq!(harness.state().view.spec.name, "");
}

#[test]
fn a_view_without_matches_names_the_filter_that_narrows_it() {
    let log = FakeLog::new(0);
    let mut harness = app(
        Arc::clone(&log) as Arc<dyn PageSource>,
        FilterSpec::parse("name:zzz since:1h").unwrap(),
        &["path"],
    );
    until(&mut harness, "the empty view", LogView::first_page_ready);
    harness.step();
    harness.get_by_label_contains("No matching events");
    harness.get_by_label_contains("the name \"zzz\"");
}

#[test]
fn the_filter_text_edits_the_spec_and_a_typo_says_where() {
    let log = FakeLog::new(1_000);
    let mut harness = log_app(&log);
    until(&mut harness, "the first page", |v| v.held_rows() == PAGE);
    harness.state_mut().view.more = true;
    harness.step();
    {
        let view = &mut harness.state_mut().view;
        view.text = String::from("kind:deleted since:1h");
    }
    harness.get_by_label("Filter text").focus();
    harness.run_steps(4);
    harness.key_press(egui::Key::Enter);
    harness.run_steps(4);
    assert!(
        harness
            .state()
            .view
            .spec
            .kinds
            .contains(&crate::filterspec::Chip::Deleted)
    );
    assert_eq!(
        harness.state().view.spec.span,
        crate::filterspec::Span::LastHour
    );

    {
        let view = &mut harness.state_mut().view;
        view.text = String::from("kind:sparkly");
    }
    harness.get_by_label("Filter text").focus();
    harness.run_steps(4);
    harness.key_press(egui::Key::Enter);
    harness.run_steps(4);
    harness.get_by_label_contains("unknown kind `sparkly`");
}

#[test]
fn stopping_a_scan_forgets_the_requests_that_were_cancelled() {
    let log = FakeLog::new(1_000);
    let mut harness = log_app(&log);
    until(&mut harness, "the first page", |v| v.held_rows() == PAGE);
    let view = &mut harness.state_mut().view;
    // A request is out that will never be answered once it is cancelled.
    assert_eq!(view.timeline.plan(&(150..190)).len(), 1);
    assert!(view.timeline.busy());
    view.stop_scan();
    assert!(!view.timeline.busy());
    assert_eq!(view.timeline.plan(&(150..190)).len(), 1, "it asks again");
}

#[test]
fn the_details_pane_follows_the_selected_row_and_can_be_closed() {
    let log = FakeLog::new(1_000);
    let mut harness = log_app(&log);
    until(&mut harness, "the first page", |v| v.held_rows() == PAGE);
    settle(&mut harness);
    harness.get_by_label_contains("Select a row");
    harness.get_by_label("f998.txt").click();
    harness.run_steps(4);
    harness.get_by_label("Raw value");
    harness.get_by_label("0x00000003  FILE_ACTION_MODIFIED");
    harness.get_by_label("Time (FILETIME)");
    harness.get_by_label("Details").click();
    harness.run_steps(4);
    assert_eq!(
        harness.state().event,
        Some(ViewEvent::DetailsToggled(false))
    );
    assert!(harness.query_by_label("Raw value").is_none());
    assert!(!harness.state().view.details_open());
}

#[test]
fn the_context_menu_narrows_the_view_and_asks_for_explorer() {
    let log = FakeLog::new(1_000);
    let mut harness = log_app(&log);
    until(&mut harness, "the first page", |v| v.held_rows() == PAGE);
    settle(&mut harness);
    open_menu(&mut harness, "f998.txt", "Copy path");
    harness.get_by_label("Copy path");
    harness.get_by_label("Copy row");
    harness.get_by_label("Open containing folder");
    harness.get_by_label("Filter to this file").click();
    harness.run_steps(4);
    assert_eq!(harness.state().view.spec.name, "f998.txt");
    until(&mut harness, "the new view", |v| !v.is_applying());

    // Explorer is the app's job: the view only says what to show.
    let mut harness = log_app(&log);
    until(&mut harness, "the first page", |v| v.held_rows() == PAGE);
    harness.step();
    open_menu(&mut harness, "f998.txt", "Show in Explorer");
    harness.get_by_label("Show in Explorer").click();
    harness.run_steps(4);
    assert_eq!(
        harness.state().event,
        Some(ViewEvent::Open {
            path: String::from("C:\\w\\f998.txt"),
            select: true
        })
    );
}

#[test]
fn the_folder_filter_uses_the_folder_of_the_row() {
    let log = FakeLog::new(500);
    let mut harness = log_app(&log);
    until(&mut harness, "the first page", |v| v.held_rows() == PAGE);
    settle(&mut harness);
    open_menu(&mut harness, "f498.txt", "Filter to this folder");
    harness.get_by_label("Filter to this folder").click();
    harness.run_steps(4);
    assert_eq!(harness.state().view.spec.subtree.as_deref(), Some("C:\\w"));
}

#[test]
fn the_export_menu_starts_an_export_that_reports_and_can_be_dismissed() {
    let log = FakeLog::new(1_000);
    let mut harness = log_app(&log);
    until(&mut harness, "the first page", |v| v.held_rows() == PAGE);
    settle(&mut harness);
    // Without a data directory there is nothing to export to: no menu.
    assert!(harness.query_by_label("Export").is_none());

    let dir = std::env::temp_dir().join(format!("pb-view-export-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("segments")).unwrap();
    harness
        .state_mut()
        .view
        .set_export_dirs(dir.join("segments"), dir.join("exports"));
    harness.step();
    harness.get_by_label("Export").click();
    harness.run_steps(4);
    harness.get_by_label("CSV").click();
    until(&mut harness, "the export", |v| {
        v.export
            .as_ref()
            .is_some_and(crate::exporter::Export::finished)
    });
    harness.run_steps(3);
    harness.get_by_label_contains("Exported 0 rows to");
    harness.get_by_label("Show in Explorer").click();
    harness.run_steps(3);
    assert!(matches!(
        harness.state().event,
        Some(ViewEvent::Open { select: true, .. })
    ));
    harness.get_by_label("Dismiss").click();
    harness.run_steps(3);
    assert!(harness.state().view.export.is_none());
    assert!(dir.join("exports").is_dir());
}

#[test]
fn every_control_of_the_log_screen_has_a_name() {
    let log = FakeLog::new(1_000);
    let mut harness = log_app(&log);
    until(&mut harness, "the first page", |v| v.held_rows() == PAGE);
    harness.step();
    crate::a11y::assert_every_control_is_named(&harness);
}

/// A log where every four rows are changes to one file, folded in compact mode.
fn compact_app() -> Harness<'static, App> {
    let log = FakeLog::new(10_000);
    log.bursts(4);
    let mut harness = log_app(&log);
    until(&mut harness, "the first page", |v| v.held_rows() == PAGE);
    harness.get_by_label("Compact").click();
    settle(&mut harness);
    harness
}

#[test]
fn the_compact_toggle_folds_bursts_and_the_raw_view_shows_every_row() {
    let mut harness = compact_app();
    let view = &harness.state().view;
    assert!(view.compact);
    // 200 rows in bursts of four: nearly three of every four are folded.
    assert!(view.folds.folded() >= 140, "{}", view.folds.folded());
    assert!(view.status.text.contains("folded into groups"));
    // The scrollbar still counts raw rows, and says it is an estimate.
    assert_eq!(view.timeline.total(), 10_000);
    assert!(view.status.text.starts_with("about 10 000 events"));

    harness.get_by_label("Compact").click();
    settle(&mut harness);
    let view = &harness.state().view;
    assert!(!view.compact);
    assert_eq!(view.folds.folded(), 0);
    assert!(!view.status.text.contains("folded"));
    // The switch is reported, so the app can save it.
    assert_eq!(
        harness.state().event,
        Some(ViewEvent::CompactToggled(false))
    );
}

#[test]
fn restored_settings_apply_without_a_write_until_the_user_resizes() {
    let log = FakeLog::new(100);
    let mut harness = log_app(&log);
    {
        let view = &mut harness.state_mut().view;
        view.set_compact(true);
        view.set_column_widths(&std::collections::BTreeMap::from([(
            String::from("path"),
            333,
        )]));
    }
    until(&mut harness, "the first page", |v| v.held_rows() > 0);
    harness.run_steps(40);
    assert!(harness.state().view.compact());
    // Restoring and drawing report nothing to save.
    assert_eq!(harness.state().event, None);
}

#[test]
fn opening_a_group_shows_its_members_and_closing_folds_them_again() {
    let mut harness = compact_app();
    let folded = harness.state().view.folds.folded();
    let badges = harness.query_all_by_label("> 4").count();
    assert!(badges > 0);
    harness.query_all_by_label("> 4").next().unwrap().click();
    settle(&mut harness);
    let view = &harness.state().view;
    assert_eq!(view.folds.folded(), folded - 3);
    assert_eq!(view.expanded.len(), 1);

    harness.query_all_by_label("v 4").next().unwrap().click();
    settle(&mut harness);
    let view = &harness.state().view;
    assert_eq!(view.folds.folded(), folded);
    assert!(view.expanded.is_empty());
}

#[test]
fn the_arrow_keys_skip_the_rows_that_are_folded_away() {
    let mut harness = compact_app();
    // Row 0 is id 10000 alone; ids 9999..=9996 are a burst: the head is row 1
    // and rows 2 to 4 are folded, so the next row that is shown is row 5, then 9.
    for expected in [0, 1, 5, 9] {
        harness.key_press(egui::Key::ArrowDown);
        harness.step();
        assert_eq!(harness.state().view.selection.cursor(), Some(expected));
    }
    harness.key_press(egui::Key::ArrowUp);
    harness.step();
    harness.key_press(egui::Key::ArrowUp);
    harness.step();
    assert_eq!(harness.state().view.selection.cursor(), Some(1));
}

#[test]
fn the_from_and_to_fields_make_a_custom_range() {
    let log = FakeLog::new(5_000);
    let mut harness = log_app(&log);
    until(&mut harness, "the first page", |v| v.held_rows() == PAGE);
    harness.state_mut().view.more = true;
    harness.run_steps(10);
    harness.get_by_label("From").focus();
    harness.run_steps(10);
    harness.get_by_label("From").type_text("2026-10-26 14:00");
    harness.run_steps(10);
    harness.get_by_label("To").focus();
    harness.run_steps(10);
    harness.get_by_label("To").type_text("2026-10-26");
    harness.run_steps(10);
    // The offset of this harness is 0, so local time is UTC.
    let span = harness.state().view.spec.span;
    assert!(matches!(span, super::Span::Between { .. }), "{span:?}");
    let super::Span::Between { from, to } = span else {
        return;
    };
    let hour = 36_000_000_000;
    assert_eq!(to.unwrap() - from.unwrap(), 10 * hour - 1);
    assert!(harness.state().view.range_error.is_none());
    assert!(harness.state().view.spec.to_text().contains("from:"));

    // A field that is not a time says so and leaves the range alone.
    harness.get_by_label("To").focus();
    harness.run_steps(10);
    harness.get_by_label("To").type_text("x");
    harness.run_steps(10);
    assert!(harness.state().view.range_error.is_some());
    assert_eq!(harness.state().view.spec.span, span);

    // Emptying both fields goes back to any time.
    for label in ["From", "To"] {
        let view = &mut harness.state_mut().view;
        if label == "From" {
            view.range_from.clear();
        } else {
            view.range_to.clear();
        }
    }
    harness.get_by_label("From").focus();
    harness.run_steps(10);
    harness.get_by_label("From").type_text(" ");
    harness.run_steps(10);
    assert_eq!(harness.state().view.spec.span, super::Span::Any);
}
