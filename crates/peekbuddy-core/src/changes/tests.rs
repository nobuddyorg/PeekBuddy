use std::collections::BTreeMap;

use proptest::prelude::*;

use super::*;

fn fold(ops: &[Op<u32>]) -> Summary<u32> {
    let mut f = Fold::new();
    for &op in ops {
        f.push(op);
    }
    f.finish()
}

fn rename(from: u32, to: u32, file_id: u128) -> [Op<u32>; 2] {
    [
        Op::RenamedFrom { key: from, file_id },
        Op::RenamedTo { key: to, file_id },
    ]
}

fn kinds(s: &Summary<u32>) -> Vec<(NetKind, u32, Option<u32>)> {
    s.rows.iter().map(|r| (r.kind, r.key, r.from)).collect()
}

#[test]
fn empty_range_has_no_change() {
    let s = fold(&[]);
    assert_eq!(s.rows, []);
    assert_eq!(s.transient, 0);
    assert!(!s.incomplete());
}

#[test]
fn created_modified_deleted_and_renamed() {
    let mut ops = vec![
        Op::Created(1),
        Op::Modified(1),
        Op::Modified(2),
        Op::Deleted(3),
    ];
    ops.extend(rename(4, 5, 9));
    let s = fold(&ops);
    assert_eq!(
        kinds(&s),
        vec![
            (NetKind::Created, 1, None),
            (NetKind::Deleted, 3, None),
            (NetKind::Modified, 2, None),
            (NetKind::Renamed, 5, Some(4)),
        ]
    );
    assert_eq!(s.raw_events, 6);
}

#[test]
fn raw_events_are_counted_per_row() {
    let s = fold(&[Op::Created(1), Op::Modified(1), Op::Modified(1)]);
    assert_eq!(s.rows.first().map(|r| r.raw_events), Some(3));
    let mut ops = vec![Op::Modified(7)];
    ops.extend(rename(7, 8, 1));
    let s = fold(&ops);
    assert_eq!(s.rows.first().map(|r| r.raw_events), Some(3));
}

#[test]
fn created_then_deleted_is_transient() {
    let s = fold(&[Op::Created(1), Op::Modified(1), Op::Deleted(1)]);
    assert_eq!(s.rows, []);
    assert_eq!(s.transient, 1);
    assert_eq!(s.raw_events, 3);
}

#[test]
fn transient_files_release_their_slot() {
    let mut f = Fold::new();
    for key in 0..1000_u32 {
        f.push(Op::Created(key));
        f.push(Op::Deleted(key));
    }
    assert_eq!(f.live_files(), 0);
    assert!(f.files.len() <= 1);
    let s = f.finish();
    assert_eq!(s.transient, 1000);
    assert_eq!(s.raw_events, 2000);
}

#[test]
fn rename_chain_is_one_row() {
    let mut ops = Vec::new();
    ops.extend(rename(1, 2, 5));
    ops.extend(rename(2, 3, 5));
    ops.extend(rename(3, 4, 5));
    let s = fold(&ops);
    assert_eq!(kinds(&s), vec![(NetKind::Renamed, 4, Some(1))]);
}

#[test]
fn rename_back_is_no_change() {
    let mut ops = Vec::new();
    ops.extend(rename(1, 2, 5));
    ops.extend(rename(2, 1, 5));
    assert_eq!(fold(&ops).rows, []);
}

#[test]
fn renamed_and_modified_says_so() {
    let mut ops = Vec::new();
    ops.extend(rename(1, 2, 5));
    ops.push(Op::Modified(2));
    let s = fold(&ops);
    assert_eq!(kinds(&s), vec![(NetKind::Renamed, 2, Some(1))]);
    assert!(s.rows.first().is_some_and(|r| r.modified));
}

#[test]
fn created_then_renamed_is_created_under_the_new_name() {
    let mut ops = vec![Op::Created(1)];
    ops.extend(rename(1, 2, 5));
    assert_eq!(kinds(&fold(&ops)), vec![(NetKind::Created, 2, None)]);
}

