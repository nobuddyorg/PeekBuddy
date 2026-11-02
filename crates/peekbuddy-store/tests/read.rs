//! The read side against real segment files (#55): keyset paging across
//! days, every filter against a naive model, a retired segment, cancel and
//! the glob post-filter.
#![expect(clippy::unwrap_used, reason = "tests unwrap")]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use peekbuddy_core::event::{Event, Flags, Kind, NameKind, NameRef, RootIdx, Source, VolIdx};
use peekbuddy_core::filter::{FilterSet, KindSet, Pattern, Rule};
use peekbuddy_core::query::{
    CancelFlag, Direction, Filter, PAGE_SIZE, Page, PageRequest, TimeRange,
};
use peekbuddy_store::batch::Batch;
use peekbuddy_store::read::{Reader, explain, explain_selective};
use peekbuddy_store::retention::{Caps, NoViewer};
use peekbuddy_store::segment::{DayFn, DayKey, Rotation};
use peekbuddy_store::writer::{RootInfo, SegmentsConfig, VolInfo, Writer};
use proptest::prelude::*;
use proptest::test_runner::{Config as ProptestConfig, TestRunner};
use rusqlite::{Connection, OpenFlags};

const HOUR: u64 = 36_000_000_000;
const DAY: u64 = 24 * HOUR;
const ROOT: &str = r"\\?\C:\Watched";

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("pb-read-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// One event of the model.
#[derive(Debug, Clone)]
struct Item {
    ts: u64,
    kind: Kind,
    rel: String,
}

impl Item {
    fn path(&self) -> String {
        format!(r"C:\Watched\{}", self.rel)
    }

    fn name(&self) -> &str {
        self.rel.rsplit('\\').next().unwrap()
    }
}

fn event(ts: u64, kind: Kind) -> Event {
    Event {
        ts,
        qpc: ts,
        src_ts: 0,
        source: Source::Rdcw,
        kind,
        raw_mask: 1,
        source_info: 0,
        is_dir: false,
        size: 1,
        attrs: 0x20,
        reparse_tag: 0,
        file_id: u128::from(ts),
        parent_file_id: 5,
        usn: 0,
        vol: VolIdx(0),
        root: RootIdx(0),
        name: NameRef::EMPTY,
        name_kind: NameKind::Long,
        flags: Flags::NONE,
    }
}

const NAMES: [&str; 8] = [
    r"proj\src\main.rs",
    r"proj\src\lib.rs",
    r"proj\docs\readme.md",
    r"tmp\a_b%c.txt",
    r"tmp\aXbYc.txt",
    r"x.y",
    r"proj\src\deep\mod.rs",
    r"other\Readme.MD",
];

const KINDS: [Kind; 4] = [
    Kind::Created,
    Kind::Modified,
    Kind::Deleted,
    Kind::RenamedTo,
];

fn day_of() -> DayFn {
    Arc::new(|ts| DayKey {
        year: 2026,
        month: 10,
        day: u8::try_from(1 + ts / DAY).unwrap_or(28).min(28),
    })
}

/// A store with `count` events spread over `days` days, and its model.
fn build(dir: &Dir, days: u64, count: u64) -> Vec<Item> {
    let plan: Vec<(usize, usize)> = (0..count)
        .map(|index| {
            let index = usize::try_from(index).unwrap();
            (index % NAMES.len(), (index / 3) % KINDS.len())
        })
        .collect();
    build_items(dir, days, &plan)
}

/// A store of one event per `(name, kind)` of `plan` (indexes into `NAMES` and
/// `KINDS`), spread over `days` days, and its model.
fn build_items(dir: &Dir, days: u64, plan: &[(usize, usize)]) -> Vec<Item> {
    build_flagged(dir, days, plan, |_| Flags::NONE)
}

/// As [`build_items`], the event at position `index` carrying `flags(index)`.
fn build_flagged(
    dir: &Dir,
    days: u64,
    plan: &[(usize, usize)],
    flags: impl Fn(usize) -> Flags,
) -> Vec<Item> {
    let count = u64::try_from(plan.len()).unwrap().max(1);
    let config = SegmentsConfig {
        rotation: Rotation {
            dir: dir.path().to_path_buf(),
            day_of: day_of(),
            size_cap: u64::MAX,
        },
        caps: Caps::default(),
        retire: Arc::new(NoViewer),
    };
    let (mut writer, _) = Writer::open_segments(config, 1).unwrap();
    writer
        .register_root(&RootInfo {
            idx: RootIdx(0),
            vol: VolInfo {
                guid: String::from("vol-1"),
                serial: 1,
                letter: Some(String::from("C:")),
                journal_id: None,
            },
            path: units(ROOT),
            recursive: true,
        })
        .unwrap();
    let mut model = Vec::new();
    for (index, &(name, kind)) in plan.iter().enumerate() {
        let index = u64::try_from(index).unwrap();
        let ts = 1 + index * (days * DAY / count) + index % 7;
        let rel = NAMES[name % NAMES.len()];
        let kind = KINDS[kind % KINDS.len()];
        let mut batch = Batch::with_capacity(2, 1024);
        let mut flagged = event(ts, kind);
        flagged.flags = flags(usize::try_from(index).unwrap());
        batch.push(flagged, &units(rel)).unwrap();
        writer.write(&[batch], &[]).unwrap();
        model.push(Item {
            ts,
            kind,
            rel: rel.to_owned(),
        });
    }
    writer.wait_background();
    writer.flush_system_rows().unwrap();
    // Rotation and sealing queued system rows: they take ids too, so the
    // model ids are looked up from the files rather than assumed.
    model
}

/// Every page in the direction, until `more` is false.
fn all_pages(reader: &Reader, filter: &Filter, limit: usize, direction: Direction) -> Vec<Page> {
    let cancel = CancelFlag::new();
    let mut pages = Vec::new();
    let mut cursor = None;
    loop {
        let request = PageRequest {
            generation: 1,
            filter: filter.clone(),
            cursor,
            direction,
            limit,
        };
        let page = reader.page(&request, &cancel).unwrap();
        cursor = match direction {
            Direction::Older => page.oldest_id(),
            Direction::Newer => page.newest_id(),
        };
        let more = page.more;
        pages.push(page);
        if !more || cursor.is_none() {
            return pages;
        }
    }
}

fn segment_files(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| {
            name.starts_with("events-")
                && Path::new(name)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("db"))
        })
        .collect()
}

