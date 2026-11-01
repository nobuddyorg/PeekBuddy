use super::*;
use crate::event::map::usn_reason as reason;
use crate::usn::record::encode::{Spec, buffer, record};

const DIR: u32 = 0x10;
const FILE: u32 = 0x20;
const ROOT: u128 = 100;

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn text(units: &[u16]) -> String {
    String::from_utf16_lossy(units)
}

fn spec(frn: u128, parent: u128, name: &str, usn: i64, reason: u32, attributes: u32) -> Spec {
    Spec {
        major: 3,
        frn,
        parent,
        usn,
        timestamp: 7,
        reason,
        source_info: 0,
        attributes,
        name: units(name),
    }
}

fn batch(next_usn: i64, specs: &[Spec]) -> Vec<u8> {
    let parts: Vec<Vec<u8>> = specs.iter().map(record).collect();
    buffer(next_usn, &parts)
}

/// Takes `capacity` events, then refuses.
#[derive(Default)]
struct Sink {
    rows: Vec<(Event, String)>,
    capacity: Option<usize>,
    batch_ends: usize,
}

impl EventSink for Sink {
    fn push(&mut self, event: &Event, name: &[u16]) -> bool {
        if self.capacity.is_some_and(|cap| self.rows.len() >= cap) {
            return false;
        }
        assert_eq!(usize::from(event.name.len), name.len());
        self.rows.push((*event, text(name)));
        true
    }

    fn batch_end(&mut self) {
        self.batch_ends += 1;
    }
}

/// Answers from a table of volume-relative paths.
#[derive(Default)]
struct Fs {
    paths: Vec<(u128, &'static str)>,
    walked: Vec<u128>,
    looked_up: usize,
}

impl Names for Fs {
    fn path_of(&mut self, frn: u128, out: &mut Vec<u16>) -> bool {
        self.looked_up += 1;
        let Some((_, path)) = self.paths.iter().find(|(id, _)| *id == frn) else {
            return false;
        };
        out.clear();
        out.extend(path.encode_utf16());
        true
    }

