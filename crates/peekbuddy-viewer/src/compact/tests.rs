use std::collections::BTreeSet;

use peekbuddy_core::derive::GroupKind;
use peekbuddy_core::event::{Kind, Source};
use peekbuddy_core::query::Row;
use proptest::prelude::*;

use super::{Fold, Folds};
use crate::rows::GridRow;

/// 10 ms in FILETIME ticks.
const MS10: u64 = 100_000;

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn grid(id: i64, ms10: u64, kind: Kind, path: &str) -> GridRow {
    GridRow::new(
        Row {
            id,
            ts: 134_374_896_000_000_000 + ms10 * MS10,
            kind,
            source: Source::Rdcw,
            raw_mask: 3,
            is_dir: false,
            size: None,
            attrs: None,
            reparse_tag: 0,
            file_id: u128::try_from(id).unwrap_or(0),
            name: Vec::new(),
            path: units(path),
            old_path: Vec::new(),
            root: units("C:\\w"),
            flags: 0,
        },
        &|_| 0,
    )
}

/// Newest first: three writes to one file, then a change to another file.
fn rows() -> Vec<GridRow> {
    vec![
        grid(5, 40, Kind::Created, r"C:\w\other.txt"),
        grid(4, 3, Kind::Modified, r"C:\w\a.txt"),
        grid(3, 2, Kind::Modified, r"C:\w\a.txt"),
        grid(2, 1, Kind::Modified, r"C:\w\a.txt"),
        grid(1, 0, Kind::Created, r"C:\w\far.txt"),
    ]
}

#[test]
fn the_newest_member_heads_a_group_and_the_older_ones_fold_away() {
    let rows = rows();
    let folds = Folds::build(rows.iter(), 0, &BTreeSet::new());
    assert_eq!(
        folds.of(4),
        Fold::Head {
            kind: GroupKind::Coalesced,
            members: 3,
            expanded: false
        }
    );
    assert_eq!(folds.of(3), Fold::Folded);
    assert_eq!(folds.of(2), Fold::Folded);
    assert_eq!(folds.of(5), Fold::Plain);
    assert_eq!(folds.of(1), Fold::Plain);
    assert_eq!(folds.folded(), 2);
}

#[test]
fn folded_rows_are_counted_by_position() {
    let rows = rows();
    let folds = Folds::build(rows.iter(), 100, &BTreeSet::new());
    // Rows 100..105 are held; ids 3 and 2 sit at 102 and 103.
    assert_eq!(folds.hidden_before(0), 0);
    assert_eq!(folds.hidden_before(102), 0);
    assert_eq!(folds.hidden_before(103), 1);
    assert_eq!(folds.hidden_before(104), 2);
    assert_eq!(folds.hidden_before(10_000), 2);
    assert!(!folds.is_folded(101));
    assert!(folds.is_folded(102));
    assert!(folds.is_folded(103));
    assert!(!folds.is_folded(104));
    assert!(!folds.is_folded(10_000));
}

#[test]
fn an_expanded_group_shows_every_member_and_folds_nothing() {
    let rows = rows();
    let expanded = BTreeSet::from([4]);
    let folds = Folds::build(rows.iter(), 0, &expanded);
    assert_eq!(folds.of(3), Fold::Inside);
    assert_eq!(folds.of(2), Fold::Inside);
    assert_eq!(folds.folded(), 0);
    assert!((0..5).all(|row| !folds.is_folded(row)));
}

#[test]
fn the_raw_view_folds_nothing() {
    let folds = Folds::none();
    assert_eq!(folds.of(4), Fold::Plain);
    assert_eq!(folds.folded(), 0);
    assert_eq!(folds.hidden_before(50), 0);
    assert!(!folds.is_folded(0));
}

#[test]
fn a_badge_says_how_many_events_stand_behind_a_row() {
    assert_eq!(Fold::badge(3, GroupKind::Saved), "3 events, saved");
    assert_eq!(Fold::badge(2, GroupKind::Renamed), "2 events, renamed");
    assert_eq!(Fold::badge(9, GroupKind::Coalesced), "9 events, changed");
}

#[test]
fn rows_that_are_not_paths_are_never_folded() {
    let rows = [
        grid(3, 3, Kind::EngineStarted, ""),
        grid(2, 2, Kind::EngineStarted, ""),
        grid(1, 1, Kind::EngineStarted, ""),
    ];
    assert_eq!(Folds::build(rows.iter(), 0, &BTreeSet::new()).folded(), 0);
}

fn kind_of(code: u8) -> Kind {
    match code % 3 {
        0 => Kind::Created,
        1 => Kind::Modified,
        _ => Kind::Deleted,
    }
}

proptest! {
    /// With every group expanded the grid shows exactly the raw rows; with
    /// none expanded the rows left are the raw rows minus the folded ones,
    /// and a group always keeps its head.
    #[test]
    fn expanding_all_groups_gives_back_exactly_the_raw_rows(
        steps in prop::collection::vec((0_u8..4, 0_u8..4, 0_u64..8), 1..80)
    ) {
        let mut ms = 0;
        let mut rows: Vec<GridRow> = Vec::new();
        for (at, (path, kind, gap)) in (1_i64..).zip(steps) {
            ms += gap;
            rows.push(grid(at, ms, kind_of(kind), &format!(r"C:\w\f{path}.txt")));
        }
        rows.reverse();
        let collapsed = Folds::build(rows.iter(), 0, &BTreeSet::new());
        let heads: BTreeSet<i64> = collapsed.heads().collect();
        let expanded = Folds::build(rows.iter(), 0, &heads);
        prop_assert_eq!(expanded.folded(), 0);
        let shown = rows
            .iter()
            .filter(|row| collapsed.of(row.id) != Fold::Folded)
            .count();
        prop_assert_eq!(
            u64::try_from(shown).unwrap() + collapsed.folded(),
            u64::try_from(rows.len()).unwrap()
        );
        // A folded row always has a head above it (towards the newest).
        for (at, row) in rows.iter().enumerate() {
            if collapsed.of(row.id) == Fold::Folded {
                let head_above = rows[..at]
                    .iter()
                    .any(|above| collapsed.is_head(above.id));
                prop_assert!(head_above, "row {} is folded with no head", row.id);
            }
        }
        // The room a row takes never shrinks towards the oldest, and a folded
        // row takes none.
        for row in 0..u64::try_from(rows.len()).unwrap() {
            let (here, next) = (collapsed.shown_before(row), collapsed.shown_before(row + 1));
            prop_assert_eq!(next - here, u64::from(!collapsed.is_folded(row)));
        }
        for row in 0..u64::try_from(rows.len()).unwrap() {
            prop_assert_eq!(
                collapsed.is_folded(row),
                collapsed.of(rows[usize::try_from(row).unwrap()].id) == Fold::Folded
            );
        }
    }
}
