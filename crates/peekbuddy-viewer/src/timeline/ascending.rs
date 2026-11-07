//! The timeline shown oldest first (F5): follow mode and the live tail work
//! at the other end of the grid.

use std::ops::Range;

use proptest::prelude::*;

use super::{Extent, Fetch, Fetched, Identified, MAX_ROWS, Order, PAGE, Tail, Timeline};

#[derive(Debug, Clone, PartialEq, Eq)]
struct Row(i64);

impl Identified for Row {
    fn id(&self) -> i64 {
        self.0
    }
}

/// A log with the contiguous ids `1..=last`, answering like the reader.
#[derive(Debug, Clone)]
struct Store {
    last: i64,
    /// Only ids that are multiples of this show (the filter); 1 shows all.
    every: i64,
}

impl Store {
    fn new(rows: i64) -> Self {
        Self {
            last: rows,
            every: 1,
        }
    }

    fn extent(&self) -> Extent {
        Extent {
            newest: self.last,
            oldest: 1,
        }
    }

    fn shown(&self, id: i64) -> bool {
        id % self.every == 0
    }

    fn answer(&self, fetch: &Fetch) -> Fetched<Row> {
        match *fetch {
            Fetch::Older { cursor }
            | Fetch::Seek {
                cursor: Some(cursor),
                ..
            } => self.older(cursor),
            Fetch::Seek { cursor: None, .. } => self.older(self.last + 1),
            Fetch::Newer { cursor } => {
                let mut rows = Vec::new();
                let mut id = (cursor + 1).max(1);
                while id <= self.last && rows.len() < PAGE {
                    if self.shown(id) {
                        rows.push(Row(id));
                    }
                    id += 1;
                }
                let more = (id..=self.last).any(|id| self.shown(id));
                rows.reverse();
                Fetched { rows, more }
            }
        }
    }

    fn older(&self, cursor: i64) -> Fetched<Row> {
        let mut rows = Vec::new();
        let mut id = (cursor - 1).min(self.last);
        while id >= 1 && rows.len() < PAGE {
            if self.shown(id) {
                rows.push(Row(id));
            }
            id -= 1;
        }
        let more = (1..=id).any(|id| self.shown(id));
        Fetched { rows, more }
    }
}

fn settle(timeline: &mut Timeline<Row>, store: &Store, visible: &Range<u64>) {
    for _ in 0..1000 {
        let fetches = timeline.plan(visible);
        if fetches.is_empty() {
            return;
        }
        for fetch in fetches {
            timeline.apply(&fetch, store.answer(&fetch));
        }
    }
    panic!("the timeline kept asking for {visible:?}");
}

/// `settle` for the rows at the bottom, where a fresh view sits.
fn settle_newest(timeline: &mut Timeline<Row>, store: &Store) {
    let view = timeline.newest_view(40);
    settle(timeline, store, &view);
}

fn poll(timeline: &mut Timeline<Row>, store: &Store) -> Tail {
    let cursor = timeline.tail_cursor().unwrap();
    timeline.ingest_tail(store.answer(&Fetch::Newer { cursor }))
}

fn ascending(store: &Store) -> Timeline<Row> {
    Timeline::new(Some(store.extent()), true, true).with_order(Order::Ascending)
}

/// The ids of the held rows in display order.
fn shown_ids(timeline: &Timeline<Row>) -> Vec<i64> {
    let top = timeline.display_top();
    let held = u64::try_from(timeline.len()).unwrap();
    (top..top + held)
        .map(|at| timeline.row_at(at).unwrap().id())
        .collect()
}

#[test]
fn a_new_view_starts_at_the_newest_row_at_the_bottom() {
    let store = Store::new(10_000);
    let mut timeline = ascending(&store);
    assert_eq!(timeline.total(), 10_000);
    let view = timeline.newest_view(40);
    assert_eq!(view, 9_960..10_000);
    settle(&mut timeline, &store, &view);
    assert_eq!(timeline.len(), PAGE);
    assert_eq!(timeline.row_at(9_999), Some(&Row(10_000)));
    assert_eq!(timeline.row_at(9_960), Some(&Row(9_961)));
    assert_eq!(timeline.row_at(10_000), None, "past the end");
    assert_eq!(timeline.newest_row(), 9_999);
    assert!(timeline.at_follow_edge(&view));
    assert!(!timeline.at_follow_edge(&(9_000..9_040)));
    let ids = shown_ids(&timeline);
    assert!(ids.windows(2).all(|pair| pair[0] + 1 == pair[1]), "rises");
}