#[test]
fn delete_then_create_is_a_replace() {
    let s = fold(&[Op::Deleted(1), Op::Created(1)]);
    assert_eq!(kinds(&s), vec![(NetKind::Modified, 1, None)]);
    assert_eq!(s.rows.first().map(|r| r.raw_events), Some(2));
}

#[test]
fn atomic_save_is_one_modified_row() {
    // temp written, original deleted, temp renamed over it.
    let mut ops = vec![Op::Created(2), Op::Modified(2), Op::Deleted(1)];
    ops.extend(rename(2, 1, 8));
    let s = fold(&ops);
    assert_eq!(kinds(&s), vec![(NetKind::Modified, 1, None)]);
}

#[test]
fn rename_over_an_existing_file_deletes_it() {
    let mut ops = Vec::new();
    ops.extend(rename(1, 2, 5));
    let s = fold(&ops);
    assert_eq!(kinds(&s), vec![(NetKind::Renamed, 2, Some(1))]);
    let mut ops = vec![Op::Modified(2)];
    ops.extend(rename(1, 2, 5));
    let s = fold(&ops);
    // 2 is gone (it was overwritten), 1 now lives at 2.
    assert_eq!(
        kinds(&s),
        vec![(NetKind::Deleted, 2, None), (NetKind::Renamed, 2, Some(1))]
    );
}

#[test]
fn a_move_out_is_a_delete_not_a_rename() {
    let s = fold(&[Op::RenamedFrom { key: 1, file_id: 5 }, Op::Modified(9)]);
    assert_eq!(
        kinds(&s),
        vec![(NetKind::Deleted, 1, None), (NetKind::Modified, 9, None)]
    );
    // Still held at the end of the range.
    let s = fold(&[Op::RenamedFrom { key: 1, file_id: 5 }]);
    assert_eq!(kinds(&s), vec![(NetKind::Deleted, 1, None)]);
}

#[test]
fn a_move_in_is_a_create_not_a_rename() {
    let s = fold(&[Op::RenamedTo { key: 2, file_id: 5 }]);
    assert_eq!(kinds(&s), vec![(NetKind::Created, 2, None)]);
}

#[test]
fn halves_pair_by_file_id_across_other_halves() {
    let s = fold(&[
        Op::RenamedFrom {
            key: 1,
            file_id: 10,
        },
        Op::RenamedFrom {
            key: 3,
            file_id: 30,
        },
        Op::RenamedTo {
            key: 4,
            file_id: 30,
        },
        Op::RenamedTo {
            key: 2,
            file_id: 10,
        },
    ]);
    // Pairing the second half first orphans the older held half.
    assert_eq!(
        kinds(&s),
        vec![
            (NetKind::Created, 2, None),
            (NetKind::Deleted, 1, None),
            (NetKind::Renamed, 4, Some(3)),
        ]
    );
}

#[test]
fn unknown_file_ids_pair_only_when_adjacent() {
    let s = fold(&[
        Op::RenamedFrom { key: 1, file_id: 0 },
        Op::RenamedTo { key: 2, file_id: 0 },
    ]);
    assert_eq!(kinds(&s), vec![(NetKind::Renamed, 2, Some(1))]);
    let s = fold(&[
        Op::RenamedFrom { key: 1, file_id: 0 },
        Op::RenamedFrom { key: 3, file_id: 0 },
        Op::RenamedTo { key: 4, file_id: 0 },
    ]);
    assert_eq!(
        kinds(&s),
        vec![(NetKind::Deleted, 1, None), (NetKind::Renamed, 4, Some(3))]
    );
    // A known id never pairs with a different known id.
    let s = fold(&[
        Op::RenamedFrom { key: 1, file_id: 7 },
        Op::RenamedTo { key: 2, file_id: 8 },
    ]);
    assert_eq!(
        kinds(&s),
        vec![(NetKind::Created, 2, None), (NetKind::Deleted, 1, None)]
    );
}

#[test]
fn an_overflow_inside_the_range_marks_it_incomplete() {
    let s = fold(&[Op::Created(1), Op::Gap { root: 0 }, Op::Modified(1)]);
    assert!(s.incomplete());
    assert_eq!(
        s.gaps,
        vec![GapInfo {
            root: 0,
            reconciled: false
        }]
    );
}

