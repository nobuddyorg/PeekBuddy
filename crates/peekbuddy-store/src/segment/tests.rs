use super::*;

fn day(year: i32, month: u8, day: u8) -> DayKey {
    DayKey { year, month, day }
}

fn row(file: &str, rows: i64) -> SegmentRow {
    SegmentRow {
        file: file.to_owned(),
        first_id: 0,
        last_id: 0,
        first_ts: 0,
        last_ts: 0,
        rows,
        bytes: 0,
        sealed: false,
        schema_version: 1,
        purge_pending: false,
    }
}

#[test]
fn names_round_trip() {
    let d = day(2026, 3, 9);
    assert_eq!(d.file_name(0), "events-2026-03-09.db");
    assert_eq!(d.file_name(2), "events-2026-03-09.2.db");
    assert_eq!(parse_name("events-2026-03-09.db"), Some((d, 0)));
    assert_eq!(parse_name("events-2026-03-09.2.db"), Some((d, 2)));
    assert_eq!(parse_name("events-2026-03-09.x.db"), None);
    assert_eq!(parse_name("catalog.db"), None);
}

#[test]
fn rotation_happens_for_a_later_day_or_a_full_file_only() {
    let today = day(2026, 10, 26);
    let cap = 1000;
    assert!(!needs_rotation(today, today, 10, cap));
    assert!(needs_rotation(today, day(2026, 10, 27), 10, cap));
    assert!(needs_rotation(today, day(2027, 1, 1), 10, cap));
    // The clock moved back: stay where the ids are.
    assert!(!needs_rotation(today, day(2026, 10, 25), 10, cap));
    // Over the cap, same day.
    assert!(needs_rotation(today, today, cap, cap));
    assert!(needs_rotation(today, day(2026, 10, 25), cap + 1, cap));
}

#[test]
fn the_next_file_never_reuses_a_name() {
    let d1 = day(2026, 10, 26);
    let d2 = day(2026, 10, 27);
    let none: Vec<SegmentRow> = Vec::new();
    // A new day starts at number 0.
    assert_eq!(
        next_file("events-2026-10-26.db", d2, &none),
        "events-2026-10-27.db"
    );
    // The size cap within a day: .1, then .2.
    let one = vec![row("events-2026-10-26.db", 5)];
    assert_eq!(
        next_file("events-2026-10-26.db", d1, &one),
        "events-2026-10-26.1.db"
    );
    let two = vec![
        row("events-2026-10-26.db", 5),
        row("events-2026-10-26.1.db", 5),
    ];
    assert_eq!(
        next_file("events-2026-10-26.1.db", d1, &two),
        "events-2026-10-26.2.db"
    );
    // The zone changed and the new day's file exists: take the next number.
    let back = vec![
        row("events-2026-10-27.db", 5),
        row("events-2026-10-26.db", 5),
    ];
    assert_eq!(
        next_file("events-2026-10-26.db", d2, &back),
        "events-2026-10-27.1.db"
    );
}

#[test]
fn a_scan_becomes_catalog_rows_with_every_but_the_newest_sealed() {
    let scan = Scan {
        segments: vec![
            SegmentRow {
                first_id: 1,
                last_id: 10,
                rows: 10,
                ..row("events-2026-10-24.db", 10)
            },
            row("events-2026-10-25.db", 0),
            SegmentRow {
                first_id: 11,
                last_id: 20,
                rows: 10,
                ..row("events-2026-10-26.db", 10)
            },
            row("events-2026-10-27.db", 0),
        ],
        skipped: Vec::new(),
    };
    let rows = rows_from_scan(&scan, 1);
    let ranges: Vec<(i64, i64, bool)> = rows
        .iter()
        .map(|r| (r.first_id, r.last_id, r.sealed))
        .collect();
    // An empty segment continues the ids of the one before it.
    assert_eq!(
        ranges,
        [
            (1, 10, true),
            (11, 10, true),
            (11, 20, true),
            (21, 20, false)
        ]
    );
}