#[test]
fn the_other_order_is_the_same_rows_mirrored() {
    let store = Store::new(5_000);
    let mut newest_first = Timeline::new(Some(store.extent()), true, true);
    let mut oldest_first = ascending(&store);
    settle(&mut newest_first, &store, &(0..40));
    settle_newest(&mut oldest_first, &store);
    for at in 0..40 {
        assert_eq!(
            newest_first.row_at(at),
            oldest_first.row_at(4_999 - at),
            "row {at}"
        );
    }
    assert_eq!(newest_first.index_of(4_990), Some(10));
    assert_eq!(oldest_first.index_of(4_990), Some(4_989));
}

#[test]
fn a_row_that_arrives_goes_below_and_no_row_moves() {
    let mut store = Store::new(10_000);
    let mut timeline = ascending(&store);
    let view = timeline.newest_view(40);
    settle(&mut timeline, &store, &view);
    let before = timeline.row_at(9_990).cloned();
    store.last += 3;
    assert_eq!(poll(&mut timeline, &store), Tail::Merged(3));
    assert_eq!(timeline.total(), 10_003);
    assert_eq!(timeline.row_at(9_990).cloned(), before, "held rows stay");
    assert_eq!(timeline.row_at(10_002), Some(&Row(10_003)));
    assert_eq!(timeline.newest_row(), 10_002);
    // Following sits at the last row, and the view that was there is now
    // three rows short of it.
    assert!(timeline.at_follow_edge(&timeline.newest_view(40)));
    assert!(!timeline.at_follow_edge(&view));
}

#[test]
fn away_from_the_bottom_rows_are_counted_and_the_view_does_not_grow() {
    let mut store = Store::new(100_000);
    let mut timeline = ascending(&store);
    settle_newest(&mut timeline, &store);
    // The user drags the scrollbar to the middle.
    settle(&mut timeline, &store, &(40_000..40_040));
    assert!(!timeline.at_newest());
    assert_eq!(timeline.row_at(40_000), Some(&Row(40_001)));
    store.last += 5;
    assert_eq!(poll(&mut timeline, &store), Tail::Counted(5));
    assert_eq!(timeline.total(), 100_000, "the new rows wait for the pill");
    assert_eq!(timeline.row_at(40_000), Some(&Row(40_001)), "nothing moved");
    // The pill: back to the bottom, the five are part of the view.
    let jump = timeline.jump_to_newest();
    timeline.apply(&jump, store.answer(&jump));
    assert_eq!(timeline.total(), 100_005);
    assert_eq!(timeline.row_at(100_004), Some(&Row(100_005)));
    assert!(timeline.at_follow_edge(&timeline.newest_view(40)));
}

#[test]
fn scrolling_down_to_the_bottom_takes_the_rows_that_were_counted() {
    let mut store = Store::new(50_000);
    let mut timeline = ascending(&store);
    settle_newest(&mut timeline, &store);
    settle(&mut timeline, &store, &(20_000..20_040));
    store.last += 7;
    assert_eq!(poll(&mut timeline, &store), Tail::Counted(7));
    // The user scrolls back down by hand: the island extends to the end.
    for at in (20_000..50_000).step_by(2_000) {
        settle(&mut timeline, &store, &(at..at + 40));
    }
    settle(&mut timeline, &store, &(49_960..50_000));
    assert!(timeline.at_newest());
    assert_eq!(timeline.total(), 50_007);
    assert_eq!(timeline.row_at(50_006), Some(&Row(50_007)));
    assert_eq!(timeline.unseen(), 0);
}

#[test]
fn a_filtered_view_keeps_its_rows_in_place_while_older_pages_arrive() {
    let mut store = Store::new(100_000);
    store.every = 7;
    let mut timeline: Timeline<Row> =
        Timeline::new(Some(store.extent()), false, false).with_order(Order::Ascending);
    let view = timeline.newest_view(40);
    settle(&mut timeline, &store, &view);
    let newest = timeline.row_at(timeline.newest_row()).cloned();
    assert_eq!(newest, Some(Row(99_995)));
    let at = timeline.index_of(99_995).unwrap();
    // Scrolling up reads older pages; the rows already there do not move.
    let top = timeline.display_top();
    settle(&mut timeline, &store, &(top..top + 40));
    assert!(timeline.len() > PAGE);
    assert_eq!(timeline.index_of(99_995), Some(at));
    assert_eq!(timeline.row_at(at).map(Identified::id), Some(99_995));
}

