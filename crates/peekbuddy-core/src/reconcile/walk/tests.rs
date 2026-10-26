#![expect(
    clippy::indexing_slicing,
    reason = "tests index small fixed vectors they just built"
)]

use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;

use proptest::prelude::*;

use super::*;
use crate::reconcile::memory::{MemorySnapshot, MemoryTree, Rows};

fn u(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn path_text(path: &[u16]) -> String {
    String::from_utf16_lossy(path)
}

const DIR: u32 = 0x10;
const ARCHIVE: u32 = 0x20;
const REPARSE_DIR: u32 = 0x410;

fn file(id: u128, size: u64, write: i64, change: i64) -> Meta {
    Meta {
        file_id: id,
        attributes: ARCHIVE,
        reparse_tag: 0,
        size,
        write_time: write,
        change_time: change,
    }
}

fn dir(id: u128) -> Meta {
    Meta {
        file_id: id,
        attributes: DIR,
        ..Meta::default()
    }
}

fn tree(entries: &[(&str, Meta)]) -> MemoryTree {
    let mut tree = MemoryTree::new();
    for (path, meta) in entries {
        tree.set(&u(path), *meta);
    }
    tree
}

fn clock() -> (u64, u64) {
    (1, 1)
}

fn reconcile(
    tree: &mut MemoryTree,
    snapshot: &mut MemorySnapshot,
    reason: Option<GapReason>,
) -> (Summary, Rows) {
    let cancel = AtomicBool::new(false);
    let cfg = Config {
        root: RootIdx(1),
        vol: VolIdx(0),
        reason,
        cancel: &cancel,
        clock,
    };
    let mut rows = Rows::new();
    let summary = run(&cfg, tree, snapshot, &mut rows).unwrap();
    (summary, rows)
}

fn kinds(rows: &Rows) -> Vec<Kind> {
    rows.rows.iter().map(|(kind, ..)| *kind).collect()
}

fn baselined(entries: &[(&str, Meta)]) -> (MemoryTree, MemorySnapshot) {
    let mut tree = tree(entries);
    let mut snapshot = MemorySnapshot::new();
    let (summary, _) = reconcile(&mut tree, &mut snapshot, None);
    assert!(summary.baseline);
    (tree, snapshot)
}

#[test]
fn the_first_walk_writes_the_snapshot_and_one_baseline_row() {
    let mut t = tree(&[
        ("a", dir(1)),
        ("a\\x.txt", file(2, 1, 1, 1)),
        ("b.txt", file(3, 1, 1, 1)),
    ]);
    let mut snapshot = MemorySnapshot::new();
    let (summary, rows) = reconcile(&mut t, &mut snapshot, None);
    assert_eq!(kinds(&rows), [Kind::BaselineEstablished]);
    let row = &rows.rows[0].2;
    assert_eq!((row.size, row.file_id), (2, 1), "files, dirs");
    assert_eq!((summary.files, summary.dirs, summary.changed), (2, 1, 0));
    assert_eq!(snapshot.paths(1), *t.entries());
}

#[test]
fn a_baseline_of_a_100k_entry_tree_is_one_row() {
    let mut t = MemoryTree::new();
    for d in 0..1000_u128 {
        t.set(&u(&format!("d{d}")), dir(d + 1));
        for f in 0..99_u128 {
            t.set(
                &u(&format!("d{d}\\f{f}")),
                file(10_000 + d * 100 + f, 1, 1, 1),
            );
        }
    }
    assert_eq!(t.entries().len(), 99_000 + 1000);
    let mut snapshot = MemorySnapshot::new();
    let (summary, rows) = reconcile(&mut t, &mut snapshot, None);
    assert_eq!(kinds(&rows), [Kind::BaselineEstablished]);
    assert_eq!(summary.scanned, 100_000);
    assert_eq!(snapshot.len(), 100_000);
}

#[test]
fn an_unchanged_tree_gives_only_the_state_rows() {
    let (mut t, mut snapshot) = baselined(&[("a", dir(1)), ("a\\x", file(2, 1, 1, 1))]);
    let (summary, rows) = reconcile(&mut t, &mut snapshot, Some(GapReason::BufferOverflow));
    assert_eq!(
        kinds(&rows),
        [Kind::ReconcileStarted, Kind::ReconcileFinished]
    );
    // The gap that asked for it is in the started row (tag + 1).
    assert_eq!(rows.rows[0].2.raw_mask, 1);
    assert_eq!((rows.rows[1].2.size, summary.changed), (2, 0));
}

#[test]
fn every_difference_case_gives_exactly_its_rows() {
    // create
    let (mut t, mut s) = baselined(&[("a.txt", file(1, 1, 1, 1))]);
    t.set(&u("new.txt"), file(2, 1, 1, 1));
    let (_, rows) = reconcile(&mut t, &mut s, None);
    assert_eq!(
        rows.differences(),
        [(Kind::ReconciledCreated, String::from("new.txt"))]
    );

    // delete
    let (mut t, mut s) = baselined(&[("a.txt", file(1, 1, 1, 1)), ("b.txt", file(2, 1, 1, 1))]);
    t.unset(&u("b.txt"));
    let (_, rows) = reconcile(&mut t, &mut s, None);
    assert_eq!(
        rows.differences(),
        [(Kind::ReconciledDeleted, String::from("b.txt"))]
    );

    // modify by size, by write time, attribute-only via change time
    for changed in [
        file(1, 2, 1, 1),
        file(1, 1, 2, 1),
        file(1, 1, 1, 2),
        Meta {
            attributes: ARCHIVE | 0x1,
            ..file(1, 1, 1, 1)
        },
    ] {
        let (mut t, mut s) = baselined(&[("a.txt", file(1, 1, 1, 1))]);
        t.set(&u("a.txt"), changed);
        let (_, rows) = reconcile(&mut t, &mut s, None);
        assert_eq!(
            rows.differences(),
            [(Kind::ReconciledModified, String::from("a.txt"))],
            "{changed:?}"
        );
    }

    // rename in place
    let (mut t, mut s) = baselined(&[("old.txt", file(7, 1, 1, 1))]);
    t.unset(&u("old.txt"));
    t.set(&u("new.txt"), file(7, 1, 1, 1));
    let (_, rows) = reconcile(&mut t, &mut s, None);
    assert_eq!(
        rows.differences(),
        [(Kind::ReconciledRenamed, String::from("new.txt"))]
    );

    // move across directories by file id
    let (mut t, mut s) = baselined(&[("x", dir(1)), ("y", dir(2)), ("x\\f.txt", file(9, 1, 1, 1))]);
    t.unset(&u("x\\f.txt"));
    t.set(&u("y\\f.txt"), file(9, 1, 1, 1));
    let (_, rows) = reconcile(&mut t, &mut s, None);
    assert_eq!(
        rows.differences(),
        [(Kind::ReconciledRenamed, String::from("y\\f.txt"))]
    );
    assert_eq!(s.paths(1), *t.entries());
}

#[test]
fn a_directory_replaced_by_a_file_with_the_same_name() {
    let (mut t, mut s) = baselined(&[("thing", dir(1)), ("thing\\inner.txt", file(2, 1, 1, 1))]);
    t.unset(&u("thing\\inner.txt"));
    t.unset(&u("thing"));
    t.set(&u("thing"), file(3, 5, 1, 1));
    let (_, rows) = reconcile(&mut t, &mut s, None);
    let mut got = rows.differences();
    got.sort_by_key(|(_, name)| name.clone());
    assert_eq!(
        got,
        [
            (Kind::ReconciledCreated, String::from("thing")),
            (Kind::ReconciledDeleted, String::from("thing")),
            (Kind::ReconciledDeleted, String::from("thing\\inner.txt")),
        ]
    );
    assert_eq!(s.paths(1), *t.entries());
}

#[test]
fn a_deleted_directory_takes_its_subtree_with_it() {
    let (mut t, mut s) = baselined(&[
        ("d", dir(1)),
        ("d\\e", dir(2)),
        ("d\\e\\f.txt", file(3, 1, 1, 1)),
        ("keep.txt", file(4, 1, 1, 1)),
    ]);
    for path in ["d\\e\\f.txt", "d\\e", "d"] {
        t.unset(&u(path));
    }
    let (_, rows) = reconcile(&mut t, &mut s, None);
    let mut got: Vec<String> = rows
        .differences()
        .into_iter()
        .map(|(kind, name)| {
            assert_eq!(kind, Kind::ReconciledDeleted);
            name
        })
        .collect();
    got.sort();
    assert_eq!(got, ["d", "d\\e", "d\\e\\f.txt"]);
    assert_eq!(s.paths(1), *t.entries());
}

#[test]
fn a_moved_directory_pairs_every_entry_below_it() {
    let (mut t, mut s) = baselined(&[
        ("old", dir(1)),
        ("old\\a.txt", file(2, 1, 1, 1)),
        ("old\\sub", dir(3)),
        ("old\\sub\\b.txt", file(4, 1, 1, 1)),
    ]);
    for path in ["old\\sub\\b.txt", "old\\sub", "old\\a.txt", "old"] {
        t.unset(&u(path));
    }
    t.set(&u("new"), dir(1));
    t.set(&u("new\\a.txt"), file(2, 1, 1, 1));
    t.set(&u("new\\sub"), dir(3));
    t.set(&u("new\\sub\\b.txt"), file(4, 1, 1, 1));
    let (_, rows) = reconcile(&mut t, &mut s, None);
    assert!(
        rows.differences()
            .iter()
            .all(|(kind, _)| *kind == Kind::ReconciledRenamed),
        "{:?}",
        rows.differences()
    );
    assert_eq!(rows.differences().len(), 4);
    assert_eq!(s.paths(1), *t.entries());
}

#[test]
fn a_reparse_point_is_logged_but_never_traversed() {
    let junction = Meta {
        attributes: REPARSE_DIR,
        reparse_tag: 0xA000_0003,
        ..dir(5)
    };
    let (mut t, mut s) = baselined(&[("real", dir(1))]);
    t.set(&u("link"), junction);
    // What the junction points to would be listed if it were followed.
    t.set(&u("link\\inside.txt"), file(6, 1, 1, 1));
    let listed_before = t.listed;
    let (summary, rows) = reconcile(&mut t, &mut s, None);
    assert_eq!(
        rows.differences(),
        [(Kind::ReconciledCreated, String::from("link"))]
    );
    assert_eq!(summary.dirs, 2);
    // Only the root and `real` were listed.
    assert_eq!(t.listed - listed_before, 2);
    assert!(!s.paths(1).contains_key(&u("link\\inside.txt")));
}

#[test]
fn a_denied_directory_is_one_row_per_run_and_keeps_its_snapshot() {
    let (mut t, mut s) = baselined(&[
        ("locked", dir(1)),
        ("locked\\secret.txt", file(2, 1, 1, 1)),
        ("open.txt", file(3, 1, 1, 1)),
    ]);
    t.deny(&u("locked"));
    let (summary, rows) = reconcile(&mut t, &mut s, None);
    assert_eq!(
        rows.differences(),
        [(Kind::AccessDenied, String::from("locked"))]
    );
    assert_eq!(summary.denied, 1);
    assert!(s.paths(1).contains_key(&u("locked\\secret.txt")));
}

/// A source that cancels the walk after `after` directories.
struct CancelAfter<'a> {
    inner: MemoryTree,
    flag: &'a AtomicBool,
    after: usize,
}