fn text(units: &[u16]) -> String {
    String::from_utf16_lossy(units)
}

/// The model rows that are real events (rotation rows have another source).
fn real(reader: &Reader) -> Vec<(i64, String)> {
    let pages = all_pages(reader, &Filter::default(), 1000, Direction::Older);
    pages
        .iter()
        .flat_map(|page| page.rows.iter())
        .filter(|row| row.source == Source::Rdcw)
        .map(|row| (row.id, text(&row.path)))
        .collect()
}

#[test]
fn paging_covers_every_row_once_in_both_directions() {
    let dir = Dir::new("paging");
    build(&dir, 3, 60);
    let reader = Reader::new(dir.path());
    let everything = all_pages(&reader, &Filter::default(), 1000, Direction::Older);
    let expected: Vec<i64> = everything[0].rows.iter().map(|row| row.id).collect();
    assert!(expected.len() >= 60, "{}", expected.len());
    assert!(expected.windows(2).all(|pair| pair[0] > pair[1]));

    for size in [1, 7, PAGE_SIZE] {
        for direction in [Direction::Older, Direction::Newer] {
            let pages = all_pages(&reader, &Filter::default(), size, direction);
            let mut ids: Vec<i64> = Vec::new();
            for page in &pages {
                assert!(page.rows.len() <= size);
                assert!(
                    page.rows.windows(2).all(|pair| pair[0].id > pair[1].id),
                    "a page is newest first"
                );
                ids.extend(page.rows.iter().map(|row| row.id));
            }
            ids.sort_by(|a, b| b.cmp(a));
            assert_eq!(ids, expected, "size {size} {direction:?}");
        }
    }
}