#[test]
fn a_small_filtered_view_ends_where_its_rows_end() {
    let mut store = Store::new(100);
    store.every = 10;
    let mut timeline: Timeline<Row> =
        Timeline::new(Some(store.extent()), false, false).with_order(Order::Ascending);
    settle_newest(&mut timeline, &store);
    assert!(timeline.at_oldest() && timeline.at_newest());
    assert_eq!(timeline.total(), 10);
    assert_eq!(shown_ids(&timeline).first(), Some(&10));
    assert_eq!(timeline.row_at(0), Some(&Row(10)));
    assert_eq!(timeline.row_at(9), Some(&Row(100)));
}

#[test]
fn the_first_rows_of_an_empty_log_arrive_at_the_top_and_the_bottom() {
    let mut store = Store::new(0);
    let mut timeline: Timeline<Row> = Timeline::new(None, true, true).with_order(Order::Ascending);
    let view = timeline.newest_view(40);
    let fetch = timeline.plan(&view)[0];
    timeline.apply(&fetch, store.answer(&fetch));
    assert!(timeline.is_empty() && timeline.at_newest());
    store.last = 3;
    let answer = store.answer(&Fetch::Newer { cursor: 0 });
    assert_eq!(timeline.ingest_tail(answer), Tail::Merged(3));
    assert_eq!(timeline.total(), 3);
    assert_eq!(timeline.row_at(0), Some(&Row(1)));
    assert_eq!(timeline.row_at(2), Some(&Row(3)));
}

#[test]
fn hidden_rows_move_the_display_index_so_the_folds_must_be_made_again() {
    let store = Store::new(1_000);
    let mut timeline = ascending(&store);
    settle_newest(&mut timeline, &store);
    let revision = timeline.revision();
    timeline.note_hidden(100);
    assert_ne!(timeline.revision(), revision);
    assert_eq!(timeline.total(), 900);
}

#[derive(Debug, Clone)]
enum Op {
    Scroll(u64),
    Grow(u8),
    GrowLost(u8),
    Jump,
}

fn arb_ops() -> impl Strategy<Value = Vec<Op>> {
    proptest::collection::vec(
        prop_oneof![
            (0u64..60_000).prop_map(Op::Scroll),
            (1u8..40).prop_map(Op::Grow),
            (1u8..40).prop_map(Op::GrowLost),
            Just(Op::Jump),
        ],
        1..60,
    )
}

/// Runs `ops` against a 50,000-row log shown oldest first. With `exact`
/// every held row must sit at its id: that holds as long as no tail poll is
/// lost (a lost poll lets a page bring rows the extent does not know yet,
/// which shifts the ids by the rows missed, never the order).
fn run(ops: Vec<Op>, exact: bool) -> Result<(), TestCaseError> {
    let mut store = Store::new(50_000);
    let mut timeline = ascending(&store);
    settle_newest(&mut timeline, &store);
    for op in ops {
        match op {
            Op::Scroll(at) => {
                let at = at.min(timeline.total().saturating_sub(40));
                settle(&mut timeline, &store, &(at..at + 40));
                timeline.keep_near(&(at..at + 40));
            }
            Op::Grow(by) => {
                store.last += i64::from(by);
                while poll(&mut timeline, &store) != Tail::Nothing {}
            }
            Op::GrowLost(by) => store.last += i64::from(by),
            Op::Jump => {
                let jump = timeline.jump_to_newest();
                timeline.apply(&jump, store.answer(&jump));
            }
        }
        let top = timeline.display_top();
        let ids = shown_ids(&timeline);
        prop_assert!(ids.len() <= MAX_ROWS);
        prop_assert!(
            ids.windows(2).all(|pair| pair[0] + 1 == pair[1]),
            "rises without holes"
        );
        if exact {
            for (offset, id) in ids.iter().enumerate() {
                let at = top + u64::try_from(offset).unwrap();
                prop_assert_eq!(*id, i64::try_from(at).unwrap() + 1, "row {}", at);
            }
        }
        if timeline.at_newest() && !ids.is_empty() {
            let last = top + u64::try_from(ids.len()).unwrap() - 1;
            prop_assert_eq!(timeline.newest_row(), last, "the newest row is the last");
        }
    }
    Ok(())
}

proptest! {
    /// With no filter every id is a row, so oldest first the row at display
    /// index `d` has id `d + 1`: whatever the pages, the tail, the pill and
    /// the scrolling did, a row never sits anywhere else.
    #[test]
    fn oldest_first_every_held_row_sits_at_its_id(ops in arb_ops()) {
        let ops = ops.into_iter().filter(|op| !matches!(op, Op::GrowLost(_))).collect();
        run(ops, true)?;
    }

    /// Even with lost polls the island is a run without holes, and the
    /// newest row is the last one once the island has reached it.
    #[test]
    fn oldest_first_stays_hole_free_when_polls_are_lost(ops in arb_ops()) {
        run(ops, false)?;
    }
}