impl DirSource for CancelAfter<'_> {
    fn list(&mut self, path: &[u16], out: &mut Listing) -> Result<(), ListError> {
        let result = self.inner.list(path, out);
        if self.inner.listed >= self.after {
            self.flag.store(true, Ordering::Release);
        }
        result
    }
}

#[test]
fn a_cancel_stops_within_a_directory_and_keeps_the_previous_snapshot() {
    let (mut t, mut s) = baselined(&[
        ("a", dir(1)),
        ("a\\x", file(2, 1, 1, 1)),
        ("b", dir(3)),
        ("b\\y", file(4, 1, 1, 1)),
    ]);
    t.listed = 0;
    let before = s.paths(1);
    t.set(&u("a\\new"), file(9, 1, 1, 1));
    t.set(&u("b\\new"), file(10, 1, 1, 1));
    let cancel = AtomicBool::new(false);
    let mut source = CancelAfter {
        inner: t,
        flag: &cancel,
        after: 2,
    };
    let cfg = Config {
        root: RootIdx(1),
        vol: VolIdx(0),
        reason: None,
        cancel: &cancel,
        clock,
    };
    let mut rows = Rows::new();
    let summary = run(&cfg, &mut source, &mut s, &mut rows).unwrap();
    assert!(summary.cancelled);
    // The root and one more directory were listed, then it stopped.
    assert_eq!(source.inner.listed, 2);
    assert_eq!(s.paths(1), before, "the previous snapshot is intact");
    assert_eq!(
        rows.rows.last().map(|r| (r.0, r.2.raw_mask)),
        Some((Kind::ReconcileFinished, 1))
    );
}

