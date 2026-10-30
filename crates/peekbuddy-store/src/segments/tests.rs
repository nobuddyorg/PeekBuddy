use super::*;

fn previous() -> SegmentRow {
    SegmentRow {
        file: String::from("events-2026-10-25.db"),
        first_id: 11,
        last_id: 60,
        first_ts: 100,
        last_ts: 200,
        rows: 50,
        bytes: 8192,
        sealed: false,
        schema_version: 1,
        purge_pending: false,
    }
}

#[test]
fn the_rotation_row_names_the_new_file_and_the_range_it_follows() {
    let row = rotated_row(300, &previous(), "events-2026-10-26.db");
    assert_eq!(row.event.kind, Kind::SegmentRotated);
    assert_eq!(row.event.source, Source::System);
    assert_eq!(row.event.root, RootIdx::NONE);
    assert_eq!((row.event.usn, row.event.size), (60, 50));
    assert_eq!(row.event.file_id, 8192);
    assert_eq!(String::from_utf16_lossy(&row.name), "events-2026-10-26.db");
}

#[test]
fn the_purge_row_carries_the_id_range_rows_and_bytes() {
    let row = purged_row(400, &previous());
    assert_eq!(row.event.kind, Kind::RetentionPurged);
    assert_eq!(row.event.usn, 11);
    assert_eq!((row.event.size, row.event.file_id), (50, 8192));
    assert_eq!(String::from_utf16_lossy(&row.name), "events-2026-10-25.db");
}
