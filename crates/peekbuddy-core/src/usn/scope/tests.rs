use super::*;
use crate::usn::record::encode::{Spec, buffer, record};
use crate::usn::record::records;

const DIR: u32 = 0x10;
const FILE: u32 = 0x20;

const ROOT: u128 = 100;

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

/// One journal record as a recorded stream would hold it.
fn spec(frn: u128, parent: u128, name: &str, reason: u32, attributes: u32) -> Spec {
    Spec {
        major: 3,
        frn,
        parent,
        usn: 0,
        timestamp: 0,
        reason,
        source_info: 0,
        attributes,
        name: units(name),
    }
}

fn apply(scope: &mut Scope, spec: &Spec) -> Verdict {
    let buf = buffer(0, &[record(spec)]);
    let rec = records(&buf).unwrap().next().unwrap().unwrap();
    scope.apply(&rec)
}

fn path(verdict: &Verdict) -> String {
    String::from_utf16_lossy(&verdict.parent_path)
}

fn scope_with_tree() -> Scope {
    // root(100) \ a(101) \ b(102)
    let mut scope = Scope::new(ROOT);
    scope.insert(101, ROOT, &units("a"));
    scope.insert(102, 101, &units("b"));
    scope
}

#[test]
fn nested_directories_and_files_are_scoped_with_their_paths() {
    let mut scope = Scope::new(ROOT);
    let made = apply(&mut scope, &spec(101, ROOT, "a", reason::FILE_CREATE, DIR));
    assert!(made.accept);
    assert_eq!(path(&made), "");
    let deeper = apply(&mut scope, &spec(102, 101, "b", reason::FILE_CREATE, DIR));
    assert_eq!(path(&deeper), "a");
    let file = apply(
        &mut scope,
        &spec(200, 102, "f.txt", reason::FILE_CREATE, FILE),
    );
    assert!(file.accept);
    assert_eq!(path(&file), r"a\b");
    assert!(!file.path_partial);
    assert_eq!(scope.len(), 3);
}

#[test]
fn records_outside_the_tree_are_ignored() {
    let mut scope = scope_with_tree();
    let file = apply(
        &mut scope,
        &spec(300, 999, "x.txt", reason::DATA_EXTEND, FILE),
    );
    assert!(!file.accept);
    let dir = apply(&mut scope, &spec(301, 999, "d", reason::FILE_CREATE, DIR));
    assert!(!dir.accept);
    assert!(!scope.contains(301));
}

#[test]
fn a_rename_inside_the_tree_logs_the_old_path_first() {
    let mut scope = scope_with_tree();
    let old = apply(
        &mut scope,
        &spec(102, 101, "b", reason::RENAME_OLD_NAME, DIR),
    );
    assert!(old.accept);
    assert_eq!(path(&old), "a");
    // Moved to the root and renamed.
    let new = apply(
        &mut scope,
        &spec(102, ROOT, "c", reason::RENAME_NEW_NAME, DIR),
    );
    assert!(new.accept);
    assert_eq!(new.action, Action::None);
    // A file below it now has the new path.
    let file = apply(&mut scope, &spec(200, 102, "f", reason::DATA_EXTEND, FILE));
    assert_eq!(path(&file), "c");
}

#[test]
fn a_directory_moved_in_asks_for_a_walk_and_its_descendants_become_visible() {
    let mut scope = scope_with_tree();
    // Outside: X(500) with child Y(501); they emit nothing when X moves.
    let before = apply(&mut scope, &spec(600, 501, "f", reason::DATA_EXTEND, FILE));
    assert!(!before.accept);
    let moved = apply(
        &mut scope,
        &spec(500, ROOT, "x", reason::RENAME_NEW_NAME, DIR),
    );
    assert!(moved.accept);
    assert_eq!(moved.action, Action::Walk(500));
    // The caller walks the subtree and adds what it finds.
    scope.insert(501, 500, &units("y"));
    let after = apply(&mut scope, &spec(600, 501, "f", reason::DATA_EXTEND, FILE));
    assert!(after.accept);
    assert_eq!(path(&after), r"x\y");
}

