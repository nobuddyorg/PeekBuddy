#![expect(
    clippy::indexing_slicing,
    reason = "tests index the fixture they built"
)]

use proptest::prelude::*;

use super::read::{ReadOptions, read_csv, read_jsonl};
use super::*;
use crate::event::{GapReason, Kind, WatchState};

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn row(id: i64, ts: u64, kind: Kind, source: Source, path: &str) -> Row {
    let path = units(path);
    Row {
        id,
        ts,
        kind,
        source,
        raw_mask: 1,
        is_dir: false,
        size: Some(10),
        attrs: Some(0x20),
        file_id: 0xABCD,
        name: path.rsplit(|u| *u == 0x5C).next().unwrap().to_vec(),
        path,
        old_path: Vec::new(),
        root: units(r"C:\Watched"),
        flags: 0,
    }
}

/// 2026-10-30T12:34:56.1234567Z.
const NOON: u64 = 134_377_920_000_000_000 + 45_296 * TICKS_PER_SECOND + 1_234_567;

fn utc(_ts: u64) -> i32 {
    0
}

fn berlin(_ts: u64) -> i32 {
    3600
}

/// A fixture with every kind of case the issue lists.
fn fixture() -> Vec<Row> {
    let mut rows = vec![
        row(1, NOON, Kind::Created, Source::Rdcw, r"C:\Watched\a.txt"),
        row(
            2,
            NOON + 1,
            Kind::Modified,
            Source::Usn,
            r"C:\Watched\dir\b,with comma.txt",
        ),
        row(
            3,
            NOON + 2,
            Kind::Deleted,
            Source::Rdcw,
            "C:\\Watched\\say \"hi\".txt",
        ),
        row(
            4,
            NOON + 3,
            Kind::RenamedTo,
            Source::Rdcw,
            r"C:\Watched\new.txt",
        ),
        row(
            5,
            NOON + 4,
            Kind::ReconciledCreated,
            Source::Reconcile,
            r"C:\Watched\r.txt",
        ),
        row(
            6,
            NOON + 5,
            Kind::Overflow(GapReason::BufferOverflow),
            Source::System,
            "",
        ),
        row(
            7,
            NOON + 6,
            Kind::WatchStateChanged(WatchState::Live),
            Source::System,
            "",
        ),
        row(
            8,
            NOON + 7,
            Kind::Created,
            Source::Rdcw,
            "C:\\Watched\\two\nlines.txt",
        ),
        row(
            9,
            NOON + 8,
            Kind::Created,
            Source::Rdcw,
            "C:\\Watched\\\u{5d0}\u{202e}gnp.exe",
        ),
        row(
            10,
            NOON + 9,
            Kind::Created,
            Source::Rdcw,
            "C:\\Watched\\=cmd|calc.txt",
        ),
    ];
    rows[3].old_path = units(r"C:\Watched\old.txt");
    rows[8].is_dir = true;
    rows[4].size = None;
    rows[4].attrs = None;
    // A lone surrogate in a name.
    let mut lone = units(r"C:\Watched\bad");
    lone.push(0xD800);
    lone.extend(units(".txt"));
    let mut bad = row(11, NOON + 10, Kind::Created, Source::Rdcw, "x");
    bad.name = lone.rsplit(|u| *u == 0x5C).next().unwrap().to_vec();
    bad.path = lone;
    rows.push(bad);
    rows
}

fn export(rows: &[Row], format: Format, options: Options, offset: OffsetFn<'_>) -> String {
    let mut writer = RowWriter::new(Vec::new(), format, options).unwrap();
    for row in rows {
        writer.write_row(row, offset).unwrap();
    }
    String::from_utf8(writer.finish().unwrap()).unwrap()
}

#[test]
fn the_csv_snapshot() {
    let text = export(&fixture(), Format::Csv, Options::default(), &berlin);
    insta::assert_snapshot!("csv", text.replace("\r\n", "<CRLF>\n"));
}

#[test]
fn the_jsonl_snapshot() {
    let text = export(&fixture(), Format::JsonLines, Options::default(), &utc);
    insta::assert_snapshot!("jsonl", text);
}

#[test]
fn the_excel_safe_csv_snapshot() {
    let options = Options {
        bom: true,
        excel_safe: true,
    };
    let text = export(&fixture(), Format::Csv, options, &utc);
    assert!(text.starts_with('\u{FEFF}'));
    insta::assert_snapshot!("csv_excel_safe", text.replace("\r\n", "<CRLF>\n"));
}

#[test]
fn csv_uses_crlf_and_jsonl_uses_lf() {
    let csv = export(&fixture(), Format::Csv, Options::default(), &utc);
    assert!(csv.ends_with("\r\n"));
    assert!(!csv.replace("\r\n", "").contains('\r'));
    let jsonl = export(&fixture(), Format::JsonLines, Options::default(), &utc);
    assert!(!jsonl.contains('\r'));
    assert!(!jsonl.starts_with('\u{FEFF}'));
}

#[test]
fn a_lone_surrogate_never_becomes_a_json_surrogate_escape() {
    let text = export(&fixture(), Format::JsonLines, Options::default(), &utc);
    assert!(!text.to_lowercase().contains("\\ud800"));
    assert!(text.contains(r"bad\\u{D800}.txt") || text.contains("\\\\u{D800}"));
    for line in text.lines() {
        serde_json::from_str::<serde_json::Value>(line).unwrap();
    }
}

