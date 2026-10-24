//! Tests of the derived views: patterns, fixtures, the model and the bounds.

use proptest::prelude::*;
use proptest_state_machine::{ReferenceStateMachine, StateMachineTest, prop_state_machine};

use super::*;
use crate::event::tests::sample;
use crate::event::{Event, Flags, GapReason, WatchState};

const MS: u64 = TICKS_PER_MS;

fn w(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

/// One input row: id, time in ms, kind, root, file id, name.
type Row = (u64, u64, Kind, u16, u128, &'static str);

fn stored(&(id, ms, kind, root, file_id, _): &Row) -> StoredEvent {
    StoredEvent {
        id,
        ev: Event {
            ts: 1_000_000 + ms * MS,
            kind,
            root: RootIdx(root),
            file_id,
            flags: Flags::NONE,
            ..sample()
        },
    }
}

fn run(config: Config, rows: &[Row]) -> (Vec<DisplayItem>, Stats) {
    let names: Vec<Vec<u16>> = rows.iter().map(|r| w(r.5)).collect();
    derive_all(
        config,
        rows.iter()
            .zip(&names)
            .map(|(r, n)| (stored(r), WideStr::new(n))),
    )
}

fn compact() -> Config {
    Config {
        mode: ViewMode::Compact,
        ..Config::default()
    }
}

/// One line per item, for readable assertions and snapshots.
fn render(items: &[DisplayItem]) -> String {
    let text = |units: &[u16]| String::from_utf16_lossy(units);
    items
        .iter()
        .map(|item| match item {
            DisplayItem::Raw(id) => format!("raw {id}"),
            DisplayItem::Unpaired(id) => format!("unpaired {id}"),
            DisplayItem::Group(g) => {
                let from = g
                    .from_path
                    .as_deref()
                    .map_or(String::new(), |f| format!("{} -> ", text(f)));
                format!(
                    "{:?} {:?} {from}{} (raw events: {})",
                    g.kind,
                    g.member_ids,
                    text(&g.path),
                    g.raw_count
                )
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

use Kind::{AttributesChanged, Created, Deleted, Modified, RenamedFrom, RenamedTo};

#[test]
fn an_adjacent_rename_is_stitched() {
    let rows = [
        (1, 0, RenamedFrom, 0, 0, "a.txt"),
        (2, 1, RenamedTo, 0, 0, "b.txt"),
        (3, 2, Modified, 0, 0, "other"),
    ];
    let (items, stats) = run(Config::default(), &rows);
    assert_eq!(
        render(&items),
        "Renamed [1, 2] a.txt -> b.txt (raw events: 2)\nraw 3"
    );
    assert_eq!(stats.unpaired, 0);
}

#[test]
fn a_rename_is_stitched_by_file_id_across_other_rows() {
    let rows = [
        (1, 0, RenamedFrom, 0, 77, "a.txt"),
        (2, 1, Modified, 0, 5, "x"),
        (3, 2, RenamedTo, 0, 77, "b.txt"),
    ];
    let (items, _) = run(Config::default(), &rows);
    assert_eq!(
        render(&items),
        "Renamed [1, 3] a.txt -> b.txt (raw events: 2)\nraw 2"
    );
    // A different file id does not pair across a gap.
    let rows = [
        (1, 0, RenamedFrom, 0, 77, "a.txt"),
        (2, 1, Modified, 0, 5, "x"),
        (3, 2, RenamedTo, 0, 78, "b.txt"),
    ];
    let (items, stats) = run(Config::default(), &rows);
    assert_eq!(render(&items), "unpaired 1\nraw 2\nunpaired 3");
    assert_eq!(stats.unpaired, 2);
}

#[test]
fn adjacency_needs_matching_or_unknown_file_ids() {
    let pair = |from, to| {
        let rows = [
            (1, 0, RenamedFrom, 0, from, "a"),
            (2, 1, RenamedTo, 0, to, "b"),
        ];
        run(Config::default(), &rows).0.len()
    };
    assert_eq!(pair(0, 0), 1);
    assert_eq!(pair(9, 9), 1);
    assert_eq!(pair(0, 9), 1);
    assert_eq!(pair(9, 0), 1);
    assert_eq!(pair(9, 8), 2);
}

#[test]
fn halves_across_roots_do_not_pair() {
    let rows = [(1, 0, RenamedFrom, 0, 0, "a"), (2, 1, RenamedTo, 1, 0, "b")];
    let (items, _) = run(Config::default(), &rows);
    assert_eq!(render(&items), "unpaired 1\nunpaired 2");
}

#[test]
fn a_rename_half_after_the_window_is_unpaired() {
    let rows = [
        (1, 0, RenamedFrom, 0, 5, "a"),
        (2, 1_000, Modified, 0, 0, "far"),
        (3, 1_001, RenamedTo, 0, 5, "b"),
    ];
    let (items, _) = run(Config::default(), &rows);
    assert_eq!(render(&items), "unpaired 1\nraw 2\nunpaired 3");
}

#[test]
fn the_window_edge_is_inclusive() {
    let cfg = Config {
        save_window_ms: 100,
        ..Config::default()
    };
    let at_edge = [(1, 0, Modified, 0, 0, "a"), (2, 100, Modified, 0, 0, "a")];
    let past = [(1, 0, Modified, 0, 0, "a"), (2, 101, Modified, 0, 0, "a")];
    let cfg = Config {
        mode: ViewMode::Compact,
        coalesce_window_ms: 50,
        ..cfg
    };
    assert_eq!(run(cfg.clone(), &at_edge).0.len(), 1);
    assert_eq!(run(cfg, &past).0.len(), 2);
}

#[test]
fn word_style_save_is_one_group() {
    // Created ~WRL, written, doc -> ~WRD, ~WRL -> doc, ~WRD deleted.
    let rows = [
        (1, 0, Created, 0, 0, "~WRL0001.tmp"),
        (2, 5, Modified, 0, 0, "~WRL0001.tmp"),
        (3, 10, RenamedFrom, 0, 0, "doc.docx"),
        (4, 10, RenamedTo, 0, 0, "~WRD0000.tmp"),
        (5, 11, RenamedFrom, 0, 0, "~WRL0001.tmp"),
        (6, 11, RenamedTo, 0, 0, "doc.docx"),
        (7, 12, Deleted, 0, 0, "~WRD0000.tmp"),
        (8, 900, Modified, 0, 0, "later"),
    ];
    let (items, _) = run(Config::default(), &rows);
    assert_eq!(
        render(&items),
        "Saved [1, 2, 3, 4, 5, 6, 7] doc.docx (raw events: 7)\nraw 8"
    );
}

#[test]
fn temp_write_and_rename_over_the_target() {
    let rows = [
        (1, 0, Created, 0, 0, "x.tmp"),
        (2, 1, Modified, 0, 0, "x.tmp"),
        (3, 2, RenamedFrom, 0, 0, "x.tmp"),
        (4, 2, RenamedTo, 0, 0, "doc.txt"),
    ];
    assert_eq!(
        render(&run(Config::default(), &rows).0),
        "Saved [1, 2, 3, 4] doc.txt (raw events: 4)"
    );
}

#[test]
fn delete_and_create_is_a_save() {
    let rows = [
        (1, 0, Deleted, 0, 0, "doc.txt"),
        (2, 3, Created, 0, 0, "doc.txt"),
        (3, 4, Modified, 0, 0, "doc.txt"),
    ];
    assert_eq!(
        render(&run(Config::default(), &rows).0),
        "Saved [1, 2, 3] doc.txt (raw events: 3)"
    );
}

#[test]
fn a_plain_rename_stays_a_rename_group_in_both_modes() {
    let rows = [(1, 0, RenamedFrom, 0, 0, "a"), (2, 1, RenamedTo, 0, 0, "b")];
    let want = "Renamed [1, 2] a -> b (raw events: 2)";
    assert_eq!(render(&run(Config::default(), &rows).0), want);
    assert_eq!(render(&run(compact(), &rows).0), want);
}

#[test]
fn coalescing_is_compact_mode_only() {
    let rows = [
        (1, 0, Modified, 0, 0, "log.txt"),
        (2, 20, Modified, 0, 0, "log.txt"),
        (3, 40, AttributesChanged, 0, 0, "log.txt"),
    ];
    assert_eq!(
        render(&run(Config::default(), &rows).0),
        "raw 1\nraw 2\nraw 3"
    );
    assert_eq!(
        render(&run(compact(), &rows).0),
        "Coalesced [1, 2, 3] log.txt (raw events: 3)"
    );
}

#[test]
fn paths_are_compared_with_the_case_rules() {
    let rows = [
        (1, 0, Modified, 0, 0, "Log.TXT"),
        (2, 5, Modified, 0, 0, "log.txt"),
    ];
    assert_eq!(run(compact(), &rows).0.len(), 1);
    let sensitive = Config {
        default_case: CaseMode::Sensitive,
        ..compact()
    };
    assert_eq!(run(sensitive, &rows).0.len(), 2);
    let overridden = Config {
        case_overrides: vec![(RootIdx(0), CaseMode::Sensitive)],
        ..compact()
    };
    assert_eq!(run(overridden, &rows).0.len(), 2);
    // Both separators name the same path.
    let rows = [
        (1, 0, Modified, 0, 0, "d\\f"),
        (2, 5, Modified, 0, 0, "d/f"),
    ];
    assert_eq!(run(compact(), &rows).0.len(), 1);
}

#[test]
fn roots_do_not_mix() {
    let rows = [(1, 0, Modified, 0, 0, "f"), (2, 5, Modified, 1, 0, "f")];
    assert_eq!(run(compact(), &rows).0.len(), 2);
}

#[test]
fn barriers_are_never_grouped_and_close_their_root() {
    let rows = [
        (1, 0, Modified, 0, 0, "f"),
        (2, 1, Modified, 1, 0, "g"),
        (3, 2, Kind::Overflow(GapReason::BufferOverflow), 0, 0, ""),
        (4, 3, Modified, 0, 0, "f"),
        (5, 4, Modified, 1, 0, "g"),
        (6, 5, Kind::ReconciledCreated, 1, 0, "f"),
        (7, 6, Kind::WatchStateChanged(WatchState::Live), 0, 0, ""),
    ];
    let (items, _) = run(compact(), &rows);
    // Root 0 is cut by the overflow, root 1 only by its own reconcile row.
    assert_eq!(
        render(&items),
        "raw 1\nCoalesced [2, 5] g (raw events: 2)\nraw 3\nraw 4\nraw 6\nraw 7"
    );
}

#[test]
fn a_waiting_rename_half_is_unpaired_when_a_barrier_comes() {
    let rows = [
        (1, 0, RenamedFrom, 0, 5, "a"),
        (2, 1, Kind::Overflow(GapReason::ParserError), 0, 0, ""),
        (3, 2, RenamedTo, 0, 5, "b"),
    ];
    assert_eq!(
        render(&run(Config::default(), &rows).0),
        "unpaired 1\nraw 2\nunpaired 3"
    );
}

#[test]
fn nameless_and_system_rows_pass_through() {
    let rows = [
        (1, 0, Modified, 0, 0, ""),
        (2, 1, Kind::EngineStarted, 0, 0, "x"),
        (3, 2, Modified, 0, 0, ""),
    ];
    assert_eq!(render(&run(compact(), &rows).0), "raw 1\nraw 2\nraw 3");
}

#[test]
fn an_open_burst_holds_back_the_rows_behind_it() {
    let names = [w("a"), w("b"), w("c")];
    let rows = [
        (1, 0, Modified, 0, 0, "a"),
        (2, 1, Kind::EngineStarted, 0, 0, "x"),
        (3, 2, Modified, 0, 0, "c"),
    ];
    let mut deriver = Deriver::new(Config::default());
    let mut out = Vec::new();
    for (row, name) in rows.iter().zip(&names) {
        deriver.push(stored(row), WideStr::new(name));
    }
    deriver.take(&mut out);
    assert!(out.is_empty(), "rows 1..3 wait for burst 1 and 3");
    assert_eq!(deriver.stats().open_groups, 2);
    assert_eq!(deriver.stats().pending_items, 3);
    // Time passes without events (a viewer frame): the windows end.
    deriver.advance(1_000_000 + 10_000 * MS);
    deriver.take(&mut out);
    assert_eq!(render(&out), "raw 1\nraw 2\nraw 3");
    assert_eq!(deriver.stats().open_groups, 0);
    assert_eq!(deriver.stats().pending_items, 0);
}

#[test]
fn too_many_open_bursts_flush_the_oldest() {
    let cfg = Config {
        max_open_groups: 2,
        ..compact()
    };
    let names: Vec<Vec<u16>> = (0..5).map(|i| w(&format!("f{i}"))).collect();
    let mut deriver = Deriver::new(cfg);
    for (i, name) in names.iter().enumerate() {
        let id = u64::try_from(i).unwrap() + 1;
        let row: Row = (id, 0, Modified, 0, 0, "x");
        deriver.push(stored(&row), WideStr::new(name));
        assert!(deriver.stats().open_groups <= 2);
    }
    assert_eq!(deriver.stats().forced_flushes, 3);
    deriver.finish();
    let mut out = Vec::new();
    deriver.take(&mut out);
    assert_eq!(render(&out), "raw 1\nraw 2\nraw 3\nraw 4\nraw 5");
}

#[test]
fn pending_items_are_bounded() {
    // One burst that never closes (an event every 40 ms on one path) holds
    // back rows of a barrier-free pass-through kind.
    let cfg = Config {
        max_open_groups: 2,
        ..compact()
    };
    let mut deriver = Deriver::new(cfg);
    let name = w("busy");
    let mut ids = 0_u64;
    let mut peak = 0;
    for _ in 0..100 {
        ids += 1;
        deriver.push(
            stored(&(ids, ids * 40, Modified, 0, 0, "busy")),
            WideStr::new(&name),
        );
        ids += 1;
        deriver.push(
            stored(&(ids, ids * 40, Kind::EngineStarted, 0, 0, "x")),
            WideStr::new(&name),
        );
        let pending = deriver.stats().pending_items;
        assert!(pending <= 2 * 4, "{:?}", deriver.stats());
        peak = peak.max(pending);
    }
    // The bound is used in full, not one short of it.
    assert_eq!(peak, 2 * 4);
    assert!(deriver.stats().forced_flushes > 0);
}

#[test]
fn one_push_can_need_several_forced_flushes() {
    // A burst with many steps closes behind two open ones and spreads into
    // many pending items at once: the bound must flush until it holds.
    let cfg = Config {
        save_window_ms: 100,
        max_open_groups: 3,
        ..Config::default()
    };
    let max_pending = 3 * 4;
    let mut deriver = Deriver::new(cfg);
    let push = |deriver: &mut Deriver, id: u64, ms: u64, path: &str| {
        deriver.push(
            stored(&(id, ms, Modified, 0, 0, "")),
            WideStr::new(&w(path)),
        );
        assert!(
            deriver.stats().pending_items <= max_pending,
            "{:?}",
            deriver.stats()
        );
    };
    push(&mut deriver, 1, 0, "a");
    for id in 2..=4 {
        push(&mut deriver, id, 0, "b");
    }
    push(&mut deriver, 5, 0, "c");
    for id in 6..=17 {
        push(&mut deriver, id, 0, "b");
    }
    // Keep the outer bursts alive, then let burst b run out.
    push(&mut deriver, 18, 60, "a");
    push(&mut deriver, 19, 60, "c");
    assert_eq!(deriver.stats().forced_flushes, 0);
    push(&mut deriver, 20, 120, "d");
    assert_eq!(deriver.stats().forced_flushes, 2);
    deriver.finish();
    assert!(deriver.is_idle());
    let mut out = Vec::new();
    deriver.take(&mut out);
    let ids: Vec<u64> = out.iter().flat_map(DisplayItem::ids).collect();
    assert_eq!(ids, (1..=20).collect::<Vec<u64>>());
}

#[test]
fn a_rename_onto_a_busy_path_merges_the_bursts() {
    // The earlier burst (id 1) joins the later one (id 2).
    let names = [w("x"), w("a"), w("x")];
    let rows = [
        (1, 0, Deleted, 0, 0, "x"),
        (2, 1, RenamedFrom, 0, 7, "a"),
        (3, 2, RenamedTo, 0, 7, "x"),
    ];
    let mut deriver = Deriver::new(Config::default());
    deriver.push(stored(&rows[0]), WideStr::new(&names[0]));
    deriver.push(stored(&rows[1]), WideStr::new(&names[1]));
    assert_eq!(deriver.stats().open_groups, 2);
    deriver.push(stored(&rows[2]), WideStr::new(&names[2]));
    assert_eq!(deriver.stats().open_groups, 1);
    assert_eq!(deriver.stats().pending_items, 1);
    deriver.finish();
    assert!(deriver.is_idle());
    let mut out = Vec::new();
    deriver.take(&mut out);
    assert_eq!(render(&out), "raw 1\nRenamed [2, 3] a -> x (raw events: 2)");
}

#[test]
fn merged_bursts_keep_their_paths_apart() {
    // Deleted x and created x around a rename onto x: a save, whichever
    // burst came first.
    let later_into_earlier = [
        (1, 0, RenamedFrom, 0, 7, "a"),
        (2, 1, Deleted, 0, 0, "x"),
        (3, 2, RenamedTo, 0, 7, "x"),
        (4, 3, Created, 0, 0, "x"),
    ];
    let (items, _) = run(Config::default(), &later_into_earlier);
    assert_eq!(render(&items), "Saved [1, 2, 3, 4] x (raw events: 4)");
    let earlier_into_later = [
        (1, 0, Deleted, 0, 0, "x"),
        (2, 1, RenamedFrom, 0, 7, "a"),
        (3, 2, RenamedTo, 0, 7, "x"),
        (4, 3, Created, 0, 0, "x"),
    ];
    let (items, _) = run(Config::default(), &earlier_into_later);
    assert_eq!(render(&items), "Saved [1, 2, 3, 4] x (raw events: 4)");
}

#[test]
fn pairing_by_file_id_leaves_no_state_behind() {
    let names = [w("a"), w("c"), w("b"), w("d")];
    let rows = [
        (1, 0, RenamedFrom, 0, 7, "a"),
        (2, 1, Modified, 0, 0, "c"),
        (3, 2, RenamedTo, 0, 7, "b"),
        (4, 3, RenamedFrom, 0, 8, "d"),
    ];
    let mut deriver = Deriver::new(Config::default());
    for (row, name) in rows.iter().zip(&names) {
        deriver.push(stored(row), WideStr::new(name));
    }
    // The pair, the lone edit and the last half each have a burst.
    assert_eq!(deriver.stats().open_groups, 3);
    deriver.finish();
    assert!(deriver.is_idle());
    let mut out = Vec::new();
    deriver.take(&mut out);
    assert_eq!(
        render(&out),
        "Renamed [1, 3] a -> b (raw events: 2)\nraw 2\nunpaired 4"
    );
}

#[test]
fn windows_are_clamped() {
    let cfg = Config {
        save_window_ms: 1,
        coalesce_window_ms: 1,
        ..Config::default()
    };
    assert_eq!(cfg.window_ticks(), u64::from(MIN_WINDOW_MS) * MS);
    let cfg = Config {
        save_window_ms: 99_999,
        ..Config::default()
    };
    assert_eq!(cfg.window_ticks(), u64::from(MAX_WINDOW_MS) * MS);
    let cfg = Config {
        mode: ViewMode::Compact,
        save_window_ms: 100,
        coalesce_window_ms: 300,
        ..Config::default()
    };
    assert_eq!(cfg.window_ticks(), 300 * MS);
    let cfg = Config {
        mode: ViewMode::Detail,
        save_window_ms: 100,
        coalesce_window_ms: 300,
        ..Config::default()
    };
    assert_eq!(cfg.window_ticks(), 100 * MS);
}

#[test]
fn display_items_report_their_ids() {
    let rows = [
        (1, 0, RenamedFrom, 0, 0, "a"),
        (2, 1, RenamedTo, 0, 0, "b"),
        (3, 2, Modified, 0, 0, "c"),
    ];
    let (items, _) = run(Config::default(), &rows);
    assert_eq!(
        items.iter().map(DisplayItem::first_id).collect::<Vec<_>>(),
        vec![1, 3]
    );
    assert_eq!(
        items.iter().flat_map(DisplayItem::ids).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
}

// ---- fixtures -------------------------------------------------------------

#[test]
fn fixture_notepad_save() {
    // Notepad rewrites the file in place: truncate and write.
    let rows = [
        (1, 0, Modified, 0, 0, "notes.txt"),
        (2, 2, Modified, 0, 0, "notes.txt"),
        (3, 4, AttributesChanged, 0, 0, "notes.txt"),
    ];
    insta::assert_snapshot!("notepad_detail", render(&run(Config::default(), &rows).0));
    insta::assert_snapshot!("notepad_compact", render(&run(compact(), &rows).0));
}

#[test]
fn fixture_vscode_save() {
    // VS Code writes in place, with a metadata update after.
    let rows = [
        (1, 0, Modified, 0, 0, "src\\main.rs"),
        (2, 1, Modified, 0, 0, "src\\main.rs"),
        (3, 30, AttributesChanged, 0, 0, "src\\main.rs"),
        (4, 400, Modified, 0, 0, "src\\lib.rs"),
    ];
    insta::assert_snapshot!("vscode_compact", render(&run(compact(), &rows).0));
}

#[test]
fn fixture_word_save() {
    let rows = [
        (1, 0, Created, 0, 0, "~WRL0001.tmp"),
        (2, 3, Modified, 0, 0, "~WRL0001.tmp"),
        (3, 4, Modified, 0, 0, "~WRL0001.tmp"),
        (4, 20, RenamedFrom, 0, 0, "report.docx"),
        (5, 20, RenamedTo, 0, 0, "~WRD0000.tmp"),
        (6, 21, RenamedFrom, 0, 0, "~WRL0001.tmp"),
        (7, 21, RenamedTo, 0, 0, "report.docx"),
        (8, 22, Deleted, 0, 0, "~WRD0000.tmp"),
        (9, 23, AttributesChanged, 0, 0, "report.docx"),
    ];
    insta::assert_snapshot!("word_detail", render(&run(Config::default(), &rows).0));
}

#[test]
fn fixture_git_checkout() {
    // Many distinct files: nothing to group, everything stays visible.
    let rows = [
        (1, 0, Modified, 0, 0, "a.rs"),
        (2, 1, Modified, 0, 0, "b.rs"),
        (3, 1, Deleted, 0, 0, "old.rs"),
        (4, 2, Created, 0, 0, "new.rs"),
        (5, 2, Modified, 0, 0, "new.rs"),
        (6, 3, Modified, 0, 0, "c.rs"),
    ];
    insta::assert_snapshot!(
        "git_checkout_detail",
        render(&run(Config::default(), &rows).0)
    );
    insta::assert_snapshot!("git_checkout_compact", render(&run(compact(), &rows).0));
}

#[test]
fn fixture_rename_within_root() {
    let rows = [
        (1, 0, RenamedFrom, 0, 0, "docs\\a.md"),
        (2, 0, RenamedTo, 0, 0, "archive\\a.md"),
        (3, 5, Modified, 0, 0, "archive\\a.md"),
    ];
    insta::assert_snapshot!(
        "rename_within_root_compact",
        render(&run(compact(), &rows).0)
    );
}

#[test]
fn fixture_rename_out_of_root() {
    // A move out of the watched tree is a lone removal or a lone "old name".
    let rows = [
        (1, 0, RenamedFrom, 0, 0, "docs\\a.md"),
        (2, 900, Modified, 0, 0, "docs\\b.md"),
        (3, 910, Deleted, 0, 0, "docs\\c.md"),
    ];
    insta::assert_snapshot!(
        "rename_out_of_root",
        render(&run(Config::default(), &rows).0)
    );
}

// ---- determinism ----------------------------------------------------------

/// A small alphabet, so that paths collide often.
fn arb_rows() -> impl Strategy<Value = Vec<Row>> {
    let kind = prop_oneof![
        4 => Just(Modified),
        2 => Just(Created),
        2 => Just(Deleted),
        2 => Just(RenamedFrom),
        2 => Just(RenamedTo),
        1 => Just(AttributesChanged),
        1 => Just(Kind::Overflow(GapReason::BufferOverflow)),
        1 => Just(Kind::ReconciledModified),
        1 => Just(Kind::EngineStarted),
    ];
    let path = prop_oneof![
        Just("a"),
        Just("b"),
        Just("c"),
        Just("A"),
        Just("d\\a"),
        Just("")
    ];
    proptest::collection::vec((kind, path, 0_u16..2, 0_u128..3, 0_u64..200), 0..60).prop_map(
        |raw| {
            let mut ms = 0;
            raw.into_iter()
                .enumerate()
                .map(|(i, (kind, path, root, file_id, dt))| {
                    ms += dt;
                    (u64::try_from(i).unwrap() + 1, ms, kind, root, file_id, path)
                })
                .collect()
        },
    )
}

fn derive_in_chunks(config: Config, rows: &[Row], chunk: usize) -> Vec<DisplayItem> {
    let names: Vec<Vec<u16>> = rows.iter().map(|r| w(r.5)).collect();
    let mut deriver = Deriver::new(config);
    let mut out = Vec::new();
    for (i, (row, name)) in rows.iter().zip(&names).enumerate() {
        deriver.push(stored(row), WideStr::new(name));
        if (i + 1) % chunk == 0 {
            deriver.take(&mut out);
        }
    }
    deriver.finish();
    assert!(deriver.is_idle(), "state left after finish");
    deriver.take(&mut out);
    out
}

proptest! {
    #[test]
    fn chunking_and_page_boundaries_do_not_change_the_output(
        rows in arb_rows(),
        chunk in 1_usize..9,
        is_compact in any::<bool>(),
    ) {
        let config = if is_compact { compact() } else { Config::default() };
        let whole = run(config.clone(), &rows).0;
        prop_assert_eq!(derive_in_chunks(config.clone(), &rows, chunk), whole.clone());
        // Every row is shown exactly once and in order of first member.
        let mut ids: Vec<u64> = whole.iter().flat_map(DisplayItem::ids).collect();
        ids.sort_unstable();
        let want: Vec<u64> = rows.iter().map(|r| r.0).collect();
        prop_assert_eq!(ids, want);
        prop_assert!(whole.iter().map(DisplayItem::first_id).collect::<Vec<_>>().is_sorted_by(|a, b| a < b));
    }
}

// ---- the model ------------------------------------------------------------

#[derive(Debug, Clone)]
struct Model {
    compact: bool,
    pushed: Vec<(u64, Kind)>,
    ms: u64,
    next_id: u64,
}

#[derive(Debug, Clone)]
enum Op {
    Push {
        kind: Kind,
        path: &'static str,
        root: u16,
        file_id: u128,
        dt_ms: u64,
    },
    Advance {
        dt_ms: u64,
    },
    Finish,
}

struct DeriveModel;

impl ReferenceStateMachine for DeriveModel {
    type State = Model;
    type Transition = Op;

    fn init_state() -> BoxedStrategy<Model> {
        any::<bool>()
            .prop_map(|compact| Model {
                compact,
                pushed: Vec::new(),
                ms: 0,
                next_id: 1,
            })
            .boxed()
    }

    fn transitions(_: &Model) -> BoxedStrategy<Op> {
        let kind = prop_oneof![
            4 => Just(Modified),
            2 => Just(Created),
            2 => Just(Deleted),
            3 => Just(RenamedFrom),
            3 => Just(RenamedTo),
            1 => Just(AttributesChanged),
            1 => Just(Kind::Overflow(GapReason::BufferOverflow)),
            1 => Just(Kind::ReconciledDeleted),
            1 => Just(Kind::WatchStateChanged(WatchState::Live)),
            1 => Just(Kind::EngineStopped),
        ];
        let path = prop_oneof![
            Just("a"),
            Just("b"),
            Just("c"),
            Just("A"),
            Just("x\\a"),
            Just("")
        ];
        prop_oneof![
            12 => (kind, path, 0_u16..2, 0_u128..3, 0_u64..160)
                .prop_map(|(kind, path, root, file_id, dt_ms)| Op::Push { kind, path, root, file_id, dt_ms }),
            1 => (0_u64..600).prop_map(|dt_ms| Op::Advance { dt_ms }),
            1 => Just(Op::Finish),
        ]
        .boxed()
    }

    fn apply(mut state: Model, op: &Op) -> Model {
        match op {
            Op::Push { kind, dt_ms, .. } => {
                state.ms += dt_ms;
                state.pushed.push((state.next_id, *kind));
                state.next_id += 1;
            }
            Op::Advance { dt_ms } => state.ms += dt_ms,
            Op::Finish => {}
        }
        state
    }
}

struct Sut {
    deriver: Deriver,
    out: Vec<DisplayItem>,
}

const MODEL_MAX_OPEN: usize = 4;

impl StateMachineTest for Sut {
    type SystemUnderTest = Self;
    type Reference = DeriveModel;

    fn init_test(model: &Model) -> Self {
        let mode = if model.compact {
            ViewMode::Compact
        } else {
            ViewMode::Detail
        };
        Self {
            deriver: Deriver::new(Config {
                mode,
                max_open_groups: MODEL_MAX_OPEN,
                ..Config::default()
            }),
            out: Vec::new(),
        }
    }

    fn apply(mut sut: Self, model: &Model, op: Op) -> Self {
        match op {
            Op::Push {
                kind,
                path,
                root,
                file_id,
                ..
            } => {
                let (id, _) = *model.pushed.last().unwrap();
                let row: Row = (id, model.ms, kind, root, file_id, path);
                sut.deriver.push(stored(&row), WideStr::new(&w(path)));
            }
            Op::Advance { .. } => sut.deriver.advance(1_000_000 + model.ms * MS),
            Op::Finish => sut.deriver.finish(),
        }
        sut.deriver.take(&mut sut.out);
        if matches!(op, Op::Finish) {
            assert!(sut.deriver.is_idle(), "state left after finish");
            // Everything pushed so far is out, exactly once.
            let mut ids: Vec<u64> = sut.out.iter().flat_map(DisplayItem::ids).collect();
            ids.sort_unstable();
            let want: Vec<u64> = model.pushed.iter().map(|p| p.0).collect();
            assert_eq!(ids, want, "after finish");
        }
        sut
    }

    fn check_invariants(sut: &Self, model: &Model) {
        let stats = sut.deriver.stats();
        assert!(stats.open_groups <= MODEL_MAX_OPEN, "{stats:?}");
        assert!(stats.pending_items <= MODEL_MAX_OPEN * 4 + 1, "{stats:?}");
        // (1) items come out in order of their first member.
        assert!(
            sut.out
                .iter()
                .map(DisplayItem::first_id)
                .collect::<Vec<_>>()
                .is_sorted_by(|a, b| a < b)
        );
        // (2) every raw id at most once, and only ids that were pushed.
        let mut seen = BTreeSet::new();
        for item in &sut.out {
            for id in item.ids() {
                assert!(seen.insert(id), "id {id} twice");
                assert!(
                    model.pushed.iter().any(|p| p.0 == id),
                    "id {id} was never pushed"
                );
            }
            if let DisplayItem::Group(g) = item {
                // (3) a group counts every row it holds.
                assert_eq!(g.raw_count, g.member_ids.len());
                assert!(g.member_ids.len() >= 2);
                assert!(g.member_ids.is_sorted_by(|a, b| a < b));
                assert_eq!(Some(&g.first_id), g.member_ids.first());
                assert_eq!(Some(&g.last_id), g.member_ids.last());
                // (4) barrier rows are never grouped.
                for id in &g.member_ids {
                    let kind = model.pushed.iter().find(|p| p.0 == *id).unwrap().1;
                    assert!(
                        !matches!(
                            kind,
                            Kind::Overflow(_)
                                | Kind::WatchStateChanged(_)
                                | Kind::ReconciledCreated
                                | Kind::ReconciledModified
                                | Kind::ReconciledDeleted
                        ),
                        "barrier {id} grouped"
                    );
                }
            }
        }
    }
}

prop_state_machine! {
    #![proptest_config(ProptestConfig { cases: 300, max_shrink_iters: 200, ..ProptestConfig::default() })]

    #[test]
    fn the_deriver_never_hides_or_reorders_a_raw_row(sequential 1..80 => Sut);
}

// ---- bounded memory -------------------------------------------------------

fn bounded_run(events: u64) {
    let cfg = Config {
        max_open_groups: 256,
        ..compact()
    };
    let mut deriver = Deriver::new(cfg);
    let mut out = Vec::new();
    let mut shown = 0_u64;
    let mut peak = 0_usize;
    for id in 1..=events {
        // Every path is new and every event has the same time: nothing ever
        // expires, so only the bound keeps memory flat.
        let name = w(&format!("dir\\file-{id}"));
        let ev = Event {
            ts: 1_000_000,
            kind: Modified,
            ..sample()
        };
        deriver.push(StoredEvent { id, ev }, WideStr::new(&name));
        peak = peak.max(deriver.stats().open_groups);
        if id % 4_096 == 0 {
            deriver.take(&mut out);
            shown += u64::try_from(out.len()).unwrap();
            out.clear();
        }
    }
    deriver.finish();
    deriver.take(&mut out);
    shown += u64::try_from(out.len()).unwrap();
    assert_eq!(shown, events, "every event is shown once");
    assert!(peak <= 256, "peak {peak}");
    assert_eq!(deriver.stats().forced_flushes, events - 256);
}

#[test]
fn many_distinct_paths_keep_the_open_groups_bounded() {
    bounded_run(200_000);
}

#[test]
#[ignore = "10 million events: run with --ignored (nightly)"]
fn ten_million_distinct_paths_keep_the_open_groups_bounded() {
    bounded_run(10_000_000);
}