#[test]
fn rows_that_are_not_durable_keep_the_snapshot_where_it_was() {
    let (mut t, mut s) = baselined(&[("a.txt", file(1, 1, 1, 1))]);
    let before = s.paths(1);
    t.set(&u("b.txt"), file(2, 1, 1, 1));
    let cancel = AtomicBool::new(false);
    let cfg = Config {
        root: RootIdx(1),
        vol: VolIdx(0),
        reason: None,
        cancel: &cancel,
        clock,
    };
    let mut rows = Rows::new();
    rows.durable = false;
    let summary = run(&cfg, &mut t, &mut s, &mut rows).unwrap();
    assert!(summary.not_durable);
    assert_eq!(s.paths(1), before);
    // The next run, with durable rows, writes the rows again and moves on.
    let (summary, rows) = reconcile(&mut t, &mut s, None);
    assert!(!summary.not_durable);
    assert_eq!(
        rows.differences(),
        [(Kind::ReconciledCreated, String::from("b.txt"))]
    );
}

#[test]
fn a_denied_root_does_not_establish_a_baseline() {
    let mut t = tree(&[("a.txt", file(1, 1, 1, 1))]);
    t.deny(&[]);
    let mut s = MemorySnapshot::new();
    let (summary, rows) = reconcile(&mut t, &mut s, None);
    assert_eq!(summary.denied, 1);
    assert_eq!(kinds(&rows), []);
    // Once the root can be read, the baseline is written.
    let mut t = tree(&[("a.txt", file(1, 1, 1, 1))]);
    let (summary, _) = reconcile(&mut t, &mut s, None);
    assert!(summary.baseline);
}

