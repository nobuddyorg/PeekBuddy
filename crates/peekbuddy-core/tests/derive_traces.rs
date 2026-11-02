//! The derived views over real traces (#41, #270): the events the capture
//! backend delivered while `git`, `cargo`, `robocopy` and `tar` worked in a
//! watched tree, recorded by `peekbuddy-win/tests/record_traces.rs`. The
//! synthetic cases in `derive/tests.rs` pin the patterns; these pin that the
//! patterns hold on what real tools do: nothing is hidden, nothing is shown
//! twice, groups are what they say, and the summary of every trace is a
//! snapshot, so a change of the deriver shows up as a changed count.
#![expect(clippy::expect_used, reason = "tests expect")]

use std::collections::BTreeSet;
use std::fmt::Write as _;

use peekbuddy_core::derive::{Config, DisplayItem, GroupKind, ViewMode, derive_all};
use peekbuddy_core::event::{
    Event, Flags, Kind, NameKind, NameRef, RootIdx, Source, StoredEvent, VolIdx,
};
use peekbuddy_core::name::WideStr;
use serde_json::Value;

const TRACES: &[(&str, &str)] = &[
    (
        "cargo-build",
        include_str!("fixtures/traces/cargo-build.jsonl"),
    ),
    (
        "git-checkout",
        include_str!("fixtures/traces/git-checkout.jsonl"),
    ),
    (
        "git-commit",
        include_str!("fixtures/traces/git-commit.jsonl"),
    ),
    (
        "robocopy-mirror",
        include_str!("fixtures/traces/robocopy-mirror.jsonl"),
    ),
    (
        "tar-extract",
        include_str!("fixtures/traces/tar-extract.jsonl"),
    ),
];

/// One recorded event: its stored form and its path.
fn load(text: &str) -> (Value, Vec<(StoredEvent, Vec<u16>)>) {
    let mut lines = text
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("a JSON line"));
    let header = lines.next().expect("a header");
    let events = lines
        .enumerate()
        .map(|(index, row)| {
            let kind = row["kind"].as_array().expect("kind");
            let tag = u8::try_from(kind[0].as_u64().expect("tag")).expect("a tag");
            let arg = u8::try_from(kind[1].as_u64().expect("arg")).expect("an arg");
            let path: Vec<u16> = row["path_u16"]
                .as_array()
                .expect("path")
                .iter()
                .map(|unit| u16::try_from(unit.as_u64().expect("unit")).expect("a unit"))
                .collect();
            let file_id = u128::from_str_radix(row["file_id"].as_str().expect("id"), 16)
                .expect("a hex file id");
            (
                StoredEvent {
                    id: u64::try_from(index).expect("small") + 1,
                    ev: Event {
                        ts: 1_000_000 + row["ts"].as_u64().expect("ts"),
                        qpc: 0,
                        src_ts: 0,
                        source: Source::Rdcw,
                        kind: Kind::from_wire(tag, arg).expect("a known kind"),
                        raw_mask: 0,
                        source_info: 0,
                        is_dir: false,
                        size: 0,
                        attrs: 0,
                        reparse_tag: 0,
                        file_id,
                        parent_file_id: 0,
                        usn: 0,
                        vol: VolIdx(0),
                        root: RootIdx(0),
                        name: NameRef::EMPTY,
                        name_kind: NameKind::Long,
                        flags: Flags::NONE,
                    },
                },
                path,
            )
        })
        .collect();
    (header, events)
}

fn derived(events: &[(StoredEvent, Vec<u16>)], mode: ViewMode) -> Vec<DisplayItem> {
    let (items, _) = derive_all(
        Config {
            mode,
            ..Config::default()
        },
        events
            .iter()
            .map(|(event, path)| (*event, WideStr::new(path))),
    );
    items
}

