use super::*;

fn row(file: &str, first: i64, last: i64, sealed: bool) -> SegmentRow {
    SegmentRow {
        file: file.to_owned(),
        first_id: first,
        last_id: last,
        first_ts: first * 10,
        last_ts: last * 10,
        rows: last - first + 1,
        bytes: 1000,
        sealed,
        schema_version: 1,
        purge_pending: false,
    }
}

#[test]
fn segment_names_are_recognised_exactly() {
    for good in [
        "events-2026-10-26.db",
        "events-2026-10-26.1.db",
        "events-2026-10-26.12.db",
    ] {
        assert!(is_segment_name(good), "{good}");
    }
    for bad in [
        "events-2026-10-26.db-wal",
        "events-2026-10-26.db-shm",
        "events-2026-1-26.db",
        "events-2026-10-26..db",
        "events-2026-10-26.x.db",
        "event-2026-10-26.db",
        "20261026.db",
        "catalog.db",
        "events-2026-10-26.1.2.db",
        "",
    ] {
        assert!(!is_segment_name(bad), "{bad}");
    }
}

#[test]
fn the_catalog_keeps_ranges_oldest_first_and_continues_the_ids() {
    let catalog = Catalog::open_in_memory().unwrap();
    assert_eq!(catalog.next_id().unwrap(), 1);
    catalog
        .put(&row("events-2026-10-02.db", 101, 150, false))
        .unwrap();
    catalog
        .put(&row("events-2026-10-01.db", 1, 100, true))
        .unwrap();
    let all = catalog.list().unwrap();
    assert_eq!(
        all.iter().map(|r| r.file.as_str()).collect::<Vec<_>>(),
        ["events-2026-10-01.db", "events-2026-10-02.db"]
    );
    assert_eq!(catalog.next_id().unwrap(), 151);
    assert_eq!(
        catalog
            .newest_first()
            .unwrap()
            .first()
            .map(|r| r.file.as_str()),
        Some("events-2026-10-02.db")
    );
    catalog.seal("events-2026-10-02.db", 4096).unwrap();
    let sealed = catalog.get("events-2026-10-02.db").unwrap().unwrap();
    assert!(sealed.sealed);
    assert_eq!(sealed.bytes, 4096);
    catalog.defer_purge("events-2026-10-01.db").unwrap();
    assert!(
        catalog
            .get("events-2026-10-01.db")
            .unwrap()
            .unwrap()
            .purge_pending
    );
    catalog.remove("events-2026-10-01.db").unwrap();
    assert_eq!(catalog.list().unwrap().len(), 1);
    assert!(catalog.get("nope.db").unwrap().is_none());
}

#[test]
fn a_catalog_from_a_newer_build_is_refused() {
    let dir = std::env::temp_dir().join(format!("pb-catalog-new-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("catalog.db");
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA user_version = 99;").unwrap();
    }
    assert!(matches!(
        Catalog::open(&path),
        Err(CatalogError::TooNew(99))
    ));
    std::fs::remove_dir_all(&dir).unwrap();
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pb-scan-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_scan_describes_good_files_and_reports_every_bad_one() {
    let dir = scratch("mixed");
    // A good segment with three events.
    let good = dir.join("events-2026-10-01.db");
    let conn = crate::schema::open(&good).unwrap();
    for id in 1..=3_i64 {
        conn.execute(
            "INSERT INTO event (id, ts, kind, source, raw_mask, node_id, is_dir, vol_id, root_id, flags) \
             VALUES (?1, ?2, 0, 0, 1, 0, 0, 0, 0, 0)",
            params![id, id * 100],
        )
        .unwrap();
    }
    drop(conn);
    // A foreign database, a truncated file, a file with a segment name that
    // is garbage, and files that are not segment names at all.
    let foreign = dir.join("events-2026-10-02.db");
    let conn = Connection::open(&foreign).unwrap();
    conn.execute_batch("CREATE TABLE x (a); PRAGMA application_id = 1234;")
        .unwrap();
    drop(conn);
    std::fs::write(
        dir.join("events-2026-10-03.db"),
        b"this is not sqlite at all",
    )
    .unwrap();
    let truncated = std::fs::read(&good).unwrap();
    std::fs::write(
        dir.join("events-2026-10-04.db"),
        &truncated[..truncated.len() / 3],
    )
    .unwrap();
    std::fs::write(dir.join("notes.txt"), b"hello").unwrap();
    std::fs::write(dir.join("events-2026-10-01.db-wal"), b"").unwrap();

    let scan = scan(&dir).unwrap();
    assert_eq!(scan.segments.len(), 1);
    let first = &scan.segments[0];
    assert_eq!(first.file, "events-2026-10-01.db");
    assert_eq!((first.first_id, first.last_id, first.rows), (1, 3, 3));
    assert_eq!((first.first_ts, first.last_ts), (100, 300));
    let skipped: Vec<&str> = scan.skipped.iter().map(|s| s.file.as_str()).collect();
    assert_eq!(
        skipped,
        [
            "events-2026-10-02.db",
            "events-2026-10-03.db",
            "events-2026-10-04.db"
        ]
    );
    assert!(
        scan.skipped[0].reason.contains("foreign application_id"),
        "{:?}",
        scan.skipped
    );
    // The skipped files were not touched.
    assert!(dir.join("events-2026-10-03.db").exists());
    assert_eq!(std::fs::read(dir.join("notes.txt")).unwrap(), b"hello");
    std::fs::remove_dir_all(&dir).unwrap();
}