// A random model: a tree with unique ids, mutated, reconciled and compared.

#[derive(Debug, Clone)]
enum Op {
    NewFile(usize, u8),
    NewDir(usize, u8),
    Delete(usize),
    Touch(usize),
    Move(usize, usize, u8),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        (0..50_usize, 0..6_u8).prop_map(|(d, n)| Op::NewFile(d, n)),
        (0..50_usize, 0..6_u8).prop_map(|(d, n)| Op::NewDir(d, n)),
        (0..50_usize).prop_map(Op::Delete),
        (0..50_usize).prop_map(Op::Touch),
        (0..50_usize, 0..50_usize, 0..6_u8).prop_map(|(a, d, n)| Op::Move(a, d, n)),
    ]
}

struct Model {
    entries: BTreeMap<String, Meta>,
    next_id: u128,
    clock: i64,
}

impl Model {
    fn dirs(&self) -> Vec<String> {
        let mut dirs = vec![String::new()];
        dirs.extend(
            self.entries
                .iter()
                .filter(|(_, m)| m.is_dir())
                .map(|(p, _)| p.clone()),
        );
        dirs
    }

    fn pick(&self, index: usize) -> Option<String> {
        let keys: Vec<&String> = self.entries.keys().collect();
        (!keys.is_empty()).then(|| keys[index % keys.len()].clone())
    }

    fn child(dir: &str, name: u8) -> String {
        if dir.is_empty() {
            format!("n{name}")
        } else {
            format!("{dir}\\n{name}")
        }
    }