#[test]
fn a_reconcile_covers_the_gaps_before_it_in_its_root() {
    let s = fold(&[
        Op::Gap { root: 0 },
        Op::Gap { root: 1 },
        Op::Reconciled { root: 0 },
    ]);
    assert!(s.incomplete());
    assert_eq!(
        s.gaps,
        vec![
            GapInfo {
                root: 0,
                reconciled: true
            },
            GapInfo {
                root: 1,
                reconciled: false
            },
        ]
    );
    let s = fold(&[Op::Gap { root: 1 }, Op::Reconciled { root: 1 }]);
    assert!(!s.incomplete());
}

#[test]
fn a_gap_after_a_reconcile_is_open_again() {
    let s = fold(&[
        Op::Gap { root: 0 },
        Op::Reconciled { root: 0 },
        Op::Gap { root: 0 },
    ]);
    assert!(s.incomplete());
}

#[test]
fn a_create_on_a_known_name_counts_as_a_change() {
    let s = fold(&[Op::Modified(1), Op::Created(1)]);
    assert_eq!(kinds(&s), vec![(NetKind::Modified, 1, None)]);
}

// --- reference model -----------------------------------------------------

/// A step on the simulated tree; names are in use or not as the tree says.
#[derive(Debug, Clone)]
enum Step {
    Create(u32),
    Modify(usize),
    Delete(usize),
    Rename(usize, u32),
}

/// (kind, key, from, modified)
type Row = (NetKind, u32, Option<u32>, bool);

const FILES: u32 = 6;
const NAMES: u32 = 10;

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        (0..NAMES).prop_map(Step::Create),
        (0..16_usize).prop_map(Step::Modify),
        (0..16_usize).prop_map(Step::Delete),
        (0..16_usize, 0..NAMES).prop_map(|(i, n)| Step::Rename(i, n)),
    ]
}

/// Ground truth: the tree knows every file's identity.
#[derive(Default)]
struct Tree {
    /// name -> identity.
    names: BTreeMap<u32, u32>,
    /// identity -> (origin name for files that existed, modified).
    meta: BTreeMap<u32, (Option<u32>, bool)>,
    next: u32,
}

impl Tree {
    fn live(&self) -> Vec<u32> {
        self.names.keys().copied().collect()
    }

    /// Run the steps; return the ops a watcher would have seen.
    fn run(&mut self, steps: &[Step]) -> Vec<Op<u32>> {
        let mut ops = Vec::new();
        for s in steps {
            let live = self.live();
            let pick = |i: usize| live.get(i.checked_rem(live.len())?).copied();
            match *s {
                Step::Create(n) if !self.names.contains_key(&n) => {
                    self.names.insert(n, self.next);
                    self.meta.insert(self.next, (None, false));
                    self.next += 1;
                    ops.push(Op::Created(n));
                }
                Step::Modify(i) if pick(i).is_some() => {
                    let n = pick(i).unwrap_or_default();
                    let id = self.names.get(&n).copied().unwrap_or_default();
                    if let Some(m) = self.meta.get_mut(&id) {
                        m.1 = true;
                    }
                    ops.push(Op::Modified(n));
                }
                Step::Delete(i) if pick(i).is_some() => {
                    let n = pick(i).unwrap_or_default();
                    self.names.remove(&n);
                    ops.push(Op::Deleted(n));
                }
                Step::Rename(i, to) if pick(i).is_some() => {
                    let from = pick(i).unwrap_or_default();
                    if self.names.contains_key(&to) {
                        continue;
                    }
                    let id = self.names.remove(&from).unwrap_or_default();
                    self.names.insert(to, id);
                    let file_id = u128::from(id) + 1;
                    ops.extend(rename(from, to, file_id));
                }
                _ => {}
            }
        }
        ops
    }

