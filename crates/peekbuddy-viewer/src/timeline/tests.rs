use proptest::prelude::*;

use super::{
    Extent, FAR, Fetch, Fetched, Identified, MAX_ROWS, PAGE, PREFETCH, SLACK, Tail, Timeline,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct Row(i64);

impl Identified for Row {
    fn id(&self) -> i64 {
        self.0
    }
}

/// A log with contiguous ids `first..=last`, answering like the real reader
/// (keyset, nearest first).
#[derive(Debug, Clone)]
struct Store {
    first: i64,
    last: i64,
    /// Ids that the filter hides (a multiple of this is hidden), 0 for none.
    hide_every: i64,
}

impl Store {
    fn new(rows: i64) -> Self {
        Self {
            first: 1,
            last: rows,
            hide_every: 0,
        }
    }

    fn extent(&self) -> Extent {
        Extent {
            newest: self.last,
            oldest: self.first,
        }
    }

    fn visible(&self, id: i64) -> bool {
        self.hide_every == 0 || id % self.hide_every != 0
    }

    fn answer(&self, fetch: &Fetch) -> Fetched<Row> {
        let limit = PAGE;
        match *fetch {
            Fetch::Older { cursor }
            | Fetch::Seek {
                cursor: Some(cursor),
                ..
            } => {
                let mut rows = Vec::new();
                let mut id = (cursor - 1).min(self.last);
                while id >= self.first && rows.len() < limit {
                    if self.visible(id) {
                        rows.push(Row(id));
                    }
                    id -= 1;
                }
                let more = (self.first..=id).any(|id| self.visible(id));
                Fetched { rows, more }
            }
            Fetch::Seek { cursor: None, .. } => {
                let mut rows = Vec::new();
                let mut id = self.last;
                while id >= self.first && rows.len() < limit {
                    if self.visible(id) {
                        rows.push(Row(id));
                    }
                    id -= 1;
                }
                let more = (self.first..=id).any(|id| self.visible(id));
                Fetched { rows, more }
            }
            Fetch::Newer { cursor } => {
                let mut rows = Vec::new();
                let mut id = (cursor + 1).max(self.first);
                while id <= self.last && rows.len() < limit {
                    if self.visible(id) {
                        rows.push(Row(id));
                    }
                    id += 1;
                }
                let more = (id..=self.last).any(|id| self.visible(id));
                rows.reverse();
                Fetched { rows, more }
            }
        }
    }

    fn grow(&mut self, by: i64) {
        self.last += by;
    }
}

/// Plans for `visible` and applies every answer, until nothing is asked.
fn settle(timeline: &mut Timeline<Row>, store: &Store, visible: std::ops::Range<u64>) {
    for _ in 0..1000 {
        let fetches = timeline.plan(&visible);
        if fetches.is_empty() {
            return;
        }
        for fetch in fetches {
            let answer = store.answer(&fetch);
            timeline.apply(&fetch, answer);
        }
    }
    panic!("the timeline kept asking for {visible:?}");
}

fn ids(timeline: &Timeline<Row>) -> Vec<i64> {
    timeline.iter().map(Identified::id).collect()
}

fn assert_sound(timeline: &Timeline<Row>, contiguous: bool) {
    let ids = ids(timeline);
    assert!(
        ids.windows(2).all(|pair| pair[0] > pair[1]),
        "ids must strictly decrease"
    );
    if contiguous {
        assert!(
            ids.windows(2).all(|pair| pair[0] - pair[1] == 1),
            "ids must have no holes"
        );
    }
    assert!(ids.len() <= MAX_ROWS);
}

/// The live tail polls the store from the timeline's own cursor.
fn poll(timeline: &mut Timeline<Row>, store: &Store) -> Tail {
    let cursor = timeline.tail_cursor().unwrap();
    let answer = store.answer(&Fetch::Newer { cursor });
    timeline.ingest_tail(answer)
}

fn fresh(store: &Store) -> Timeline<Row> {
    Timeline::new(Some(store.extent()), true, true)
}

#[test]
fn a_new_view_asks_for_the_newest_page_and_knows_its_size() {
    let store = Store::new(10_000_000);
    let mut timeline = fresh(&store);
    assert_eq!(timeline.total(), 10_000_000);
    assert_eq!(
        timeline.plan(&(0..40)),
        vec![Fetch::Seek {
            index: 0,
            cursor: None
        }]
    );
    // Not asked twice while the answer is out.
    assert!(timeline.plan(&(0..40)).is_empty());
    let seek = Fetch::Seek {
        index: 0,
        cursor: None,
    };
    assert!(timeline.apply(&seek, store.answer(&seek)));
    assert!(timeline.at_newest());
    assert!(!timeline.at_oldest());
    assert_eq!(timeline.top(), 0);
    assert_eq!(timeline.row_at(0), Some(&Row(10_000_000)));
    assert_eq!(timeline.row_at(199), Some(&Row(10_000_000 - 199)));
    assert_eq!(timeline.row_at(200), None);
}

#[test]
fn scrolling_down_fetches_older_pages_once_each() {
    let store = Store::new(100_000);
    let mut timeline = fresh(&store);
    settle(&mut timeline, &store, 0..40);
    assert_eq!(timeline.len(), PAGE);
    // Near the island's old edge: one fetch, not repeated while pending.
    let edge = PAGE as u64 - PREFETCH;
    let first = timeline.plan(&(edge..edge + 40));
    assert_eq!(
        first,
        vec![Fetch::Older {
            cursor: 100_000 - 199
        }]
    );
    assert!(timeline.plan(&(edge..edge + 40)).is_empty());
    let answer = store.answer(&first[0]);
    assert!(timeline.apply(&first[0], answer));
    assert_eq!(timeline.len(), 2 * PAGE);
    assert_sound(&timeline, true);
}

#[test]
fn a_far_jump_seeks_by_arithmetic_and_lands_on_the_right_row() {
    let store = Store::new(10_000_000);
    let mut timeline = fresh(&store);
    settle(&mut timeline, &store, 0..40);
    let target = 6_500_000_u64;
    let fetches = timeline.plan(&(target..target + 40));
    assert_eq!(fetches.len(), 1);
    let Fetch::Seek { index, cursor } = fetches[0] else {
        panic!("{fetches:?}");
    };
    assert_eq!(index, target);
    // Row `target` has id `newest - target`; the cursor is one above it.
    assert_eq!(cursor, Some(10_000_000 - 6_500_000 + 1));
    timeline.apply(&fetches[0], store.answer(&fetches[0]));
    assert_eq!(timeline.top(), target);
    assert_eq!(timeline.row_at(target), Some(&Row(10_000_000 - 6_500_000)));
    assert!(!timeline.at_newest());
    assert_sound(&timeline, true);
}

#[test]
fn scrolling_back_up_from_a_seek_extends_towards_the_newest() {
    let store = Store::new(1_000_000);
    let mut timeline = fresh(&store);
    settle(&mut timeline, &store, 500_000..500_040);
    let top = timeline.top();
    settle(&mut timeline, &store, top..top + 40);
    assert!(timeline.top() < top, "extended upwards");
    // The row at every held index still has the id arithmetic predicts.
    for index in timeline.top()..timeline.top() + timeline.len() as u64 {
        assert_eq!(
            timeline.row_at(index).map(Identified::id),
            Some(1_000_000 - i64::try_from(index).unwrap())
        );
    }
    assert_sound(&timeline, true);
}

#[test]
fn the_end_of_the_log_is_found_and_the_total_is_kept() {
    let store = Store::new(5_000);
    let mut timeline = fresh(&store);
    settle(&mut timeline, &store, 4_900..4_940);
    settle(&mut timeline, &store, 4_960..5_000);
    assert!(timeline.at_oldest());
    assert_eq!(timeline.total(), 5_000);
    assert_eq!(timeline.row_at(4_999), Some(&Row(1)));
    assert_eq!(timeline.row_at(5_000), None);
}

#[test]
fn memory_is_bounded_whatever_the_size_of_the_log() {
    // The island after scrolling the whole of a 10M-row log is as big as
    // after scrolling 100k rows: it never exceeds the cap.
    let mut held = Vec::new();
    for rows in [100_000_i64, 10_000_000] {
        let store = Store::new(rows);
        let mut timeline = fresh(&store);
        let mut at = 0_u64;
        while at + 40 < u64::try_from(rows).unwrap() && at < 60_000 {
            settle(&mut timeline, &store, at..at + 40);
            timeline.keep_near(&(at..at + 40));
            assert!(timeline.len() <= MAX_ROWS);
            at += 30;
        }
        held.push(timeline.len());
    }
    assert!(held.iter().all(|len| *len <= MAX_ROWS), "{held:?}");
    assert_eq!(
        held[0], held[1],
        "the cache size does not depend on the log"
    );
}

#[test]
fn trimming_drops_the_end_farther_from_the_view() {
    let store = Store::new(1_000_000);
    let mut timeline = fresh(&store);
    let mut at = 0_u64;
    while at < 20_000 {
        settle(&mut timeline, &store, at..at + 40);
        at += 100;
    }
    // Unbounded growth so far (no keep_near): trim around the bottom.
    assert!(timeline.len() > MAX_ROWS);
    timeline.keep_near(&(19_900..19_940));
    assert_eq!(timeline.len(), MAX_ROWS);
    assert!(!timeline.at_newest(), "the newest rows were dropped");
    assert!(timeline.row_at(19_900).is_some());
    assert_sound(&timeline, true);
}

#[test]
fn a_stale_answer_is_ignored() {
    let store = Store::new(100_000);
    let mut timeline = fresh(&store);
    settle(&mut timeline, &store, 0..40);
    let older = Fetch::Older { cursor: 12345 };
    assert!(!timeline.apply(&older, store.answer(&older)), "never asked");
    let edge = PAGE as u64 - PREFETCH;
    let asked = timeline.plan(&(edge..edge + 40))[0];
    // A jump supersedes the extension that is out.
    let jump = timeline.jump_to_newest();
    assert!(!timeline.apply(&asked, store.answer(&asked)));
    assert!(timeline.apply(&jump, store.answer(&jump)));
    assert_eq!(timeline.len(), PAGE);
}

#[test]
fn the_tail_prepends_while_at_the_newest_and_counts_while_away() {
    let mut store = Store::new(100_000);
    let mut timeline = fresh(&store);
    settle(&mut timeline, &store, 0..40);

    store.grow(7);
    assert_eq!(poll(&mut timeline, &store), Tail::Merged(7));
    assert_eq!(timeline.top(), 0);
    assert_eq!(timeline.row_at(0), Some(&Row(100_007)));
    assert_eq!(timeline.total(), 100_007);
    assert_eq!(timeline.unseen(), 0);
    assert_eq!(poll(&mut timeline, &store), Tail::Nothing, "nothing new");
    assert_sound(&timeline, true);

    // The user scrolls away; new rows are counted, the island stays.
    settle(&mut timeline, &store, 50_000..50_040);
    let held = ids(&timeline);
    store.grow(5);
    assert_eq!(poll(&mut timeline, &store), Tail::Counted(5));
    assert_eq!(timeline.unseen(), 5);
    assert_eq!(ids(&timeline), held, "the island did not move");
    assert_eq!(timeline.total(), 100_012);

    // The pill: back to the newest rows, counted rows are part of them.
    let jump = timeline.jump_to_newest();
    assert_eq!(timeline.unseen(), 0);
    timeline.apply(&jump, store.answer(&jump));
    assert_eq!(timeline.row_at(0), Some(&Row(100_012)));
    assert!(timeline.at_newest());
    assert_eq!(poll(&mut timeline, &store), Tail::Nothing);
}

#[test]
fn a_hole_in_the_tail_is_never_shown_and_the_next_poll_fills_it() {
    let mut store = Store::new(1_000);
    let mut timeline = fresh(&store);
    settle(&mut timeline, &store, 0..40);
    store.grow(10);
    // A frame that was dropped: the batch skips ids 1001..=1004.
    let mut answer = store.answer(&Fetch::Newer { cursor: 1_000 });
    answer.rows.retain(|row| row.0 > 1_004);
    assert_eq!(timeline.ingest_tail(answer), Tail::Hole);
    assert_eq!(timeline.row_at(0), Some(&Row(1_000)));
    assert_eq!(
        timeline.tail_cursor(),
        Some(1_000),
        "the cursor did not move"
    );
    // The next poll starts from the same cursor and brings everything.
    assert_eq!(poll(&mut timeline, &store), Tail::Merged(10));
    assert_eq!(timeline.row_at(0), Some(&Row(1_010)));
    assert_sound(&timeline, true);
}

#[test]
fn duplicates_and_old_rows_in_a_batch_are_dropped() {
    let mut store = Store::new(500);
    let mut timeline = fresh(&store);
    settle(&mut timeline, &store, 0..40);
    store.grow(3);
    let mut answer = store.answer(&Fetch::Newer { cursor: 500 });
    // The batch repeats rows the island already holds.
    answer.rows.push(Row(500));
    answer.rows.push(Row(499));
    assert_eq!(timeline.ingest_tail(answer), Tail::Merged(3));
    assert_sound(&timeline, true);
    assert_eq!(timeline.len(), PAGE + 3);
}

#[test]
fn a_tail_batch_during_a_seek_is_left_to_the_seek() {
    let mut store = Store::new(1_000);
    let mut timeline = fresh(&store);
    settle(&mut timeline, &store, 0..40);
    let jump = timeline.jump_to_newest();
    store.grow(2);
    let answer = store.answer(&Fetch::Newer { cursor: 1_000 });
    assert_eq!(timeline.ingest_tail(answer), Tail::Nothing);
    // The seek reads the database as it is now and includes the new rows.
    timeline.apply(&jump, store.answer(&jump));
    assert_eq!(timeline.row_at(0), Some(&Row(1_002)));
}

#[test]
fn a_failed_fetch_is_asked_for_again() {
    let store = Store::new(10_000);
    let mut timeline = fresh(&store);
    let first = timeline.plan(&(0..40));
    assert!(timeline.plan(&(0..40)).is_empty());
    timeline.failed(&first[0]);
    assert_eq!(timeline.plan(&(0..40)), first);
    let seek = first[0];
    timeline.apply(&seek, store.answer(&seek));
    let edge = PAGE as u64 - PREFETCH;
    let older = timeline.plan(&(edge..edge + 40));
    assert_eq!(older.len(), 1);
    assert!(timeline.plan(&(edge..edge + 40)).is_empty());
    timeline.failed(&older[0]);
    assert_eq!(timeline.plan(&(edge..edge + 40)), older);
}

#[test]
fn an_empty_log_is_an_empty_view() {
    let store = Store {
        first: 1,
        last: 0,
        hide_every: 0,
    };
    let mut timeline: Timeline<Row> = Timeline::new(None, true, true);
    let fetch = timeline.plan(&(0..40))[0];
    timeline.apply(&fetch, store.answer(&fetch));
    assert!(timeline.is_empty());
    assert_eq!(timeline.total(), 0);
    assert!(timeline.at_newest() && timeline.at_oldest());
}

#[test]
fn a_seek_past_the_end_shrinks_the_view_and_settles() {
    // The estimate said 1,000,000 rows, the store has far fewer.
    let store = Store::new(3_000);
    let mut timeline: Timeline<Row> = Timeline::new(
        Some(Extent {
            newest: 3_000,
            oldest: 1,
        }),
        true,
        false,
    );
    settle(&mut timeline, &store, 0..40);
    settle(&mut timeline, &store, 2_900..2_940);
    assert_sound(&timeline, true);
}

#[test]
fn a_filtered_view_streams_and_the_scrollbar_has_slack() {
    let store = Store::new(10_000);
    let mut timeline: Timeline<Row> = Timeline::new(None, false, false);
    assert_eq!(timeline.total(), 0);
    settle(&mut timeline, &store, 0..40);
    assert_eq!(timeline.total(), PAGE as u64 + SLACK);
    assert!(timeline.is_estimate());
    // It can not seek: the end key does nothing.
    assert!(timeline.jump_to_oldest().is_none());
    // Reading on grows the view.
    settle(&mut timeline, &store, 150..190);
    assert!(timeline.total() > PAGE as u64 + SLACK);
}

#[test]
fn hidden_rows_make_the_index_an_estimate_but_never_break_the_island() {
    // Every 10th id is hidden; the view is an estimate and ids still fall.
    let mut store = Store::new(100_000);
    store.hide_every = 10;
    let mut timeline: Timeline<Row> = Timeline::new(Some(store.extent()), true, false);
    settle(&mut timeline, &store, 0..40);
    settle(&mut timeline, &store, 70_000..70_040);
    assert_sound(&timeline, false);
    assert!(ids(&timeline).iter().all(|id| id % 10 != 0));
    assert!(timeline.is_estimate());
}

#[test]
fn the_end_key_seeks_to_the_oldest_row() {
    let store = Store::new(1_000_000);
    let mut timeline = fresh(&store);
    settle(&mut timeline, &store, 0..40);
    let fetch = timeline.jump_to_oldest().unwrap();
    timeline.apply(&fetch, store.answer(&fetch));
    assert!(timeline.at_oldest());
    assert_eq!(timeline.row_at(999_999), Some(&Row(1)));
}

#[test]
fn index_of_finds_a_held_row() {
    let store = Store::new(1_000);
    let mut timeline = fresh(&store);
    settle(&mut timeline, &store, 0..40);
    assert_eq!(timeline.index_of(1_000), Some(0));
    assert_eq!(timeline.index_of(950), Some(50));
    assert_eq!(timeline.index_of(1), None, "not held");
    assert_eq!(timeline.index_of(5_000), None);
}

#[test]
fn far_is_four_pages() {
    assert_eq!(FAR, 4 * PAGE as u64);
}

#[derive(Debug, Clone)]
enum Op {
    /// The user scrolls to this virtual row.
    Scroll(u64),
    /// The store grows and the tail polls.
    Grow(u8),
    /// The store grows but the poll is lost (a dropped frame).
    GrowLost(u8),
    /// The pill.
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

proptest! {
    #[test]
    fn any_interleaving_of_pages_and_tail_batches_stays_monotonic_and_hole_free(ops in arb_ops()) {
        let mut store = Store::new(50_000);
        let mut timeline = fresh(&store);
        settle(&mut timeline, &store, 0..40);
        for op in ops {
            match op {
                Op::Scroll(at) => {
                    settle(&mut timeline, &store, at..at + 40);
                    timeline.keep_near(&(at..at + 40));
                }
                Op::Grow(by) => {
                    store.grow(i64::from(by));
                    while poll(&mut timeline, &store) != Tail::Nothing {}
                }
                Op::GrowLost(by) => store.grow(i64::from(by)),
                Op::Jump => {
                    let jump = timeline.jump_to_newest();
                    timeline.apply(&jump, store.answer(&jump));
                }
            }
            // Whatever happened, what is held is a strictly decreasing run
            // without holes and within the cap.
            let held = ids(&timeline);
            prop_assert!(held.windows(2).all(|pair| pair[0] - pair[1] == 1), "{:?}", held.windows(2).find(|pair| pair[0] - pair[1] != 1));
            prop_assert!(held.len() <= MAX_ROWS);
            // And each row sits where the id arithmetic says, once the
            // island has been told about every new row.
            if timeline.at_newest() {
                prop_assert_eq!(timeline.top(), 0);
            }
        }
    }
}

#[test]
fn a_view_past_the_end_is_the_end() {
    // Found by the property test: Scroll(50_800) on a 50,000-row log.
    let store = Store::new(50_000);
    let mut timeline = fresh(&store);
    settle(&mut timeline, &store, 0..40);
    settle(&mut timeline, &store, 50_800..50_840);
    assert!(timeline.row_at(49_999).is_some());
    assert_sound(&timeline, true);
}

#[test]
fn the_first_rows_of_an_empty_log_arrive_through_the_tail() {
    let mut store = Store {
        first: 1,
        last: 0,
        hide_every: 0,
    };
    let mut timeline: Timeline<Row> = Timeline::new(None, true, true);
    // Before it has looked, nothing is taken.
    assert_eq!(
        timeline.ingest_tail(Fetched {
            rows: vec![Row(1)],
            more: false
        }),
        Tail::Nothing
    );
    let fetch = timeline.plan(&(0..40))[0];
    timeline.apply(&fetch, store.answer(&fetch));
    assert!(timeline.at_newest() && timeline.is_empty());
    store.last = 3;
    let answer = store.answer(&Fetch::Seek {
        index: 0,
        cursor: None,
    });
    assert_eq!(timeline.ingest_tail(answer), Tail::Merged(3));
    assert_eq!(timeline.row_at(0), Some(&Row(3)));
    assert_eq!(timeline.row_at(2), Some(&Row(1)));
    assert_eq!(timeline.tail_cursor(), Some(3));
}

#[test]
fn hidden_rows_shrink_the_scrollbar_and_are_replaced_not_added_up() {
    let store = Store::new(1_000);
    let mut timeline: Timeline<Row> = Timeline::new(Some(store.extent()), true, false);
    assert_eq!(timeline.total(), 1_000);
    timeline.note_hidden(100);
    assert_eq!(timeline.total(), 900);
    // A newer count replaces the old one.
    timeline.note_hidden(40);
    assert_eq!(timeline.total(), 960);
    // A view that streams has no use for it.
    let mut streaming: Timeline<Row> = Timeline::new(None, false, false);
    streaming.note_hidden(5);
    assert_eq!(streaming.total(), 0);
}
