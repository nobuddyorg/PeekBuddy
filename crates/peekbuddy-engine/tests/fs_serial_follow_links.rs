//! The link search of `follow_link_targets` (#265, F18) on real junctions:
//! which watches ask for it, what it adds and what it reports.
#![cfg(windows)]
#![expect(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns may unwrap"
)]

use std::path::{Path, PathBuf};

use peekbuddy_core::settings::{Settings, Watch};
use peekbuddy_engine::links::{MAX_TARGETS, describe, follow_link_targets};
use peekbuddy_win::fs::create_junction;

/// A fresh directory below the temp dir, removed on drop.
struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!("pb-links-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(std::fs::canonicalize(&path).unwrap())
    }

    fn make(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::create_dir_all(&path).unwrap();
        path
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn plain(path: &Path) -> String {
    path.to_string_lossy()
        .trim_start_matches(r"\\?\")
        .to_owned()
}

fn watch(id: &str, path: &Path, follow: bool) -> Watch {
    Watch {
        id: id.to_owned(),
        path: plain(path),
        follow_link_target: follow,
        ..Watch::default()
    }
}

fn settings(watches: Vec<Watch>) -> Settings {
    Settings {
        watches,
        ..Settings::default()
    }
}

#[test]
fn a_junction_target_becomes_a_derived_watch_of_its_own() {
    let dir = Dir::new("derived");
    let tree = dir.make("tree");
    let sibling = dir.make("sibling");
    create_junction(&tree.join("linked"), &sibling).unwrap();
    let mut settings = settings(vec![Watch {
        buffer_kib: Some(64),
        ..watch("w0", &tree, true)
    }]);

    let report = follow_link_targets(&mut settings);

    assert_eq!(report.followed, 1);
    assert_eq!(settings.watches.len(), 2);
    let added = &settings.watches[1];
    assert_eq!(added.id, "w0.link1");
    assert_eq!(added.path, plain(&sibling));
    assert_eq!(added.derived_from.as_deref(), Some("w0"));
    assert!(!added.follow_link_target);
    assert_eq!(added.buffer_kib, Some(64));
    assert!(added.recursive && added.enabled);
}

#[test]
fn a_target_is_searched_for_links_of_its_own() {
    let dir = Dir::new("chain");
    let tree = dir.make("tree");
    let first = dir.make("first");
    let second = dir.make("second");
    create_junction(&tree.join("a"), &first).unwrap();
    create_junction(&first.join("b"), &second).unwrap();
    let mut settings = settings(vec![watch("w0", &tree, true)]);

    let report = follow_link_targets(&mut settings);

    assert_eq!(report.followed, 2);
    let paths: Vec<&str> = settings.watches.iter().map(|w| w.path.as_str()).collect();
    assert!(paths.contains(&plain(&first).as_str()));
    assert!(paths.contains(&plain(&second).as_str()));
}

#[test]
fn only_an_enabled_recursive_watch_that_asks_for_it_is_searched() {
    let dir = Dir::new("asks");
    let mut watches = Vec::new();
    for (name, follow, enabled, recursive) in [
        ("plain", false, true, true),
        ("paused", true, false, true),
        ("shallow", true, true, false),
    ] {
        let tree = dir.make(name);
        let target = dir.make(&format!("{name}-target"));
        create_junction(&tree.join("linked"), &target).unwrap();
        watches.push(Watch {
            enabled,
            recursive,
            ..watch(name, &tree, follow)
        });
    }
    let mut settings = settings(watches);

    let report = follow_link_targets(&mut settings);

    assert_eq!(report.followed, 0);
    assert_eq!(settings.watches.len(), 3);
}

#[test]
fn two_links_to_one_directory_add_it_once() {
    let dir = Dir::new("twice");
    let tree = dir.make("tree");
    let sibling = dir.make("sibling");
    create_junction(&tree.join("first"), &sibling).unwrap();
    create_junction(&tree.join("second"), &sibling).unwrap();
    let mut settings = settings(vec![watch("w0", &tree, true)]);

    let report = follow_link_targets(&mut settings);

    assert_eq!((report.followed, report.duplicates), (1, 1));
    assert_eq!(settings.watches.len(), 2);
}

#[test]
fn a_link_back_to_the_watched_root_is_not_added() {
    let dir = Dir::new("loop");
    let tree = dir.make("tree");
    create_junction(&tree.join("loop"), &tree).unwrap();
    let mut settings = settings(vec![watch("w0", &tree, true)]);

    let report = follow_link_targets(&mut settings);

    assert_eq!((report.followed, report.duplicates), (0, 1));
    assert_eq!(settings.watches.len(), 1);
}

#[test]
fn a_link_whose_target_is_gone_is_counted_as_unreadable() {
    let dir = Dir::new("dangling");
    let tree = dir.make("tree");
    let gone = dir.make("gone");
    create_junction(&tree.join("linked"), &gone).unwrap();
    std::fs::remove_dir(&gone).unwrap();
    let mut settings = settings(vec![watch("w0", &tree, true)]);

    let report = follow_link_targets(&mut settings);

    assert_eq!((report.followed, report.unreadable), (0, 1));
}

#[test]
fn no_more_than_the_most_targets_are_added() {
    let dir = Dir::new("many");
    let tree = dir.make("tree");
    for at in 0..=MAX_TARGETS {
        let target = dir.make(&format!("target{at}"));
        create_junction(&tree.join(format!("link{at}")), &target).unwrap();
    }
    let mut settings = settings(vec![watch("w0", &tree, true)]);

    follow_link_targets(&mut settings);

    assert_eq!(settings.watches.len(), 1 + MAX_TARGETS);
}

#[test]
fn the_log_line_names_every_count() {
    let report = peekbuddy_core::reparse::follow::Report {
        followed: 1,
        duplicates: 2,
        overlapping: 3,
        unreadable: 4,
    };

    assert_eq!(
        describe(report),
        "links: followed 1, duplicate 2, overlapping 3, unreadable 4"
    );
}