#[test]
fn every_recorded_event_is_shown_exactly_once_in_both_modes() {
    for (name, text) in TRACES {
        let (header, events) = load(text);
        assert_eq!(
            header["events"].as_u64(),
            Some(events.len() as u64),
            "{name}: the header counts the events"
        );
        assert!(events.len() >= 50, "{name}: only {} events", events.len());
        let all: BTreeSet<u64> = events.iter().map(|(event, _)| event.id).collect();
        for mode in [ViewMode::Detail, ViewMode::Compact] {
            let items = derived(&events, mode);
            let mut seen = Vec::new();
            for item in &items {
                seen.extend(item.ids());
            }
            let unique: BTreeSet<u64> = seen.iter().copied().collect();
            assert_eq!(seen.len(), unique.len(), "{name} {mode:?}: an id twice");
            assert_eq!(unique, all, "{name} {mode:?}: an id lost");
            // The items come out in order of their first member.
            assert!(
                items
                    .windows(2)
                    .all(|pair| pair[0].first_id() <= pair[1].first_id()),
                "{name} {mode:?}: out of order"
            );
            for item in &items {
                if let DisplayItem::Group(group) = item {
                    assert_eq!(group.raw_count, group.member_ids.len(), "{name}");
                    assert!(group.raw_count >= 2, "{name}: a group of one");
                    assert_eq!(group.first_id, group.member_ids[0], "{name}");
                }
            }
        }
    }
}

#[test]
fn the_compact_view_never_shows_more_rows_than_the_detail_view() {
    for (name, text) in TRACES {
        let (_, events) = load(text);
        let detail = derived(&events, ViewMode::Detail).len();
        let compact = derived(&events, ViewMode::Compact).len();
        assert!(compact <= detail, "{name}: {compact} > {detail}");
        assert!(detail <= events.len(), "{name}");
    }
}

#[test]
fn the_save_patterns_of_real_tools_are_found() {
    let saved = |name: &str| {
        let (_, events) = load(TRACES.iter().find(|(n, _)| *n == name).unwrap().1);
        derived(&events, ViewMode::Detail)
            .iter()
            .filter(
                |item| matches!(item, DisplayItem::Group(group) if group.kind == GroupKind::Saved),
            )
            .count()
    };
    // git writes `.git/index.lock` and renames it over `.git/index`; cargo's
    // compiler and tar write a file and replace the old one.
    for name in ["git-checkout", "git-commit", "cargo-build", "tar-extract"] {
        assert!(saved(name) >= 1, "{name}: no save found");
    }
    // robocopy copies in place: no save, and no rename either.
    assert_eq!(saved("robocopy-mirror"), 0);
}

#[test]
fn the_summary_of_every_trace_is_pinned() {
    let mut out = String::new();
    for (name, text) in TRACES {
        let (header, events) = load(text);
        writeln!(
            out,
            "{name} ({}): {} events",
            header["tool"]
                .as_str()
                .unwrap()
                .split_whitespace()
                .next()
                .unwrap_or(""),
            events.len()
        )
        .unwrap();
        for mode in [ViewMode::Detail, ViewMode::Compact] {
            let items = derived(&events, mode);
            let (mut raw, mut unpaired) = (0, 0);
            let (mut renamed, mut saved, mut coalesced) = (0, 0, 0);
            for item in &items {
                match item {
                    DisplayItem::Raw(_) => raw += 1,
                    DisplayItem::Unpaired(_) => unpaired += 1,
                    DisplayItem::Group(group) => match group.kind {
                        GroupKind::Renamed => renamed += 1,
                        GroupKind::Saved => saved += 1,
                        GroupKind::Coalesced => coalesced += 1,
                    },
                }
            }
            writeln!(
                out,
                "  {mode:?}: {} rows = {raw} raw, {unpaired} unpaired, {renamed} renamed, {saved} saved, {coalesced} coalesced",
                items.len()
            )
            .unwrap();
        }
    }
    insta::assert_snapshot!("trace_summary", out);
}
