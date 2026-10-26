#![expect(
    clippy::indexing_slicing,
    reason = "the model indexes arrays of three roots"
)]

use std::sync::Arc;

use proptest::prelude::*;
use proptest_state_machine::{ReferenceStateMachine, StateMachineTest, prop_state_machine};

use super::*;
use crate::event::GapReason;

const ROOTS: [RootIdx; 3] = [RootIdx(0), RootIdx(1), RootIdx(2)];

fn event(root: RootIdx, kind: Kind) -> Event {
    Transition {
        root,
        state: WatchState::Live,
        missed: 0,
        lower_bound: false,
    }
    .to_event(VolIdx(0), 1, 1)
    .with_kind(kind)
}

trait WithKind {
    fn with_kind(self, kind: Kind) -> Self;
}

impl WithKind for Event {
    fn with_kind(mut self, kind: Kind) -> Self {
        self.kind = kind;
        self
    }
}

/// Counts what reaches the inner sink.
#[derive(Debug, Default)]
struct Counting {
    events: u64,
    gaps: u64,
}

impl EventSink for Counting {
    fn push(&mut self, event: &Event, _name: &[u16]) -> bool {
        match event.kind {
            Kind::Overflow(_) => self.gaps += 1,
            _ => self.events += 1,
        }
        true
    }
}

#[test]
fn a_pause_row_then_a_resume_row_with_the_missed_count() {
    let pause = Pause::new();
    let paused = pause.set_global(true, &ROOTS);
    assert_eq!(paused.len(), 3);
    assert!(
        paused
            .iter()
            .all(|t| t.state == WatchState::SuspendedByUser && t.missed == 0)
    );
    for _ in 0..5 {
        pause.note_missed(RootIdx(1));
    }
    let resumed = pause.set_global(false, &ROOTS);
    let one = resumed.iter().find(|t| t.root == RootIdx(1)).unwrap();
    assert_eq!(
        (one.state, one.missed, one.lower_bound),
        (WatchState::Live, 5, false)
    );
    assert_eq!(pause.missed(RootIdx(1)), 0, "the count starts again");
}

#[test]
fn switching_to_the_same_state_is_not_a_change() {
    let pause = Pause::new();
    assert_eq!(pause.set_global(false, &ROOTS), []);
    assert_eq!(pause.set_watch(RootIdx(0), false), None);
    assert!(pause.set_watch(RootIdx(0), true).is_some());
    assert_eq!(pause.set_watch(RootIdx(0), true), None);
}

#[test]
fn a_watch_paused_on_its_own_stays_paused_after_a_global_resume() {
    let pause = Pause::new();
    pause.set_watch(RootIdx(0), true);
    // Only the two live watches change.
    assert_eq!(pause.set_global(true, &ROOTS).len(), 2);
    pause.note_missed(RootIdx(0));
    let resumed = pause.set_global(false, &ROOTS);
    assert_eq!(resumed.len(), 2);
    assert!(resumed.iter().all(|t| t.root != RootIdx(0)));
    assert!(pause.is_paused(RootIdx(0)));
    let own = pause.set_watch(RootIdx(0), false).unwrap();
    assert_eq!((own.state, own.missed), (WatchState::Live, 1));
}

#[test]
fn an_individual_switch_under_a_global_pause_gives_no_row() {
    let pause = Pause::new();
    pause.set_global(true, &ROOTS);
    assert_eq!(pause.set_watch(RootIdx(1), true), None);
    assert_eq!(pause.set_watch(RootIdx(1), false), None);
    assert!(pause.is_paused(RootIdx(1)));
}

#[test]
fn a_gap_while_paused_makes_the_count_a_lower_bound() {
    let pause = Arc::new(Pause::new());
    let mut sink = PausableSink::new(Counting::default(), Arc::clone(&pause));
    pause.set_watch(RootIdx(2), true);
    assert!(sink.push(&event(RootIdx(2), Kind::Created), &[]));
    assert!(sink.push(
        &event(RootIdx(2), Kind::Overflow(GapReason::BufferOverflow)),
        &[]
    ));
    let row = pause.set_watch(RootIdx(2), false).unwrap();
    assert_eq!((row.missed, row.lower_bound), (1, true));
    let stored = row.to_event(VolIdx(4), 7, 8);
    assert_eq!(stored.kind, Kind::WatchStateChanged(WatchState::Live));
    assert_eq!((stored.size, stored.source), (1, Source::System));
    assert!(stored.flags.contains(Flags::GAP_FOLLOWS));
    let inner = sink.into_inner();
    assert_eq!((inner.events, inner.gaps), (0, 0));
}

#[test]
fn live_watches_pass_through() {
    let pause = Arc::new(Pause::new());
    let mut sink = PausableSink::new(Counting::default(), Arc::clone(&pause));
    pause.set_watch(RootIdx(0), true);
    sink.push(&event(RootIdx(0), Kind::Created), &[]);
    sink.push(&event(RootIdx(1), Kind::Created), &[]);
    sink.push(&event(RootIdx(1), Kind::Overflow(GapReason::RingFull)), &[]);
    let inner = sink.into_inner();
    assert_eq!((inner.events, inner.gaps), (1, 1));
}

#[test]
fn a_root_past_the_table_is_never_paused() {
    let pause = Pause::new();
    let far = RootIdx(u16::try_from(MAX_WATCHES).unwrap() + 1);
    assert_eq!(pause.set_watch(far, true), None);
    assert!(!pause.is_paused(far));
    pause.note_missed(far);
    assert_eq!(pause.missed(far), 0);
}

