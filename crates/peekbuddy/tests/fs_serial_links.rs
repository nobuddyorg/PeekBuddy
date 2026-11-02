//! Links under a watched root (#49, F18): the engine logs the link object and
//! never follows it out of the root, in live capture and in the reconcile
//! walk. Real NTFS junctions, the real engine, the `fs-serial` group.
#![cfg(windows)]
#![expect(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns may unwrap"
)]

#[path = "support/engine.rs"]
mod engine;

use std::path::Path;
use std::time::Duration;

use engine::{all_rows, quit, spawn, wait_for_window, wait_until_settled};
use peekbuddy_core::event::{Kind, Source};
use peekbuddy_core::query::Row;
use peekbuddy_testkit::TestDir;
use peekbuddy_win::fs::create_junction;

fn text(row: &Row) -> String {
    String::from_utf16_lossy(&row.path)
}

/// Starts the engine on `tree`, runs `act`, lets the log settle, stops it and
/// returns every row.
fn watch(dir: &TestDir, tree: &Path, act: impl FnOnce()) -> Vec<Row> {
    watch_following(dir, tree, false, act)
}

/// As [`watch`], with `follow_link_target` as given.
fn watch_following(dir: &TestDir, tree: &Path, follow: bool, act: impl FnOnce()) -> Vec<Row> {
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    let plain = tree
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .to_owned();
    std::fs::write(
        data.join("settings.toml"),
        format!(
            "version = 1\n[general]\nfirst_run = false\n[[watch]]\nid = \"w0\"\npath = '{plain}'\nfollow_link_target = {follow}\n"
        ),
    )
    .unwrap();
    let mut engine = spawn(&data);
    let window = wait_for_window(&data, &mut engine.0);
    wait_until_settled(&data, Duration::from_secs(3));
    act();
    wait_until_settled(&data, Duration::from_secs(4));
    quit(window, &mut engine.0);
    all_rows(&data)
}

fn tree_and_sibling(dir: &TestDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let tree = dir.path().join("tree");
    let sibling = dir.path().join("sibling");
    std::fs::create_dir_all(&tree).unwrap();
    std::fs::create_dir_all(&sibling).unwrap();
    (
        std::fs::canonicalize(&tree).unwrap(),
        std::fs::canonicalize(&sibling).unwrap(),
    )
}

#[test]
fn changes_in_a_junction_target_are_not_logged_through_the_root() {
    let dir = TestDir::new("links-target").unwrap();
    let (tree, sibling) = tree_and_sibling(&dir);
    let rows = watch(&dir, &tree, || {
        create_junction(&tree.join("linked"), &sibling).unwrap();
        // Let the engine see the link before the target changes.
        std::thread::sleep(Duration::from_millis(700));
        std::fs::write(sibling.join("behind.txt"), b"x").unwrap();
        std::fs::create_dir(sibling.join("deeper")).unwrap();
        std::fs::write(sibling.join("deeper").join("more.txt"), b"x").unwrap();
    });
    let rdcw: Vec<String> = rows
        .iter()
        .filter(|row| row.source == Source::Rdcw)
        .map(text)
        .collect();
    // The link object is a row; nothing behind it is.
    assert!(
        rdcw.iter()
            .any(|path| path.to_lowercase().ends_with(r"\tree\linked")),
        "the junction itself was not logged: {rdcw:?}"
    );
    assert!(
        !rdcw
            .iter()
            .any(|path| path.contains("behind.txt") || path.contains("more.txt")),
        "a change behind the junction was logged: {rdcw:?}"
    );
}

#[test]
fn the_reconcile_walk_does_not_enter_a_junction() {
    let dir = TestDir::new("links-walk").unwrap();
    let (tree, sibling) = tree_and_sibling(&dir);
    std::fs::write(sibling.join("behind.txt"), b"x").unwrap();
    create_junction(&tree.join("linked"), &sibling).unwrap();
    std::fs::write(tree.join("own.txt"), b"x").unwrap();
    // The first start only records a baseline; the second start walks again
    // and reports differences, which must not include anything behind the link.
    let first = watch(&dir, &tree, || {});
    assert!(
        !first.iter().any(|row| text(row).contains("behind.txt")),
        "the baseline entered the junction"
    );
    std::fs::write(tree.join("new while down.txt"), b"x").unwrap();
    std::fs::write(sibling.join("also behind.txt"), b"x").unwrap();
    let second = watch(&dir, &tree, || {});
    let reconciled: Vec<String> = second
        .iter()
        .filter(|row| row.kind == Kind::ReconciledCreated)
        .map(text)
        .collect();
    assert!(
        reconciled
            .iter()
            .any(|path| path.ends_with("new while down.txt")),
        "{reconciled:?}"
    );
    assert!(
        !second.iter().any(|row| text(row).contains("behind.txt")),
        "the walk entered the junction"
    );
}