    /// The net change by identity, then the replace rule.
    fn expect(&self) -> (Vec<Row>, u32) {
        let now: BTreeMap<u32, u32> = self.names.iter().map(|(&n, &id)| (id, n)).collect();
        let mut rows = Vec::new();
        let mut transient = 0;
        for (&id, &(origin, modified)) in &self.meta {
            match (origin, now.get(&id).copied()) {
                (None, Some(n)) => rows.push((NetKind::Created, n, None, false)),
                (None, None) => transient += 1,
                (Some(o), None) => rows.push((NetKind::Deleted, o, None, false)),
                (Some(o), Some(n)) if o != n => {
                    rows.push((NetKind::Renamed, n, Some(o), modified));
                }
                (Some(o), Some(_)) if modified => rows.push((NetKind::Modified, o, None, true)),
                (Some(_), Some(_)) => {}
            }
        }
        let deleted: Vec<u32> = rows
            .iter()
            .filter(|r| r.0 == NetKind::Deleted)
            .map(|r| r.1)
            .collect();
        for key in deleted {
            let created = rows
                .iter()
                .position(|r| r.0 == NetKind::Created && r.1 == key);
            if let Some(c) = created {
                rows.remove(c);
                rows.retain(|r| !(r.0 == NetKind::Deleted && r.1 == key));
                rows.push((NetKind::Modified, key, None, true));
            }
        }
        rows.sort_by_key(|r| (r.0, r.1));
        (rows, transient)
    }
}

proptest! {
    /// The net diff equals a reference model applied to a simulated tree.
    #[test]
    fn fold_equals_the_reference_model(steps in proptest::collection::vec(step(), 0..60)) {
        let mut tree = Tree::default();
        for n in 0..FILES {
            tree.names.insert(n, tree.next);
            tree.meta.insert(tree.next, (Some(n), false));
            tree.next += 1;
        }
        let ops = tree.run(&steps);
        let (want, want_transient) = tree.expect();
        let s = fold(&ops);
        let got: Vec<_> = s.rows.iter().map(|r| (r.kind, r.key, r.from, r.modified)).collect();
        prop_assert_eq!(got, want);
        prop_assert_eq!(s.transient, want_transient);
        prop_assert_eq!(s.raw_events, ops.len() as u64);
    }

    /// Memory follows the surviving paths, never the raw rows.
    #[test]
    fn live_files_never_exceed_distinct_names(steps in proptest::collection::vec(step(), 0..80)) {
        let mut tree = Tree::default();
        for n in 0..FILES {
            tree.names.insert(n, tree.next);
            tree.meta.insert(tree.next, (Some(n), false));
            tree.next += 1;
        }
        let ops = tree.run(&steps);
        let mut f = Fold::new();
        for op in ops {
            f.push(op);
            prop_assert!(f.live_files() <= NAMES as usize);
        }
    }
}

#[test]
fn rows_map_to_ops_and_noise_maps_to_nothing() {
    use crate::event::{GapReason, Kind, WatchState};
    let op = |kind, is_dir| op_for(kind, is_dir, 7_u32, 9, 3);
    assert_eq!(op(Kind::Created, false), Some(Op::Created(7)));
    assert_eq!(op(Kind::Created, true), Some(Op::Created(7)));
    assert_eq!(op(Kind::Deleted, true), Some(Op::Deleted(7)));
    assert_eq!(op(Kind::ReconciledRenamed, false), Some(Op::Created(7)));
    assert_eq!(op(Kind::Modified, false), Some(Op::Modified(7)));
    assert_eq!(op(Kind::AttributesChanged, false), Some(Op::Modified(7)));
    assert_eq!(op(Kind::Modified, true), None);
    assert_eq!(op(Kind::LinkChanged, true), None);
    assert_eq!(
        op(Kind::RenamedFrom, false),
        Some(Op::RenamedFrom { key: 7, file_id: 9 })
    );
    assert_eq!(
        op(Kind::RenamedTo, true),
        Some(Op::RenamedTo { key: 7, file_id: 9 })
    );
    assert_eq!(
        op(Kind::Overflow(GapReason::RingFull), false),
        Some(Op::Gap { root: 3 })
    );
    assert_eq!(
        op(Kind::ReconcileFinished, false),
        Some(Op::Reconciled { root: 3 })
    );
    for quiet in [
        Kind::ReconcileStarted,
        Kind::Marker,
        Kind::EngineStarted,
        Kind::WatchStateChanged(WatchState::Live),
        Kind::Other,
    ] {
        assert_eq!(op(quiet, false), None, "{quiet:?}");
    }
}