#[test]
fn forgetting_a_watch_clears_its_switch_and_count() {
    let pause = Pause::new();
    pause.set_watch(RootIdx(1), true);
    pause.note_missed(RootIdx(1));
    pause.note_gap(RootIdx(1));
    pause.forget(RootIdx(1));
    assert!(!pause.is_paused(RootIdx(1)));
    assert_eq!(pause.missed(RootIdx(1)), 0);
    pause.set_watch(RootIdx(1), true);
    let row = pause.set_watch(RootIdx(1), false).unwrap();
    assert_eq!((row.missed, row.lower_bound), (0, false));
}

// The model: what the switches and counters must be after any sequence of
// operations, written without atomics.

#[derive(Debug, Clone, Default)]
struct Model {
    global: bool,
    watch: [bool; 3],
    missed: [u64; 3],
    lower: [bool; 3],
    delivered: u64,
    delivered_gaps: u64,
    rows: Vec<Transition>,
}

impl Model {
    fn paused(&self, root: usize) -> bool {
        self.global || self.watch.get(root).copied().unwrap_or(false)
    }

    fn row(&mut self, root: usize, paused: bool) {
        let root_idx = RootIdx(u16::try_from(root).unwrap());
        if paused {
            self.rows.push(Transition {
                root: root_idx,
                state: WatchState::SuspendedByUser,
                missed: 0,
                lower_bound: false,
            });
        } else {
            self.rows.push(Transition {
                root: root_idx,
                state: WatchState::Live,
                missed: std::mem::take(&mut self.missed[root]),
                lower_bound: std::mem::take(&mut self.lower[root]),
            });
        }
    }
}

#[derive(Debug, Clone)]
enum Op {
    Global(bool),
    Watch(usize, bool),
    Event(usize),
    Gap(usize),
    Forget(usize),
}

struct PauseModel;

impl ReferenceStateMachine for PauseModel {
    type State = Model;
    type Transition = Op;

    fn init_state() -> BoxedStrategy<Model> {
        Just(Model::default()).boxed()
    }

    fn transitions(_: &Model) -> BoxedStrategy<Op> {
        prop_oneof![
            any::<bool>().prop_map(Op::Global),
            (0..3_usize, any::<bool>()).prop_map(|(r, b)| Op::Watch(r, b)),
            (0..3_usize).prop_map(Op::Event),
            (0..3_usize).prop_map(Op::Event),
            (0..3_usize).prop_map(Op::Gap),
            (0..3_usize).prop_map(Op::Forget),
        ]
        .boxed()
    }

    fn apply(mut m: Model, op: &Op) -> Model {
        match *op {
            Op::Global(on) => {
                if on != m.global {
                    m.global = on;
                    for root in 0..3 {
                        if !m.watch[root] {
                            m.row(root, on);
                        }
                    }
                }
            }
            Op::Watch(root, on) => {
                if on != m.watch[root] {
                    m.watch[root] = on;
                    if !m.global {
                        m.row(root, on);
                    }
                }
            }
            Op::Event(root) => {
                if m.paused(root) {
                    m.missed[root] += 1;
                } else {
                    m.delivered += 1;
                }
            }
            Op::Gap(root) => {
                if m.paused(root) {
                    m.lower[root] = true;
                } else {
                    m.delivered_gaps += 1;
                }
            }
            Op::Forget(root) => {
                m.watch[root] = false;
                m.missed[root] = 0;
                m.lower[root] = false;
            }
        }
        m
    }
}

struct Sut {
    pause: Arc<Pause>,
    sink: PausableSink<Counting>,
    rows: Vec<Transition>,
}

impl StateMachineTest for Sut {
    type SystemUnderTest = Self;
    type Reference = PauseModel;

    fn init_test(_: &Model) -> Self {
        let pause = Arc::new(Pause::new());
        Self {
            sink: PausableSink::new(Counting::default(), Arc::clone(&pause)),
            pause,
            rows: Vec::new(),
        }
    }

    fn apply(mut sut: Self, _: &Model, op: Op) -> Self {
        let root = |r: usize| ROOTS[r];
        match op {
            Op::Global(on) => sut.rows.extend(sut.pause.set_global(on, &ROOTS)),
            Op::Watch(r, on) => sut.rows.extend(sut.pause.set_watch(root(r), on)),
            Op::Event(r) => {
                sut.sink.push(&event(root(r), Kind::Created), &[]);
            }
            Op::Gap(r) => {
                sut.sink.push(
                    &event(root(r), Kind::Overflow(GapReason::BufferOverflow)),
                    &[],
                );
            }
            Op::Forget(r) => sut.pause.forget(root(r)),
        }
        sut
    }

    fn check_invariants(sut: &Self, m: &Model) {
        // Every effective change is exactly one row, with the right count.
        assert_eq!(sut.rows, m.rows);
        // What was not delivered was counted: nothing is lost silently.
        assert_eq!(sut.sink.inner.events, m.delivered);
        assert_eq!(sut.sink.inner.gaps, m.delivered_gaps);
        for (i, root) in ROOTS.iter().enumerate() {
            assert_eq!(sut.pause.is_paused(*root), m.paused(i));
            assert_eq!(sut.pause.missed(*root), m.missed[i]);
        }
        assert_eq!(sut.pause.global(), m.global);
    }
}

prop_state_machine! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    #[test]
    fn pause_matches_its_model(sequential 1..60 => Sut);
}