#[test]
fn cloud_churn_is_hidden_in_the_default_view_and_counted() {
    let dir = Dir::new("hide");
    // 24 events over two days; every third one is cloud churn.
    let plan: Vec<(usize, usize)> = (0..24).map(|i| (i % NAMES.len(), i % 4)).collect();
    build_flagged(&dir, 2, &plan, |index| {
        if index % 3 == 0 {
            Flags::CLOUD_CHURN
        } else {
            Flags::NONE
        }
    });
    let reader = Reader::new(dir.path());
    let churn = |row: &peekbuddy_core::query::Row| row.flags & Flags::CLOUD_CHURN.bits() != 0;
    let rows_of = |filter: &Filter| -> Vec<peekbuddy_core::query::Row> {
        all_pages(&reader, filter, 7, Direction::Older)
            .into_iter()
            .flat_map(|page| page.rows)
            .collect()
    };
    let everything = rows_of(&Filter::default());
    assert_eq!(everything.iter().filter(|row| churn(row)).count(), 8);
    // The default view shows no churn row and every other row.
    let view = rows_of(&Filter::default_view());
    assert!(view.iter().all(|row| !churn(row)));
    assert_eq!(view.len(), everything.len() - 8);
    // The hidden counter is what the view lacks; an export-style read of the
    // whole log is not affected.
    let cancel = CancelFlag::default();
    assert_eq!(
        reader.hidden(&Filter::default_view(), &cancel).unwrap(),
        Some(8)
    );
    assert_eq!(reader.hidden(&Filter::default(), &cancel).unwrap(), Some(0));
    // The count follows the rest of the filter.
    let by_kind = Filter {
        kinds: Some(KindSet::of(&[Kind::Created])),
        ..Filter::default_view()
    };
    let created_hidden = u64::try_from(
        everything
            .iter()
            .filter(|row| churn(row) && row.kind == Kind::Created)
            .count(),
    )
    .unwrap();
    assert!(created_hidden > 0 && created_hidden < 8);
    assert_eq!(
        reader.hidden(&by_kind, &cancel).unwrap(),
        Some(created_hidden)
    );
    // A cancelled count says so.
    let stop = CancelFlag::default();
    stop.cancel();
    assert_eq!(reader.hidden(&Filter::default_view(), &stop).unwrap(), None);
}

#[test]
fn a_row_carries_its_path_and_name() {
    let dir = Dir::new("rows");
    build(&dir, 1, 8);
    let reader = Reader::new(dir.path());
    let page = &all_pages(&reader, &Filter::default(), 50, Direction::Older)[0];
    let row = page
        .rows
        .iter()
        .find(|row| text(&row.path).ends_with(r"\main.rs"))
        .unwrap();
    assert_eq!(text(&row.path), r"C:\Watched\proj\src\main.rs");
    assert_eq!(text(&row.name), "main.rs");
    assert_eq!(row.kind, Kind::Created);
    assert_eq!(row.source, Source::Rdcw);
}

fn expected_ids(reader: &Reader, model: &[Item], keep: impl Fn(&Item) -> bool) -> Vec<i64> {
    // Real events are the ones whose path is in the model, in order.
    let mut real = real(reader);
    real.sort_by_key(|(id, _)| *id);
    assert_eq!(real.len(), model.len());
    let mut ids: Vec<i64> = real
        .iter()
        .zip(model)
        .filter(|(_, item)| keep(item))
        .map(|((id, path), item)| {
            assert_eq!(*path, item.path());
            *id
        })
        .collect();
    ids.sort_by(|a, b| b.cmp(a));
    ids
}

fn ids_of(reader: &Reader, filter: &Filter) -> Vec<i64> {
    let mut ids: Vec<i64> = all_pages(reader, filter, 5, Direction::Older)
        .iter()
        .flat_map(|page| page.rows.iter().map(|row| row.id))
        .collect();
    ids.sort_by(|a, b| b.cmp(a));
    ids
}

