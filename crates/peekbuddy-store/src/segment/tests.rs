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

#[test]
fn the_default_size_cap_is_one_gibibyte() {
    assert_eq!(DEFAULT_SIZE_CAP, 1_073_741_824);
}

#[test]
fn a_rotation_shows_its_directory_and_cap_in_debug_output() {
    let rotation = Rotation {
        dir: PathBuf::from("some-segment-dir"),
        day_of: Arc::new(|_| day(2026, 1, 1)),
        size_cap: 777,
    };
    let text = format!("{rotation:?}");
    assert!(text.contains("Rotation"), "{text}");
    assert!(text.contains("some-segment-dir"), "{text}");
    assert!(text.contains("size_cap: 777"), "{text}");
}

#[test]
fn reconciling_drops_rows_of_missing_files_and_keeps_what_the_catalog_knows() {
    let dir = std::env::temp_dir().join(format!("pb-reconcile-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for name in ["events-2026-10-01.db", "events-2026-10-02.db"] {
        drop(crate::schema::open(&dir.join(name)).unwrap());
    }
    let catalog = Catalog::open_in_memory().unwrap();
    // The first file was marked for purging, the newest was sealed by an
    // earlier run, and a third row names a file that is gone.
    catalog
        .put(&SegmentRow {
            sealed: true,
            purge_pending: true,
            ..row("events-2026-10-01.db", 0)
        })
        .unwrap();
    catalog
        .put(&SegmentRow {
            sealed: true,
            ..row("events-2026-10-02.db", 0)
        })
        .unwrap();
    catalog.put(&row("events-2026-09-30.db", 0)).unwrap();

    let (rows, skipped) = reconcile_catalog(&dir, &catalog).unwrap();
    assert!(skipped.is_empty(), "{skipped:?}");
    let files: Vec<&str> = rows.iter().map(|r| r.file.as_str()).collect();
    assert_eq!(files, ["events-2026-10-01.db", "events-2026-10-02.db"]);
    let first = rows.first().unwrap();
    let newest = rows.get(1).unwrap();
    assert!(first.purge_pending, "the purge mark is kept");
    assert!(first.sealed);
    // The scan would leave the newest segment open; the catalog's word that
    // it was sealed stays.
    assert!(newest.sealed);
    assert!(!newest.purge_pending);
    std::fs::remove_dir_all(&dir).unwrap();
}