    fn apply(&mut self, op: &Op) {
        self.clock += 1;
        match *op {
            Op::NewFile(d, n) | Op::NewDir(d, n) => {
                let dirs = self.dirs();
                let path = Self::child(&dirs[d % dirs.len()], n);
                if self.entries.contains_key(&path) {
                    return;
                }
                self.next_id += 1;
                let meta = if matches!(op, Op::NewDir(..)) {
                    dir(self.next_id)
                } else {
                    file(self.next_id, 1, self.clock, self.clock)
                };
                self.entries.insert(path, meta);
            }
            Op::Delete(i) => {
                if let Some(path) = self.pick(i) {
                    let prefix = format!("{path}\\");
                    self.entries
                        .retain(|p, _| *p != path && !p.starts_with(&prefix));
                }
            }
            Op::Touch(i) => {
                if let Some(path) = self.pick(i) {
                    let clock = self.clock;
                    if let Some(meta) = self.entries.get_mut(&path).filter(|m| !m.is_dir()) {
                        meta.size += 1;
                        meta.write_time = clock;
                        meta.change_time = clock;
                    }
                }
            }
            Op::Move(i, d, n) => {
                let Some(from) = self.pick(i) else { return };
                let dirs = self.dirs();
                let target_dir = dirs[d % dirs.len()].clone();
                let to = Self::child(&target_dir, n);
                let prefix = format!("{from}\\");
                if to == from || to.starts_with(&prefix) || self.entries.contains_key(&to) {
                    return;
                }
                let moved: Vec<(String, Meta)> = self
                    .entries
                    .iter()
                    .filter(|(p, _)| **p == from || p.starts_with(&prefix))
                    .map(|(p, m)| (p.clone(), *m))
                    .collect();
                for (path, _) in &moved {
                    self.entries.remove(path);
                }
                for (path, meta) in moved {
                    let renamed = format!("{to}{}", &path[from.len()..]);
                    self.entries.insert(renamed, meta);
                }
            }
        }
    }

    fn source(&self) -> MemoryTree {
        let mut tree = MemoryTree::new();
        for (path, meta) in &self.entries {
            tree.set(&u(path), *meta);
        }
        tree
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 200, ..ProptestConfig::default() })]

    #[test]
    fn a_reconcile_leaves_the_snapshot_equal_to_the_tree_and_explains_every_change(
        setup in proptest::collection::vec(op(), 0..25),
        mutations in proptest::collection::vec(op(), 1..25),
    ) {
        let mut model = Model { entries: BTreeMap::new(), next_id: 100, clock: 10 };
        for step in &setup {
            model.apply(step);
        }
        let mut snapshot = MemorySnapshot::new();
        let mut source = model.source();
        let (baseline, _) = reconcile(&mut source, &mut snapshot, None);
        prop_assert!(baseline.baseline);
        prop_assert_eq!(snapshot.paths(1), source.entries().clone());

        let before = model.entries.clone();
        for step in &mutations {
            model.apply(step);
        }
        let mut source = model.source();
        let (_, rows) = reconcile(&mut source, &mut snapshot, None);
        prop_assert_eq!(snapshot.paths(1), source.entries().clone());

        // Every difference is explained by a row: a path that is new or
        // changed has a row under its path; a path that is gone has a delete
        // row, or its file id shows up in a rename row.
        let renamed_ids: Vec<u128> = rows
            .rows
            .iter()
            .filter(|(kind, ..)| *kind == Kind::ReconciledRenamed)
            .map(|(_, _, event)| event.file_id)
            .collect();
        let row_paths: Vec<(Kind, String)> = rows.differences();
        let has = |path: &str, kinds: &[Kind]| {
            row_paths.iter().any(|(k, p)| p == path && kinds.contains(k))
        };
        for (path, meta) in &model.entries {
            match before.get(path) {
                None => prop_assert!(
                    has(path, &[Kind::ReconciledCreated, Kind::ReconciledRenamed]),
                    "new {path} has no row"
                ),
                Some(old) if meta.differs_from(old) => prop_assert!(
                    has(path, &[
                        Kind::ReconciledModified,
                        Kind::ReconciledCreated,
                        Kind::ReconciledRenamed,
                    ]),
                    "changed {path} has no row"
                ),
                Some(_) => {}
            }
        }
        for (path, meta) in &before {
            if !model.entries.contains_key(path) {
                prop_assert!(
                    has(path, &[Kind::ReconciledDeleted]) || renamed_ids.contains(&meta.file_id),
                    "gone {path} has no row"
                );
            }
        }

        // Nothing is left to find.
        let (again, rows) = reconcile(&mut source, &mut snapshot, None);
        prop_assert_eq!(again.changed, 0);
        prop_assert_eq!(rows.differences(), vec![]);
    }
}

#[test]
fn path_text_helper_round_trips() {
    assert_eq!(path_text(&u("a\\b")), "a\\b");
}