#[test]
fn the_fixture_round_trips_in_both_formats() {
    let rows = fixture();
    let csv = export(&rows, Format::Csv, Options::default(), &berlin);
    assert_eq!(read_csv(&csv, ReadOptions::default()).unwrap(), rows);
    let jsonl = export(&rows, Format::JsonLines, Options::default(), &utc);
    assert_eq!(read_jsonl(&jsonl).unwrap(), rows);
}

#[test]
fn excel_safe_csv_reads_back_unchanged_with_the_option() {
    let rows = fixture();
    let options = Options {
        bom: true,
        excel_safe: true,
    };
    let csv = export(&rows, Format::Csv, options, &utc);
    let back = read_csv(&csv, ReadOptions { excel_safe: true }).unwrap();
    assert_eq!(back, rows);
}

#[test]
fn timestamps_have_the_documented_shape() {
    assert_eq!(iso_utc(NOON), "2026-10-30T12:34:56.1234567Z");
    assert_eq!(iso_local(NOON, 3600), "2026-10-30T13:34:56.1234567+01:00");
    assert_eq!(
        iso_local(NOON, -19_800),
        "2026-10-30T07:04:56.1234567-05:30"
    );
    assert_eq!(iso_utc(0), "1601-01-01T00:00:00.0000000Z");
    assert_eq!(parse_iso_utc("2026-10-30T12:34:56.1234567Z"), Some(NOON));
    assert_eq!(parse_iso_utc("2026-10-30 12:34:56.1234567Z"), None);
}

#[test]
fn the_exact_form_round_trips() {
    let samples = [
        vec![],
        units("plain"),
        units(r"a\b"),
        vec![0xD800, 0x61, 0xDC00],
        vec![0x20, 0x7E, 0x7F, 0x80, 0xFFFF],
    ];
    for sample in samples {
        assert_eq!(parse_exact(&exact(&sample)), Some(sample));
    }
    assert_eq!(parse_exact(r"\u{12345}"), None);
    assert_eq!(parse_exact(r"\x"), None);
    assert_eq!(parse_exact(r"\u{}"), None);
}

#[test]
fn reading_garbage_is_an_error_not_a_panic() {
    assert!(read_csv("id,ts_utc\r\n1,2\r\n", ReadOptions::default()).is_err());
    assert!(read_csv("\"unterminated", ReadOptions::default()).is_err());
    assert!(read_jsonl("{\"id\": 1}\n").is_err());
    assert!(read_jsonl("not json\n").is_err());
    assert!(read_jsonl("[1]\n").is_err());
    assert_eq!(read_csv("", ReadOptions::default()).unwrap(), Vec::new());
}

fn arb_units() -> impl Strategy<Value = Vec<u16>> {
    proptest::collection::vec(
        prop_oneof![
            4 => 0x20_u16..0x7F,
            1 => Just(0x5C_u16),
            1 => any::<u16>(),
            1 => 0xD800_u16..0xE000,
        ],
        0..24,
    )
}

fn arb_row() -> impl Strategy<Value = Row> {
    (
        (any::<i64>(), 0_u64..=MAX_TS, 0_usize..26),
        (
            any::<u32>(),
            any::<bool>(),
            proptest::option::of(any::<u64>()),
            proptest::option::of(any::<u32>()),
            any::<u128>(),
        ),
        (
            arb_units(),
            arb_units(),
            arb_units(),
            any::<u16>(),
            0_usize..4,
        ),
    )
        .prop_map(
            |(
                (id, ts, kind),
                (raw_mask, is_dir, size, attrs, file_id),
                (path, old_path, root, flags, source),
            )| {
                let kind = (0..=255)
                    .flat_map(|t| (0..=8).map(move |a| (t, a)))
                    .filter_map(|(t, a)| Kind::from_wire(t, a))
                    .nth(kind)
                    .unwrap_or(Kind::Other);
                Row {
                    id,
                    ts,
                    kind,
                    source: Source::ALL[source],
                    raw_mask,
                    is_dir,
                    size,
                    attrs,
                    file_id,
                    name: path
                        .rsplit(|u| *u == 0x5C)
                        .next()
                        .unwrap_or_default()
                        .to_vec(),
                    path,
                    old_path,
                    root,
                    flags,
                }
            },
        )
}

proptest! {
    #[test]
    fn export_then_import_gives_identical_rows(rows in proptest::collection::vec(arb_row(), 0..12)) {
        let csv = export(&rows, Format::Csv, Options::default(), &berlin);
        prop_assert_eq!(&read_csv(&csv, ReadOptions::default()).unwrap(), &rows);
        let jsonl = export(&rows, Format::JsonLines, Options::default(), &utc);
        prop_assert_eq!(&read_jsonl(&jsonl).unwrap(), &rows);
    }

    #[test]
    fn the_readers_never_panic_on_arbitrary_text(text in ".{0,300}") {
        let _ = read_csv(&text, ReadOptions::default());
        let _ = read_csv(&text, ReadOptions { excel_safe: true });
        let _ = read_jsonl(&text);
    }

    #[test]
    fn timestamps_round_trip(ts in 0_u64..=MAX_TS) {
        prop_assert_eq!(parse_iso_utc(&iso_utc(ts)), Some(ts));
    }
}