    fn walk(&mut self, frn: u128, scope: &mut Scope) {
        self.walked.push(frn);
        scope.insert(frn + 1000, frn, &units("inner"));
    }
}

fn feed() -> Feed {
    let mut scope = Scope::new(ROOT);
    scope.insert(101, ROOT, &units("a"));
    Feed::new(scope, VolIdx(2), RootIdx(3), &units(r"\proj\"))
}

fn one(rows: &[(Event, String)]) -> &(Event, String) {
    assert_eq!(rows.len(), 1);
    rows.first().unwrap()
}

const STAMP: Stamp = Stamp { ts: 11, qpc: 12 };

#[test]
fn a_record_with_a_name_becomes_an_event_relative_to_the_root() {
    let mut feed = feed();
    let mut sink = Sink::default();
    let mut fs = Fs::default();
    let buf = batch(
        90,
        &[
            spec(200, 101, "x.txt", 50, reason::FILE_CREATE, FILE),
            spec(201, ROOT, "top.txt", 60, reason::DATA_EXTEND, FILE),
        ],
    );
    assert_eq!(feed.decode(&buf, STAMP, &mut fs, &mut sink), Ok(90));
    assert_eq!(sink.rows.len(), 2);
    let (first, first_name) = sink.rows.first().unwrap();
    let (second, second_name) = sink.rows.get(1).unwrap();
    assert_eq!(first_name, r"a\x.txt");
    assert_eq!(second_name, "top.txt");
    assert_eq!(first.kind, Kind::Created);
    assert_eq!(second.kind, Kind::Modified);
    assert_eq!(first.source, Source::Usn);
    assert_eq!((first.file_id, first.parent_file_id), (200, 101));
    assert_eq!((first.usn, first.src_ts), (50, 7));
    assert_eq!((first.vol, first.root), (VolIdx(2), RootIdx(3)));
    assert_eq!((first.ts, first.qpc), (11, 12));
    assert_eq!(first.raw_mask, reason::FILE_CREATE);
    assert_eq!(first.attrs, FILE);
    assert!(!first.is_dir);
    assert_eq!(first.flags, Flags::NONE);
    assert_eq!(sink.batch_ends, 1);
    assert_eq!(fs.looked_up, 0, "a named record needs no lookup");
    let counters = feed.counters();
    assert_eq!((counters.batches, counters.records), (1, 2));
}

#[test]
fn a_withheld_name_is_resolved_and_made_relative() {
    let mut feed = feed();
    let mut sink = Sink::default();
    let mut fs = Fs {
        paths: vec![(200, r"\PROJ\a\x.txt")],
        ..Fs::default()
    };
    let buf = batch(9, &[spec(200, 101, "", 50, reason::DATA_EXTEND, FILE)]);
    feed.decode(&buf, STAMP, &mut fs, &mut sink).unwrap();
    let (event, name) = one(&sink.rows);
    assert_eq!(name, r"a\x.txt");
    assert_eq!(event.flags, Flags::NONE);
    assert_eq!(feed.counters().unresolved, 0);
}

#[test]
fn an_unresolvable_name_still_writes_the_row_with_the_parent_path() {
    let mut feed = feed();
    let mut sink = Sink::default();
    let mut fs = Fs::default();
    let buf = batch(9, &[spec(200, 101, "", 50, reason::FILE_DELETE, FILE)]);
    feed.decode(&buf, STAMP, &mut fs, &mut sink).unwrap();
    let (event, name) = one(&sink.rows);
    assert_eq!(name, "a");
    assert_eq!(event.flags, Flags::NAME_UNRESOLVED);
    assert_eq!(event.kind, Kind::Deleted);
    assert_eq!(feed.counters().unresolved, 1);
}

#[test]
fn a_path_outside_the_root_is_unresolved_not_misfiled() {
    let mut feed = feed();
    let mut sink = Sink::default();
    // A hard link: the file's path now resolves elsewhere.
    let mut fs = Fs {
        paths: vec![(200, r"\other\x.txt"), (201, r"\proj")],
        ..Fs::default()
    };
    let buf = batch(
        9,
        &[
            spec(200, 101, "", 50, reason::DATA_EXTEND, FILE),
            spec(201, 101, "", 51, reason::DATA_EXTEND, FILE),
        ],
    );
    feed.decode(&buf, STAMP, &mut fs, &mut sink).unwrap();
    let names: Vec<_> = sink.rows.iter().map(|(_, n)| n.as_str()).collect();
    assert_eq!(names, ["a", "a"]);
    assert!(
        sink.rows
            .iter()
            .all(|(e, _)| e.flags == Flags::NAME_UNRESOLVED)
    );
}

#[test]
fn a_name_in_a_directory_without_a_name_is_flagged() {
    let mut scope = Scope::new(ROOT);
    scope.insert(101, ROOT, &[]);
    let mut feed = Feed::new(scope, VolIdx(0), RootIdx(0), &units(r"\proj"));
    let mut sink = Sink::default();
    let buf = batch(9, &[spec(200, 101, "x.txt", 50, reason::FILE_CREATE, FILE)]);
    feed.decode(&buf, STAMP, &mut Fs::default(), &mut sink)
        .unwrap();
    let (event, name) = one(&sink.rows);
    assert_eq!(name, "x.txt");
    assert_eq!(event.flags, Flags::NAME_UNRESOLVED);
}

#[test]
fn records_outside_the_tree_make_no_event() {
    let mut feed = feed();
    let mut sink = Sink::default();
    let buf = batch(
        9,
        &[spec(300, 999, "no.txt", 50, reason::FILE_CREATE, FILE)],
    );
    feed.decode(&buf, STAMP, &mut Fs::default(), &mut sink)
        .unwrap();
    assert_eq!(sink.rows.len(), 0);
    assert_eq!(feed.counters().outside, 1);
}

#[test]
fn a_resumed_feed_skips_what_was_stored_already() {
    let mut feed = feed();
    feed.resume_after(50);
    let mut sink = Sink::default();
    let buf = batch(
        90,
        &[
            spec(200, 101, "old.txt", 40, reason::FILE_CREATE, FILE),
            spec(201, 101, "last.txt", 50, reason::FILE_CREATE, FILE),
            spec(202, 101, "new.txt", 51, reason::FILE_CREATE, FILE),
        ],
    );
    feed.decode(&buf, STAMP, &mut Fs::default(), &mut sink)
        .unwrap();
    let names: Vec<_> = sink.rows.iter().map(|(_, n)| n.as_str()).collect();
    assert_eq!(names, [r"a\new.txt"]);
    assert_eq!(feed.counters().replayed, 2);
}

#[test]
fn a_full_sink_owes_one_ring_full_row_that_goes_out_first_next_time() {
    let mut feed = feed();
    let mut sink = Sink {
        capacity: Some(1),
        ..Sink::default()
    };
    let buf = batch(
        90,
        &[
            spec(200, 101, "1", 50, reason::FILE_CREATE, FILE),
            spec(201, 101, "2", 51, reason::FILE_CREATE, FILE),
            spec(202, 101, "3", 52, reason::FILE_CREATE, FILE),
        ],
    );
    feed.decode(&buf, STAMP, &mut Fs::default(), &mut sink)
        .unwrap();
    assert_eq!(sink.rows.len(), 1);
    let counters = feed.counters();
    assert_eq!((counters.records, counters.dropped_full), (1, 2));
    assert_eq!(feed.owed_total(), 1);

    sink.capacity = None;
    let buf = batch(99, &[spec(203, 101, "4", 60, reason::FILE_CREATE, FILE)]);
    feed.decode(&buf, STAMP, &mut Fs::default(), &mut sink)
        .unwrap();
    let kinds: Vec<_> = sink.rows.iter().map(|(e, _)| e.kind).collect();
    assert_eq!(
        kinds,
        [
            Kind::Created,
            Kind::Overflow(GapReason::RingFull),
            Kind::Created
        ]
    );
    assert_eq!(feed.owed_total(), 0);
    assert_eq!(feed.counters().gap_rows, 1);
}

#[test]
fn a_damaged_buffer_keeps_the_records_before_it_and_owes_a_parser_gap() {
    let mut feed = feed();
    let mut sink = Sink::default();
    let mut buf = batch(
        90,
        &[
            spec(200, 101, "ok.txt", 50, reason::FILE_CREATE, FILE),
            spec(201, 101, "bad.txt", 51, reason::FILE_CREATE, FILE),
        ],
    );
    // Cut the second record short.
    buf.truncate(buf.len() - 12);
    assert_eq!(
        feed.decode(&buf, STAMP, &mut Fs::default(), &mut sink),
        Ok(90)
    );
    let kinds: Vec<_> = sink.rows.iter().map(|(e, _)| e.kind).collect();
    assert_eq!(
        kinds,
        [Kind::Created, Kind::Overflow(GapReason::ParserError)]
    );
    assert_eq!(feed.counters().parse_errors, 1);
}

#[test]
fn a_buffer_without_a_header_is_an_error_and_a_gap() {
    let mut feed = feed();
    let mut sink = Sink::default();
    let error = feed
        .decode(&[1, 2, 3], STAMP, &mut Fs::default(), &mut sink)
        .unwrap_err();
    assert_eq!(error, DecodeError::NoHeader);
    let kinds: Vec<_> = sink.rows.iter().map(|(e, _)| e.kind).collect();
    assert_eq!(kinds, [Kind::Overflow(GapReason::ParserError)]);
}

#[test]
fn a_gap_row_is_written_at_once_or_owed_when_the_sink_is_full() {
    let mut feed = feed();
    let mut sink = Sink::default();
    assert!(feed.gap(GapReason::JournalIdChanged, STAMP, &mut sink));
    let (event, name) = one(&sink.rows);
    assert_eq!(event.kind, Kind::Overflow(GapReason::JournalIdChanged));
    assert_eq!(event.source, Source::Usn);
    assert_eq!(name, "");
    assert_eq!(feed.counters().gap_rows, 1);

    sink.capacity = Some(1);
    assert!(!feed.gap(GapReason::JournalEntryDeleted, STAMP, &mut sink));
    assert_eq!(feed.owed_total(), 1);
    assert!(!feed.flush_owed(STAMP, &mut sink));
    sink.capacity = None;
    assert!(feed.flush_owed(STAMP, &mut sink));
    assert_eq!(feed.owed_total(), 0);
    assert_eq!(feed.counters().gap_rows, 2);
}

#[test]
fn a_directory_moved_in_is_walked_and_its_contents_become_visible() {
    let mut feed = feed();
    let mut sink = Sink::default();
    let mut fs = Fs::default();
    let buf = batch(
        90,
        &[
            spec(500, 101, "moved", 50, reason::RENAME_NEW_NAME, DIR),
            spec(600, 1500, "file.txt", 51, reason::DATA_EXTEND, FILE),
        ],
    );
    feed.decode(&buf, STAMP, &mut fs, &mut sink).unwrap();
    assert_eq!(fs.walked, [500]);
    assert_eq!(feed.counters().walks, 1);
    assert!(feed.scope().contains(1500));
    let names: Vec<_> = sink.rows.iter().map(|(_, n)| n.as_str()).collect();
    assert_eq!(names, [r"a\moved", r"a\moved\inner\file.txt"]);
}

#[test]
fn a_directory_created_without_a_name_is_named_from_the_file_system() {
    let mut feed = feed();
    let mut sink = Sink::default();
    let mut fs = Fs {
        paths: vec![(500, r"\proj\a\fresh"), (600, r"\proj\a\fresh\f.txt")],
        ..Fs::default()
    };
    let buf = batch(
        90,
        &[
            spec(500, 101, "", 50, reason::FILE_CREATE, DIR),
            spec(600, 500, "", 51, reason::FILE_CREATE, FILE),
        ],
    );
    feed.decode(&buf, STAMP, &mut fs, &mut sink).unwrap();
    let names: Vec<_> = sink.rows.iter().map(|(_, n)| n.as_str()).collect();
    assert_eq!(names, [r"a\fresh", r"a\fresh\f.txt"]);
    assert!(sink.rows.iter().all(|(e, _)| e.flags == Flags::NONE));
}

#[test]
fn a_name_longer_than_an_event_can_hold_is_cut_and_flagged_not_dropped() {
    let mut feed = feed();
    let mut sink = Sink::default();
    // The longest name a record holds, below a directory: the path is longer
    // than any name an event can carry.
    let long = "x".repeat(MAX_NAME_UNITS);
    let buf = batch(9, &[spec(200, 101, &long, 50, reason::FILE_CREATE, FILE)]);
    feed.decode(&buf, STAMP, &mut Fs::default(), &mut sink)
        .unwrap();
    let (event, name) = one(&sink.rows);
    assert_eq!(name.len(), MAX_NAME_UNITS);
    assert_eq!(event.flags, Flags::NAME_UNRESOLVED);
}

#[test]
fn leaf_and_relative_path_helpers_cut_at_the_separators() {
    assert_eq!(text(leaf_of(&units(r"\a\b\c.txt"))), "c.txt");
    assert_eq!(text(leaf_of(&units("c.txt"))), "c.txt");
    assert_eq!(text(leaf_of(&units(r"\a\"))), "");
    let root = units(r"\Proj");
    assert_eq!(
        relative_to(&units(r"\proj\a\b"), &root).map(text),
        Some(r"a\b".to_owned())
    );
    assert_eq!(relative_to(&units(r"\proj"), &root), None);
    assert_eq!(relative_to(&units(r"\proj\"), &root), None);
    assert_eq!(relative_to(&units(r"\projx\a"), &root), None);
    assert_eq!(relative_to(&units(r"\pr"), &root), None);
    // A volume root: everything is below it.
    assert_eq!(
        relative_to(&units(r"\a\b"), &[]).map(text),
        Some(r"a\b".to_owned())
    );
}