type Keep = Box<dyn Fn(&Item) -> bool>;
type Case = (&'static str, Filter, Keep);

/// Filters with the model predicate each must agree with.
#[expect(clippy::too_many_lines, reason = "a table of cases")]
fn cases() -> Vec<Case> {
    let low = DAY + 10 * HOUR;
    let high = 2 * DAY;
    vec![
        (
            "kinds",
            Filter {
                kinds: Some(KindSet::of(&[Kind::Created, Kind::Deleted])),
                ..Filter::default()
            },
            Box::new(|item| matches!(item.kind, Kind::Created | Kind::Deleted)),
        ),
        (
            "root",
            Filter {
                root: Some(units(ROOT)),
                ..Filter::default()
            },
            Box::new(|_| true),
        ),
        (
            "an unknown root",
            Filter {
                root: Some(units(r"\\?\D:\Elsewhere")),
                ..Filter::default()
            },
            Box::new(|_| false),
        ),
        (
            "time",
            Filter {
                time: TimeRange {
                    from: Some(low),
                    to: Some(high),
                },
                ..Filter::default()
            },
            Box::new(move |item| item.ts >= low && item.ts <= high),
        ),
        (
            "an open time range",
            Filter {
                time: TimeRange {
                    from: Some(high),
                    to: None,
                },
                ..Filter::default()
            },
            Box::new(move |item| item.ts >= high),
        ),
        (
            "subtree",
            Filter {
                subtree: Some(units(r"C:\Watched\proj\src")),
                ..Filter::default()
            },
            Box::new(|item| item.rel.starts_with(r"proj\src\")),
        ),
        (
            "name of three characters",
            Filter {
                name: Some(String::from("read")),
                ..Filter::default()
            },
            Box::new(|item| item.name().to_lowercase().contains("read")),
        ),
        (
            "a short name",
            Filter {
                name: Some(String::from("rs")),
                ..Filter::default()
            },
            Box::new(|item| item.name().to_lowercase().contains("rs")),
        ),
        (
            "a name with a wildcard character",
            Filter {
                name: Some(String::from("a_b")),
                ..Filter::default()
            },
            Box::new(|item| item.name().to_lowercase().contains("a_b")),
        ),
        (
            "a short name with a percent sign",
            Filter {
                name: Some(String::from("%c")),
                ..Filter::default()
            },
            Box::new(|item| item.name().contains("%c")),
        ),
        (
            "the name, not the path",
            Filter {
                name: Some(String::from("proj")),
                ..Filter::default()
            },
            Box::new(|_| false),
        ),
        (
            "combined",
            Filter {
                kinds: Some(KindSet::of(&[Kind::Modified])),
                subtree: Some(units(r"C:\Watched\proj")),
                name: Some(String::from(".rs")),
                time: TimeRange {
                    from: Some(DAY),
                    to: None,
                },
                ..Filter::default()
            },
            Box::new(|item| {
                item.kind == Kind::Modified
                    && item.rel.starts_with(r"proj\")
                    && item.name().contains(".rs")
                    && item.ts >= DAY
            }),
        ),
    ]
}

#[test]
fn every_filter_matches_the_naive_model() {
    let dir = Dir::new("filters");
    let model = build(&dir, 3, 90);
    let reader = Reader::new(dir.path());
    for (label, filter, keep) in cases() {
        let wanted: Vec<i64> = expected_ids(&reader, &model, &keep).into_iter().collect();
        // Rows of other sources (rotation, retention) only pass an empty
        // filter, a kind filter that names them, or a time filter.
        let got: Vec<i64> = {
            let rows: Vec<_> = all_pages(&reader, &filter, 5, Direction::Older)
                .into_iter()
                .flat_map(|page| page.rows)
                .filter(|row| row.source == Source::Rdcw)
                .map(|row| row.id)
                .collect();
            let mut rows = rows;
            rows.sort_by(|a, b| b.cmp(a));
            rows
        };
        assert_eq!(got, wanted, "{label}");
    }
}

#[test]
fn a_glob_filters_the_reconstructed_path() {
    let dir = Dir::new("glob");
    let model = build(&dir, 2, 40);
    let reader = Reader::new(dir.path());
    let set = FilterSet::compile(&[Rule::include(Pattern::Glob(String::from("*.rs")))]).unwrap();
    let filter = Filter {
        pattern: Some(Arc::new(set)),
        ..Filter::default()
    };
    let wanted = expected_ids(&reader, &model, |item| {
        item.rel.rsplit('.').next() == Some("rs")
    });
    assert_eq!(ids_of(&reader, &filter), wanted);
    let page = &all_pages(&reader, &filter, 1000, Direction::Older)[0];
    assert!(page.scanned > page.rows.len() as u64, "a post-filter scans");
}

#[test]
fn a_cancelled_request_stops_with_a_flag() {
    let dir = Dir::new("cancel");
    build(&dir, 1, 20);
    let reader = Reader::new(dir.path());
    let cancel = CancelFlag::new();
    cancel.cancel();
    let page = reader
        .page(&PageRequest::newest(9, Filter::default()), &cancel)
        .unwrap();
    assert!(page.cancelled);
    assert_eq!(page.generation, 9);
    assert_eq!(page.rows, []);
}

#[test]
fn a_retired_segment_is_reported_and_skipped() {
    let dir = Dir::new("retired");
    build(&dir, 3, 30);
    let reader = Reader::new(dir.path());
    let before = ids_of(&reader, &Filter::default());

    let oldest = segment_files(dir.path()).into_iter().min().unwrap();
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(dir.path().join(format!("{oldest}{suffix}")));
    }
    let pages = all_pages(&reader, &Filter::default(), 1000, Direction::Older);
    let retired: Vec<&String> = pages.iter().flat_map(|page| &page.retired).collect();
    assert_eq!(retired, [&oldest]);
    let after = ids_of(&reader, &Filter::default());
    assert!(!after.is_empty() && after.len() < before.len());
    assert_eq!(after, before[..after.len()]);
}

/// A store with one event per name (as UTF-16 units), in one segment.
fn named(dir: &Dir, names: &[Vec<u16>]) {
    let config = SegmentsConfig {
        rotation: Rotation {
            dir: dir.path().to_path_buf(),
            day_of: day_of(),
            size_cap: u64::MAX,
        },
        caps: Caps::default(),
        retire: Arc::new(NoViewer),
    };
    let (mut writer, _) = Writer::open_segments(config, 1).unwrap();
    writer
        .register_root(&RootInfo {
            idx: RootIdx(0),
            vol: VolInfo {
                guid: String::from("vol-1"),
                serial: 1,
                letter: Some(String::from("C:")),
                journal_id: None,
            },
            path: units(ROOT),
            recursive: true,
        })
        .unwrap();
    let mut batch = Batch::with_capacity(names.len() + 1, 1 << 20);
    for (index, name) in names.iter().enumerate() {
        batch
            .push(event(1 + index as u64, Kind::Created), name)
            .unwrap();
    }
    writer.write(&[batch], &[]).unwrap();
    writer.wait_background();
}

fn found(reader: &Reader, needle: &str) -> Vec<String> {
    let filter = Filter {
        name: Some(needle.to_owned()),
        ..Filter::default()
    };
    let mut names: Vec<String> = all_pages(reader, &filter, 50, Direction::Older)
        .into_iter()
        .flat_map(|page| page.rows)
        .filter(|row| row.source == Source::Rdcw)
        .map(|row| String::from_utf16_lossy(&row.name))
        .collect();
    names.sort();
    names
}

#[test]
fn name_searches_have_the_documented_results() {
    let dir = Dir::new("names");
    let mut lone = units("bad");
    lone.push(0xD800);
    lone.extend(units("name.txt"));
    named(
        &dir,
        &[
            units("Cr\u{e8}me.txt"),
            units("creme.txt"),
            units("\u{c4}PFEL.md"),
            units("\u{65e5}\u{672c}\u{8a9e}.txt"),
            lone.clone(),
        ],
    );
    let reader = Reader::new(dir.path());
    // Three characters or more: the trigram index folds case and diacritics.
    assert_eq!(found(&reader, "CREME"), ["Cr\u{e8}me.txt", "creme.txt"]);
    assert_eq!(found(&reader, "\u{e4}pf"), ["\u{c4}PFEL.md"]);
    // Fewer than three: LIKE, which folds ASCII case only.
    assert_eq!(
        found(&reader, "\u{65e5}\u{672c}"),
        ["\u{65e5}\u{672c}\u{8a9e}.txt"]
    );
    assert_eq!(found(&reader, "md"), ["\u{c4}PFEL.md"]);
    assert_eq!(found(&reader, "\u{e4}p"), Vec::<String>::new());
    // A lone surrogate is stored as U+FFFD for search; the row keeps the
    // exact units for display (F17).
    assert_eq!(found(&reader, "d\u{fffd}n").len(), 1);
    let page = &all_pages(
        &reader,
        &Filter {
            name: Some(String::from("name.txt")),
            ..Filter::default()
        },
        10,
        Direction::Older,
    )[0];
    assert_eq!(page.rows.first().map(|row| row.name.clone()), Some(lone));
}

/// Calls a request again with the resume cursor until the scan is through;
/// returns every row found and the number of calls.
fn resumed(reader: &Reader, filter: &Filter) -> (Vec<String>, usize) {
    let cancel = CancelFlag::new();
    let (mut found, mut calls, mut cursor) = (Vec::new(), 0, None);
    loop {
        let page = reader
            .page(
                &PageRequest {
                    cursor,
                    ..PageRequest::newest(1, filter.clone())
                },
                &cancel,
            )
            .unwrap();
        calls += 1;
        found.extend(page.rows.iter().map(|row| text(&row.path)));
        assert!(calls < 10_000, "the scan never ends");
        match page.resume {
            Some(at) => cursor = Some(at),
            None => return (found, calls),
        }
    }
}

#[test]
fn a_rare_pattern_stops_at_the_time_budget_and_resumes() {
    let dir = Dir::new("scanstep");
    let total = 45_000_u64;
    let names: Vec<Vec<u16>> = (0..total)
        .map(|index| {
            if index == 3 {
                units("needle.txt")
            } else {
                units(&format!("f{index}.dat"))
            }
        })
        .collect();
    named(&dir, &names);
    let set = FilterSet::compile(&[Rule::include(Pattern::Glob(String::from("needle*")))]).unwrap();
    let filter = Filter {
        pattern: Some(Arc::new(set)),
        ..Filter::default()
    };
    // No time at all: every call scans one chunk and hands back a cursor.
    let reader = Reader::new(dir.path()).with_scan_budget(Duration::ZERO);
    let first = reader
        .page(&PageRequest::newest(1, filter.clone()), &CancelFlag::new())
        .unwrap();
    assert_eq!(first.rows, []);
    assert!(first.more);
    assert!(first.resume.is_some());
    assert_eq!(first.scanned, 1_000, "one chunk, not a row count of 20,000");
    let (found, calls) = resumed(&reader, &filter);
    assert_eq!(found.len(), 1);
    assert!(calls >= 45, "{calls} calls for 45,000 rows");
    // With the real budget the same scan needs fewer, longer calls.
    let (found, fewer) = resumed(&Reader::new(dir.path()), &filter);
    assert_eq!(found.len(), 1);
    assert!(fewer <= calls, "{fewer} > {calls}");
}

#[test]
fn a_broad_filter_on_old_rows_is_scanned_in_windows_and_resumes() {
    let dir = Dir::new("windows");
    // The first 25,000 rows (the oldest) are in `needle`, the next 25,000
    // (the newest) are not: a newest-first page has to walk past all of them,
    // and more than SELECTIVE_EVENTS rows match, so the ids are walked.
    let names: Vec<Vec<u16>> = (0..50_000_u64)
        .map(|index| {
            if index < 25_000 {
                units(&format!(r"needle\needle-f{index}.dat"))
            } else {
                units(&format!(r"common\f{index}.dat"))
            }
        })
        .collect();
    named(&dir, &names);
    let by_name = Filter {
        name: Some(String::from("needle")),
        ..Filter::default()
    };
    let by_subtree = Filter {
        subtree: Some(units(r"C:\Watched\needle")),
        ..Filter::default()
    };
    let by_kind = Filter {
        kinds: Some(KindSet::of(&[Kind::Deleted])),
        ..Filter::default()
    };
    // No time at all: one window per call.
    let tight = Reader::new(dir.path()).with_scan_budget(Duration::ZERO);
    let ordinary = Reader::new(dir.path());
    for (label, filter) in [("name", &by_name), ("subtree", &by_subtree)] {
        let first = tight
            .page(&PageRequest::newest(1, filter.clone()), &CancelFlag::new())
            .unwrap();
        assert!(
            first.resume.is_some(),
            "{label}: one window is not the end: scanned {}, more {}, rows {}",
            first.scanned,
            first.more,
            first.rows.len()
        );
        assert!(first.more, "{label}");
        assert!(
            first.scanned > 0,
            "{label}: the window counts what it scanned"
        );
        assert_eq!(
            first.rows,
            [],
            "{label}: the newest rows are not in the filter"
        );
        let (found, calls) = resumed(&tight, filter);
        assert_eq!((found.len(), calls > 1), (PAGE_SIZE, true), "{label}");
        let (found, _) = resumed(&ordinary, filter);
        assert_eq!(found.len(), PAGE_SIZE, "{label}");
    }
    // A filter that matches nothing ends, and ends empty.
    let (found, _) = resumed(&tight, &by_kind);
    assert_eq!(found, Vec::<String>::new());
}

#[test]
fn a_filter_with_few_events_is_answered_through_the_index_in_one_call() {
    let dir = Dir::new("selective");
    let names: Vec<Vec<u16>> = (0..50_000_u64)
        .map(|index| {
            if index == 5 {
                units(r"rare\needle-item.txt")
            } else {
                units(&format!(r"common\f{index}.dat"))
            }
        })
        .collect();
    named(&dir, &names);
    // Even with no time at all: no window to walk, so no resume.
    let tight = Reader::new(dir.path()).with_scan_budget(Duration::ZERO);
    for filter in [
        Filter {
            name: Some(String::from("needle-item")),
            ..Filter::default()
        },
        Filter {
            subtree: Some(units(r"C:\Watched\rare")),
            ..Filter::default()
        },
    ] {
        let page = tight
            .page(&PageRequest::newest(1, filter), &CancelFlag::new())
            .unwrap();
        assert_eq!(page.rows.len(), 1);
        assert_eq!(page.resume, None);
        assert!(!page.more);
    }
}

#[test]
fn a_window_that_finds_a_whole_page_does_not_scan_the_rest() {
    let dir = Dir::new("firstwindow");
    let names: Vec<Vec<u16>> = (0..30_000_u64)
        .map(|index| units(&format!(r"same\f{index}.dat")))
        .collect();
    named(&dir, &names);
    let filter = Filter {
        name: Some(String::from("f2")),
        ..Filter::default()
    };
    let page = Reader::new(dir.path())
        .page(&PageRequest::newest(1, filter), &CancelFlag::new())
        .unwrap();
    assert_eq!(page.rows.len(), PAGE_SIZE);
    assert!(page.more);
    // Nowhere near the whole store was looked at to fill a page.
    assert!(page.scanned < 30_000, "{}", page.scanned);
}

#[test]
fn the_query_plans_use_an_index() {
    let dir = Dir::new("plans");
    build(&dir, 1, 24);
    let newest = segment_files(dir.path()).into_iter().max().unwrap();
    let conn =
        Connection::open_with_flags(dir.path().join(newest), OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let shapes: Vec<(&str, Filter)> = vec![
        ("none", Filter::default()),
        (
            "kinds",
            Filter {
                kinds: Some(KindSet::of(&[Kind::Created])),
                ..Filter::default()
            },
        ),
        (
            "root",
            Filter {
                root: Some(units(ROOT)),
                ..Filter::default()
            },
        ),
        (
            "time",
            Filter {
                time: TimeRange {
                    from: Some(5),
                    to: Some(10 * HOUR),
                },
                ..Filter::default()
            },
        ),
        (
            "subtree",
            Filter {
                subtree: Some(units(r"C:\Watched\proj")),
                ..Filter::default()
            },
        ),
        (
            "name",
            Filter {
                name: Some(String::from("main")),
                ..Filter::default()
            },
        ),
        (
            "short name",
            Filter {
                name: Some(String::from("rs")),
                ..Filter::default()
            },
        ),
    ];
    let mut out = String::new();
    for (label, filter) in &shapes {
        for direction in [Direction::Older, Direction::Newer] {
            let plan = explain(&conn, filter, direction).unwrap();
            assert!(!plan.is_empty(), "{label}");
            assert!(
                !plan.iter().any(|line| line.starts_with("SCAN e")),
                "{label} scans the event table: {plan:?}"
            );
            // No page ever sorts all its matches before the first row: the
            // subtree and name filters walk the ids in order (#267).
            assert!(
                !plan.iter().any(|line| line.contains("TEMP B-TREE")),
                "{label} sorts in a temp b-tree: {plan:?}"
            );
            writeln!(out, "{label} {direction:?}").unwrap();
            for line in plan {
                writeln!(out, "  {line}").unwrap();
            }
        }
    }
    // A filter on nodes with few events takes the other statement: the index
    // finds them and a sort orders them. It is chosen per segment by a capped
    // count (`SELECTIVE_EVENTS`), so the sort never has much to sort.
    for (label, filter) in shapes
        .iter()
        .filter(|(label, _)| matches!(*label, "subtree" | "name" | "short name"))
    {
        for direction in [Direction::Older, Direction::Newer] {
            let plan = explain_selective(&conn, filter, direction).unwrap();
            assert!(
                plan.iter()
                    .any(|line| line.contains("USING INDEX event_node")),
                "{label} selective does not use event_node: {plan:?}"
            );
            writeln!(out, "{label} selective {direction:?}").unwrap();
            for line in plan {
                writeln!(out, "  {line}").unwrap();
            }
        }
    }
    insta::assert_snapshot!("page_query_plans", out);
}

/// Cases for the heavy property tests; see `heavy_cases` in `pipeline.rs`.
fn heavy_cases() -> u32 {
    std::env::var("PEEKBUDDY_HEAVY_CASES")
        .ok()
        .and_then(|text| text.parse().ok())
        .unwrap_or(6)
}

/// A random filter over the kinds the generated events use (so that the rows
/// of the engine itself never pass) and a name needle.
fn arb_filter() -> impl Strategy<Value = (Vec<usize>, Option<usize>, Option<usize>)> {
    (
        proptest::collection::vec(0_usize..KINDS.len(), 1..=KINDS.len()),
        proptest::option::of(0_usize..NEEDLES.len()),
        proptest::option::of(0_usize..SUBTREES.len()),
    )
}

const NEEDLES: [&str; 5] = ["main", "rs", "read", "txt", "zzz"];
const SUBTREES: [&str; 3] = [r"C:\Watched\proj", r"C:\Watched\tmp", r"C:\Watched\other"];

#[test]
fn paging_from_any_cursor_equals_a_naive_model() {
    let mut runner = TestRunner::new(ProptestConfig {
        cases: heavy_cases(),
        ..ProptestConfig::default()
    });
    let strategy = (
        proptest::collection::vec((0_usize..NAMES.len(), 0_usize..KINDS.len()), 1..60),
        1_u64..4,
        arb_filter(),
        0_usize..100,
        any::<bool>(),
        1_usize..9,
    );
    runner
        .run(
            &strategy,
            |(plan, days, (kind_picks, needle, subtree), at, older, limit)| {
                let dir = Dir::new(&format!("prop{}", next_tag()));
                let model = build_items(&dir, days, &plan);
                let reader = Reader::new(dir.path());

                // The ids of the model rows, in order.
                let mut stored = real(&reader);
                stored.sort_by_key(|(id, _)| *id);
                prop_assert_eq!(stored.len(), model.len());

                let kinds: Vec<Kind> = kind_picks.iter().map(|k| KINDS[*k]).collect();
                let keep = |item: &Item| {
                    kinds.contains(&item.kind)
                        && needle.is_none_or(|n| item.name().to_lowercase().contains(NEEDLES[n]))
                        && subtree.is_none_or(|s| {
                            let prefix = SUBTREES[s].trim_start_matches(r"C:\Watched\");
                            item.rel.starts_with(&format!("{prefix}\\"))
                        })
                };
                let wanted: Vec<i64> = stored
                    .iter()
                    .zip(&model)
                    .filter(|(_, item)| keep(item))
                    .map(|((id, _), _)| *id)
                    .collect();

                let filter = Filter {
                    kinds: Some(KindSet::of(&kinds)),
                    name: needle.map(|n| NEEDLES[n].to_owned()),
                    subtree: subtree.map(|s| units(SUBTREES[s])),
                    ..Filter::default()
                };
                // A cursor: none, or one of the stored ids (matching or not).
                let cursor = (at % (stored.len() + 1))
                    .checked_sub(1)
                    .map(|i| stored[i].0);
                let direction = if older {
                    Direction::Older
                } else {
                    Direction::Newer
                };
                let page = reader
                    .page(
                        &PageRequest {
                            generation: 5,
                            filter,
                            cursor,
                            direction,
                            limit,
                        },
                        &CancelFlag::new(),
                    )
                    .unwrap();

                let beyond: Vec<i64> = wanted
                    .iter()
                    .copied()
                    .filter(|id| match (direction, cursor) {
                        (_, None) => true,
                        (Direction::Older, Some(c)) => *id < c,
                        (Direction::Newer, Some(c)) => *id > c,
                    })
                    .collect();
                let (shown, more) = match direction {
                    // The newest `limit` rows below the cursor.
                    Direction::Older => {
                        let start = beyond.len().saturating_sub(limit);
                        let mut shown = beyond[start..].to_vec();
                        shown.reverse();
                        (shown, beyond.len() > limit)
                    }
                    // The oldest `limit` rows above it, shown newest first.
                    Direction::Newer => {
                        let mut shown: Vec<i64> = beyond.iter().copied().take(limit).collect();
                        shown.reverse();
                        (shown, beyond.len() > limit)
                    }
                };
                let got: Vec<i64> = page.rows.iter().map(|row| row.id).collect();
                prop_assert_eq!(page.generation, 5);
                prop_assert_eq!(&got, &shown);
                prop_assert_eq!(page.more, more);
                Ok(())
            },
        )
        .unwrap();
}

fn next_tag() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

#[test]
fn a_page_that_holds_exactly_what_is_left_has_nothing_more() {
    let dir = Dir::new("exact");
    build(&dir, 1, 10);
    let reader = Reader::new(dir.path());
    let page = |limit: usize| {
        reader
            .page(
                &PageRequest {
                    limit,
                    ..PageRequest::newest(1, Filter::default())
                },
                &CancelFlag::new(),
            )
            .unwrap()
    };
    let all = page(10);
    assert_eq!((all.rows.len(), all.more), (10, false));
    let some = page(9);
    assert_eq!((some.rows.len(), some.more), (9, true));
    let more = page(11);
    assert_eq!((more.rows.len(), more.more), (10, false));
}