#[test]
fn a_junction_to_its_own_root_does_not_loop() {
    let dir = TestDir::new("links-loop").unwrap();
    let (tree, _) = tree_and_sibling(&dir);
    std::fs::write(tree.join("file.txt"), b"x").unwrap();
    let rows = watch(&dir, &tree, || {
        create_junction(&tree.join("loop"), &tree).unwrap();
        std::fs::write(tree.join("after.txt"), b"x").unwrap();
    });
    // The log settled (watch() waits for it) and no path runs through the loop.
    assert!(
        !rows
            .iter()
            .any(|row| text(row).to_lowercase().contains(r"\loop\")),
        "a path through the loop junction was logged"
    );
    assert!(rows.iter().any(|row| text(row).ends_with("after.txt")));
}

/// Creates a directory symlink. Without the symlink privilege (a developer
/// PC that is not elevated and has no developer mode) this is `false`; on CI,
/// where the runners are elevated, it must work.
fn symlink_dir(link: &Path, target: &Path) -> std::io::Result<bool> {
    /// `ERROR_PRIVILEGE_NOT_HELD`.
    const PRIVILEGE_NOT_HELD: i32 = 1314;

    match std::os::windows::fs::symlink_dir(target, link) {
        Ok(()) => Ok(true),
        Err(error)
            if error.raw_os_error() == Some(PRIVILEGE_NOT_HELD)
                && std::env::var_os("CI").is_none() =>
        {
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

/// The rows of the link `name` below the watched tree.
fn rows_of<'a>(rows: &'a [Row], name: &str) -> Vec<&'a Row> {
    rows.iter()
        .filter(|row| row.source == Source::Rdcw && text(row).to_lowercase().ends_with(name))
        .collect()
}

#[test]
fn each_kind_of_link_is_one_object_with_its_kind() {
    use peekbuddy_core::reparse::{LinkKind, link_kind};
    use peekbuddy_win::fs::create_app_exec_link;

    let dir = TestDir::new("links-kinds").unwrap();
    let (tree, sibling) = tree_and_sibling(&dir);
    let mut expected = vec![
        (r"\tree\junction", LinkKind::MountPoint),
        (r"\tree\alias.exe", LinkKind::AppExecLink),
    ];
    let rows = watch(&dir, &tree, || {
        create_junction(&tree.join("junction"), &sibling).unwrap();
        create_app_exec_link(
            &tree.join("alias.exe"),
            "Vendor.App_1234567890abc",
            "Vendor.App_1234567890abc!App",
            r"C:\Program Files\WindowsApps\Vendor.App\app.exe",
        )
        .unwrap();
        if symlink_dir(&tree.join("symlink"), &sibling).unwrap() {
            expected.push((r"\tree\symlink", LinkKind::Symlink));
        }
        std::thread::sleep(Duration::from_millis(700));
        // Changes behind each link are not logged through it.
        std::fs::write(sibling.join("behind.txt"), b"x").unwrap();
    });
    for (name, kind) in expected {
        let link = rows_of(&rows, name);
        assert!(!link.is_empty(), "no row for {name}");
        assert!(
            link.iter()
                .any(|row| link_kind(row.reparse_tag) == Some(kind)),
            "{name}: tags {:x?}, want {kind:?}",
            link.iter().map(|row| row.reparse_tag).collect::<Vec<_>>()
        );
        // One object: exactly one creation row.
        assert_eq!(
            link.iter().filter(|row| row.kind == Kind::Created).count(),
            1,
            "{name}"
        );
    }
    assert!(
        !rows.iter().any(|row| text(row).contains("behind.txt")),
        "a change behind a link was logged"
    );
}

#[test]
fn a_target_reached_by_two_links_is_watched_once() {
    let dir = TestDir::new("links-follow").unwrap();
    let (tree, sibling) = tree_and_sibling(&dir);
    // Two junctions to one directory, and one that points back at the root
    // (a loop that must not add a watch).
    create_junction(&tree.join("first"), &sibling).unwrap();
    create_junction(&tree.join("second"), &sibling).unwrap();
    create_junction(&tree.join("loop"), &tree).unwrap();
    let rows = watch_following(&dir, &tree, true, || {
        std::fs::write(sibling.join("followed.txt"), b"x").unwrap();
        std::fs::write(tree.join("own.txt"), b"x").unwrap();
    });
    let created: Vec<String> = rows
        .iter()
        .filter(|row| row.source == Source::Rdcw && row.kind == Kind::Created)
        .map(text)
        .filter(|path| path.contains("followed.txt"))
        .collect();
    assert_eq!(
        created.len(),
        1,
        "watched more or less than once: {created:?}"
    );
    assert!(
        created[0]
            .to_lowercase()
            .ends_with(r"\sibling\followed.txt"),
        "{created:?}"
    );
    // The tree itself is still watched, once.
    let own = rows
        .iter()
        .filter(|row| row.kind == Kind::Created && text(row).ends_with("own.txt"))
        .count();
    assert_eq!(own, 1);
}

#[test]
fn a_target_is_not_watched_unless_asked() {
    let dir = TestDir::new("links-nofollow").unwrap();
    let (tree, sibling) = tree_and_sibling(&dir);
    create_junction(&tree.join("first"), &sibling).unwrap();
    let rows = watch_following(&dir, &tree, false, || {
        std::fs::write(sibling.join("unwatched.txt"), b"x").unwrap();
    });
    assert!(!rows.iter().any(|row| text(row).contains("unwatched.txt")));
}
