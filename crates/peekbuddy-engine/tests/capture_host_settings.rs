//! The pure parts of the capture host: the settings turned into watch specs,
//! compiled filters and canonical directories (#234).
#![cfg(windows)]
#![expect(clippy::unwrap_used, reason = "tests unwrap")]

use peekbuddy_core::event::{Event, Flags, Kind, NameKind, NameRef, RootIdx, Source, VolIdx};
use peekbuddy_core::filter::Scratch;
use peekbuddy_core::name::WideStr;
use peekbuddy_core::settings::{Filters, Settings, Watch};
use peekbuddy_engine::capture_host::{
    canonical_dirs, filters_from_settings, watches_from_settings,
};

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn watch(path: &str) -> Watch {
    Watch {
        id: path.to_owned(),
        path: path.to_owned(),
        ..Watch::default()
    }
}

fn event(root: u16) -> Event {
    Event {
        ts: 1,
        qpc: 1,
        src_ts: 0,
        source: Source::Rdcw,
        kind: Kind::Created,
        raw_mask: 1,
        source_info: 0,
        is_dir: false,
        size: 1,
        attrs: 0x20,
        reparse_tag: 0,
        file_id: 1,
        parent_file_id: 5,
        usn: 0,
        vol: VolIdx(0),
        root: RootIdx(root),
        name: NameRef::EMPTY,
        name_kind: NameKind::Long,
        flags: Flags::NONE,
    }
}

fn dropped(set: &peekbuddy_core::filter::FilterSet, root: u16, name: &str) -> bool {
    let name = units(name);
    set.eval(&event(root), WideStr::new(&name), &mut Scratch::new())
        .matched_rule
        .is_some()
}

struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("pb-host-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_watch_becomes_a_spec_in_order_with_its_options() {
    let dir = TempDir::new("specs");
    let first = dir.0.join("one");
    let second = dir.0.join("two");
    std::fs::create_dir_all(&first).unwrap();
    std::fs::create_dir_all(&second).unwrap();
    let settings = Settings {
        watches: vec![
            Watch {
                recursive: true,
                buffer_kib: Some(33),
                ..watch(first.to_str().unwrap())
            },
            Watch {
                recursive: false,
                ..watch(second.to_str().unwrap())
            },
        ],
        ..Settings::default()
    };
    let specs = watches_from_settings(&settings);
    assert_eq!(specs.len(), 2);
    assert_eq!(specs[0].root, RootIdx(0));
    assert_eq!(specs[1].root, RootIdx(1));
    assert_eq!(specs[0].plain, units(first.to_str().unwrap()));
    assert!(specs[0].long.starts_with(&units(r"\\?\")));
    assert!(specs[0].recursive);
    assert!(!specs[1].recursive);
    assert_eq!(specs[0].buffer_bytes, 33 * 1024);
    assert_eq!(specs[1].buffer_bytes, 1024 * 1024);
    // Both live on one volume: the same index.
    assert_eq!(specs[0].vol, specs[1].vol);
    assert_eq!(specs[0].vol, VolIdx(0));
}

#[test]
fn a_path_that_cannot_be_opened_still_gets_a_spec() {
    let dir = TempDir::new("missing");
    let missing = dir.0.join("nothing-here");
    let settings = Settings {
        watches: vec![watch(missing.to_str().unwrap())],
        ..Settings::default()
    };
    let specs = watches_from_settings(&settings);
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].vol, VolIdx(0));
    assert_eq!(specs[0].plain, units(missing.to_str().unwrap()));
}

#[test]
fn no_rules_means_no_filter_set() {
    assert!(filters_from_settings(&Settings::default()).is_none());
}

#[test]
fn global_and_watch_rules_keep_their_scope() {
    let settings = Settings {
        filters: Filters {
            exclude_globs: vec![String::from("*.log")],
            exclude_regex: vec![String::from("cache$")],
            ..Filters::default()
        },
        watches: vec![
            Watch {
                exclude_globs: vec![String::from("*.tmp")],
                ..watch(r"C:\one")
            },
            watch(r"C:\two"),
        ],
        ..Settings::default()
    };
    let set = filters_from_settings(&settings).unwrap();
    assert_eq!(set.rule_count(), 3);
    // Global rules apply to every root.
    assert!(dropped(&set, 0, "a.log"));
    assert!(dropped(&set, 1, "a.log"));
    assert!(dropped(&set, 1, "thecache"));
    // The watch's own rule applies to its root only.
    assert!(dropped(&set, 0, "a.tmp"));
    assert!(!dropped(&set, 1, "a.tmp"));
    assert!(!dropped(&set, 0, "a.txt"));
}

#[test]
fn canonical_forms_are_given_for_existing_directories_only() {
    let dir = TempDir::new("canonical");
    let missing = dir.0.join("nothing-here");
    let forms = canonical_dirs(&[dir.0.as_path(), missing.as_path()]);
    assert_eq!(forms.len(), 1);
    let text = String::from_utf16_lossy(&forms[0]).to_lowercase();
    assert!(text.ends_with(&format!("pb-host-canonical-{}", std::process::id())));
}
