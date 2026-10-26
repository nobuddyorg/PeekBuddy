use std::sync::atomic::AtomicBool;

use peekbuddy_core::event::{GapReason, Kind, RootIdx, VolIdx};
use peekbuddy_core::reconcile::memory::{MemoryTree, Rows};
use peekbuddy_core::reconcile::walk::{Config, Summary, run};

use super::*;

fn u(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn file(id: u128, size: u64) -> Meta {
    Meta {
        file_id: id,
        attributes: 0x20,
        reparse_tag: 0,
        size,
        write_time: 5,
        change_time: 6,
    }
}

fn dir(id: u128) -> Meta {
    Meta {
        file_id: id,
        attributes: 0x10,
        ..Meta::default()
    }
}

fn clock() -> (u64, u64) {
    (1, 1)
}

fn walk(tree: &mut MemoryTree, db: &mut SnapshotDb) -> (Summary, Rows) {
    let cancel = AtomicBool::new(false);
    let cfg = Config {
        root: RootIdx(3),
        vol: VolIdx(0),
        reason: Some(GapReason::NotifyEnumDir),
        cancel: &cancel,
        clock,
    };
    let mut rows = Rows::new();
    let summary = run(&cfg, tree, db, &mut rows).unwrap();
    (summary, rows)
}

fn temp(tag: &str) -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("peekbuddy-snapshot-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path.join("snapshot.db")
}

#[test]
fn a_baseline_and_a_reconcile_against_sqlite() {
    let mut db = SnapshotDb::open_in_memory().unwrap();
    let mut tree = MemoryTree::new();
    tree.set(&u("a"), dir(1));
    tree.set(&u("a\\x.txt"), file(2, 10));
    tree.set(&u("b.txt"), file(3, 20));
    let (summary, rows) = walk(&mut tree, &mut db);
    assert!(summary.baseline);
    assert_eq!(rows.rows.len(), 1);
    assert_eq!(db.paths(3).unwrap(), *tree.entries());
    assert_eq!(db.count(3).unwrap(), 3);
    assert_eq!(db.count(4).unwrap(), 0, "other roots are separate");

    tree.unset(&u("b.txt"));
    tree.set(&u("a\\x.txt"), file(2, 11));
    tree.set(&u("a\\y.txt"), file(4, 1));
    let (summary, rows) = walk(&mut tree, &mut db);
    assert!(!summary.baseline);
    let mut got = rows.differences();
    got.sort_by_key(|(_, name)| name.clone());
    assert_eq!(
        got,
        [
            (Kind::ReconciledModified, String::from(r"a\x.txt")),
            (Kind::ReconciledCreated, String::from(r"a\y.txt")),
            (Kind::ReconciledDeleted, String::from("b.txt")),
        ]
    );
    assert_eq!(db.paths(3).unwrap(), *tree.entries());
}

#[test]
fn the_snapshot_survives_a_restart() {
    let path = temp("restart");
    let mut tree = MemoryTree::new();
    tree.set(&u("keep.txt"), file(1, 1));
    tree.set(&u("gone.txt"), file(2, 1));
    {
        let mut db = SnapshotDb::open(&path).unwrap();
        walk(&mut tree, &mut db);
    }
    tree.unset(&u("gone.txt"));
    let mut db = SnapshotDb::open(&path).unwrap();
    let (summary, rows) = walk(&mut tree, &mut db);
    assert!(!summary.baseline);
    assert_eq!(
        rows.differences(),
        [(Kind::ReconciledDeleted, String::from("gone.txt"))]
    );
    drop(db);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn a_rollback_leaves_the_previous_snapshot() {
    let mut db = SnapshotDb::open_in_memory().unwrap();
    let mut tree = MemoryTree::new();
    tree.set(&u("a.txt"), file(1, 1));
    walk(&mut tree, &mut db);
    let before = db.paths(3).unwrap();

    tree.set(&u("b.txt"), file(2, 1));
    let cancel = AtomicBool::new(false);
    let cfg = Config {
        root: RootIdx(3),
        vol: VolIdx(0),
        reason: None,
        cancel: &cancel,
        clock,
    };
    let mut rows = Rows::new();
    rows.durable = false;
    let summary = run(&cfg, &mut tree, &mut db, &mut rows).unwrap();
    assert!(summary.not_durable);
    assert_eq!(db.paths(3).unwrap(), before);
    // The database is usable afterwards.
    let (summary, _) = walk(&mut tree, &mut db);
    assert!(!summary.not_durable);
    assert_eq!(db.count(3).unwrap(), 2);
}

#[test]
fn exact_names_and_wide_ids_round_trip() {
    let mut db = SnapshotDb::open_in_memory().unwrap();
    let mut tree = MemoryTree::new();
    // Two names that differ only in a lone surrogate, a 128-bit id and a
    // size above i64::MAX.
    let lone_a: Vec<u16> = vec![0x61, 0xD800, 0x62];
    let lone_b: Vec<u16> = vec![0x61, 0xD801, 0x62];
    tree.set(&lone_a, file(u128::MAX - 1, u64::MAX));
    tree.set(&lone_b, file(1 << 100, 1 << 63));
    walk(&mut tree, &mut db);
    let back = db.paths(3).unwrap();
    assert_eq!(back.get(&lone_a), tree.entries().get(&lone_a));
    assert_eq!(back.get(&lone_b), tree.entries().get(&lone_b));
    assert_eq!(back.len(), 2);
}

#[test]
fn forgetting_a_root_makes_the_next_walk_a_baseline() {
    let mut db = SnapshotDb::open_in_memory().unwrap();
    let mut tree = MemoryTree::new();
    tree.set(&u("a.txt"), file(1, 1));
    walk(&mut tree, &mut db);
    db.forget_root(3).unwrap();
    assert_eq!(db.count(3).unwrap(), 0);
    let (summary, rows) = walk(&mut tree, &mut db);
    assert!(summary.baseline);
    assert_eq!(rows.rows.len(), 1);
}

#[test]
fn a_database_from_a_newer_build_is_refused() {
    let path = temp("toonew");
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA user_version = 99;").unwrap();
    }
    let error = SnapshotDb::open(&path).unwrap_err();
    assert!(matches!(error, OpenError::TooNew(99)), "{error}");
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}