#[test]
fn a_directory_moved_out_takes_its_members_with_it() {
    let mut scope = scope_with_tree();
    let old = apply(
        &mut scope,
        &spec(101, ROOT, "a", reason::RENAME_OLD_NAME, DIR),
    );
    assert!(old.accept);
    let out = apply(
        &mut scope,
        &spec(101, 777, "gone", reason::RENAME_NEW_NAME, DIR),
    );
    assert!(!out.accept);
    assert!(!scope.contains(101));
    assert!(!scope.contains(102));
    let file = apply(&mut scope, &spec(200, 102, "f", reason::DATA_EXTEND, FILE));
    assert!(!file.accept);
    assert_eq!(scope.len(), 1);
}

#[test]
fn a_deleted_directory_is_removed_with_its_members() {
    let mut scope = scope_with_tree();
    let gone = apply(&mut scope, &spec(101, ROOT, "a", reason::FILE_DELETE, DIR));
    assert!(gone.accept);
    assert_eq!(scope.len(), 1);
}

#[test]
fn a_reused_mft_slot_with_a_new_sequence_number_is_another_directory() {
    let mut scope = scope_with_tree();
    // Same index (102 is 0x66) but sequence 2 in the top bits of the low half.
    let reused: u128 = 0x66 | (2_u128 << 48);
    assert_ne!(reused, 102);
    let file = apply(
        &mut scope,
        &spec(200, reused, "f", reason::FILE_CREATE, FILE),
    );
    assert!(!file.accept, "a stale parent number must not match");
    // The old directory is deleted, the slot reused for a new one elsewhere.
    apply(&mut scope, &spec(102, 101, "b", reason::FILE_DELETE, DIR));
    assert!(!scope.contains(102));
    let other = apply(
        &mut scope,
        &spec(reused, 999, "z", reason::FILE_CREATE, DIR),
    );
    assert!(!other.accept);
    assert!(!scope.contains(reused));
}

#[test]
fn hard_links_are_judged_by_the_parent_of_each_record() {
    let mut scope = scope_with_tree();
    let inside = apply(
        &mut scope,
        &spec(200, 102, "link1", reason::HARD_LINK_CHANGE, FILE),
    );
    let outside = apply(
        &mut scope,
        &spec(200, 999, "link2", reason::HARD_LINK_CHANGE, FILE),
    );
    assert!(inside.accept);
    assert!(!outside.accept);
}

#[test]
fn a_withheld_name_is_resolved_later_and_the_path_is_partial_until_then() {
    let mut scope = Scope::new(ROOT);
    let made = apply(&mut scope, &spec(101, ROOT, "", reason::FILE_CREATE, DIR));
    assert_eq!(made.action, Action::NeedName(101));
    let file = apply(&mut scope, &spec(200, 101, "", reason::FILE_CREATE, FILE));
    assert!(file.accept);
    assert!(file.path_partial);
    scope.set_name(101, &units("docs"));
    let again = apply(&mut scope, &spec(200, 101, "", reason::DATA_EXTEND, FILE));
    assert!(!again.path_partial);
    assert_eq!(path(&again), "docs");
}

#[test]
fn the_root_cannot_be_removed_or_replaced() {
    let mut scope = scope_with_tree();
    assert_eq!(scope.remove_subtree(ROOT), 0);
    scope.insert(ROOT, 5, &units("evil"));
    scope.set_name(ROOT, &units("evil"));
    assert!(scope.contains(ROOT));
    let file = apply(&mut scope, &spec(200, ROOT, "f", reason::DATA_EXTEND, FILE));
    assert_eq!(path(&file), "");
}

#[test]
fn a_damaged_parent_chain_does_not_loop() {
    let mut scope = Scope::new(ROOT);
    scope.insert(1, 2, &units("a"));
    scope.insert(2, 1, &units("b"));
    let file = apply(&mut scope, &spec(200, 1, "f", reason::DATA_EXTEND, FILE));
    assert!(file.accept);
    assert!(file.path_partial);
}

#[test]
fn memory_per_directory_is_small_and_linear() {
    let mut scope = Scope::new(ROOT);
    for index in 1..=20_000_u128 {
        scope.insert(ROOT + index, ROOT, &units("dir"));
    }
    let per_dir = scope.memory_usage() / scope.len();
    // The set entry plus the name share; the target is 16 to 32 B for the
    // entry, this table keeps the parent and a name vector per directory.
    assert!(per_dir <= 160, "{per_dir} B per directory");
    assert!(per_dir >= 16);
}
