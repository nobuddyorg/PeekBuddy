use std::cell::RefCell;

use super::*;

const DAY: i64 = TICKS_PER_DAY;

fn seg(file: &str, rows: i64, bytes: i64, last_ts: i64, sealed: bool) -> SegmentRow {
    SegmentRow {
        file: file.to_owned(),
        first_id: 1,
        last_id: rows,
        first_ts: last_ts - DAY,
        last_ts,
        rows,
        bytes,
        sealed,
        schema_version: 1,
        purge_pending: false,
    }
}

/// Ten sealed days of 100 MB each and today's open segment.
fn ten_days(now: i64) -> Vec<SegmentRow> {
    let mut all: Vec<SegmentRow> = (0..10_i64)
        .rev()
        .map(|age| {
            seg(
                &format!("events-{:02}.db", 10 - age),
                1000,
                100_000_000,
                now - (age + 1) * DAY,
                true,
            )
        })
        .collect();
    all.push(seg("events-today.db", 10, 5_000_000, now, false));
    all
}

#[test]
fn nothing_goes_without_caps() {
    let now = 100 * DAY;
    assert_eq!(
        plan(&ten_days(now), &Caps::default(), now),
        Vec::<String>::new()
    );
}

#[test]
fn the_age_cap_removes_the_old_ones_oldest_first() {
    let now = 100 * DAY;
    let caps = Caps {
        max_age_days: Some(4),
        ..Caps::default()
    };
    let gone = plan(&ten_days(now), &caps, now);
    // Ages 1..=10 days; those whose newest event is older than 4 days go.
    assert_eq!(gone.len(), 6);
    assert_eq!(gone.first().map(String::as_str), Some("events-01.db"));
}

#[test]
fn the_size_cap_leaves_at_most_the_cap_plus_one_segment() {
    let now = 100 * DAY;
    let segments = ten_days(now);
    for cap_mb in [150_u64, 450, 1_000, 5_000] {
        let caps = Caps {
            max_total_bytes: Some(cap_mb * 1_000_000),
            ..Caps::default()
        };
        let gone = plan(&segments, &caps, now);
        let left: i64 = segments
            .iter()
            .filter(|s| !gone.contains(&s.file))
            .map(|s| s.bytes)
            .sum();
        let cap = i64::try_from(cap_mb * 1_000_000).unwrap();
        // After retention the total is at the cap, or only the open segment
        // is left when it is bigger: at most the cap plus one segment.
        assert!(left <= cap.max(5_000_000), "cap {cap_mb} MB left {left}");
        // And nothing more than needed was removed.
        assert!(left + 100_000_000 > cap.min(1_005_000_000), "{left}");
    }
}

#[test]
fn the_row_cap_works_the_same_way() {
    let now = 100 * DAY;
    let caps = Caps {
        max_rows: Some(3_000),
        ..Caps::default()
    };
    let gone = plan(&ten_days(now), &caps, now);
    assert_eq!(gone.len(), 8, "{gone:?}");
}

#[test]
fn the_newest_and_the_unsealed_are_never_deleted() {
    let now = 100 * DAY;
    let caps = Caps {
        max_age_days: Some(1),
        max_total_bytes: Some(1),
        max_rows: Some(1),
    };
    let mut segments = ten_days(now);
    let gone = plan(&segments, &caps, now);
    assert!(!gone.contains(&String::from("events-today.db")));
    assert_eq!(gone.len(), 10);
    // An unsealed segment in the middle stops the plan there.
    segments[3].sealed = false;
    assert_eq!(plan(&segments, &caps, now).len(), 3);
    // A lone segment is the newest.
    assert_eq!(plan(&segments[..1], &caps, now), Vec::<String>::new());
}

#[test]
fn a_deferred_purge_is_always_a_candidate() {
    let now = 100 * DAY;
    let mut segments = ten_days(now);
    segments[0].purge_pending = true;
    let gone = plan(&segments, &Caps::default(), now);
    assert_eq!(gone, ["events-01.db"]);
}

#[test]
fn an_empty_segment_is_not_old() {
    let now = 100 * DAY;
    let mut segments = ten_days(now);
    segments[0].rows = 0;
    segments[0].last_ts = 0;
    let caps = Caps {
        max_age_days: Some(4),
        ..Caps::default()
    };
    // The oldest is empty (no timestamp to judge), so the plan stops there:
    // oldest-first is the contract.
    assert_eq!(plan(&segments, &caps, now), Vec::<String>::new());
}

struct Recorder {
    seen: RefCell<Vec<String>>,
    ack: bool,
}

impl Retire for Recorder {
    fn announce(&self, file: &str) -> bool {
        self.seen.borrow_mut().push(file.to_owned());
        self.ack
    }
}

fn make_files(dir: &Path, names: &[&str]) {
    std::fs::create_dir_all(dir).unwrap();
    for name in names {
        for suffix in ["", "-wal", "-shm"] {
            std::fs::write(dir.join(format!("{name}{suffix}")), b"data").unwrap();
        }
    }
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("pb-retention-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn catalog_with(names: &[&str]) -> Catalog {
    let catalog = Catalog::open_in_memory().unwrap();
    for (i, name) in names.iter().enumerate() {
        let i = i64::try_from(i).unwrap();
        catalog.put(&seg(name, 10, 100, 1000 + i, true)).unwrap();
    }
    catalog
}

#[test]
fn a_purge_announces_deletes_the_files_and_forgets_the_segment() {
    let dir = scratch("ok");
    make_files(&dir, &["a.db", "b.db"]);
    let catalog = catalog_with(&["a.db", "b.db"]);
    let viewer = Recorder {
        seen: RefCell::new(Vec::new()),
        ack: true,
    };
    let out = purge(&dir, &catalog, &[String::from("a.db")], &viewer).unwrap();
    assert!(matches!(out.as_slice(), [Outcome::Purged(row)] if row.file == "a.db"));
    assert_eq!(*viewer.seen.borrow(), ["a.db"]);
    for suffix in ["", "-wal", "-shm"] {
        assert!(!dir.join(format!("a.db{suffix}")).exists());
        assert!(dir.join(format!("b.db{suffix}")).exists());
    }
    assert!(catalog.get("a.db").unwrap().is_none());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn without_an_ack_the_delete_is_deferred_and_retried_later() {
    let dir = scratch("noack");
    make_files(&dir, &["a.db", "b.db"]);
    let catalog = catalog_with(&["a.db", "b.db"]);
    let deaf = Recorder {
        seen: RefCell::new(Vec::new()),
        ack: false,
    };
    let out = purge(&dir, &catalog, &[String::from("a.db")], &deaf).unwrap();
    assert!(matches!(out.as_slice(), [Outcome::Deferred(row)] if row.file == "a.db"));
    assert!(dir.join("a.db").exists(), "nothing was deleted");
    assert!(catalog.get("a.db").unwrap().unwrap().purge_pending);

    // The next pass, with a viewer that acks, plans it first and deletes it.
    let segments = catalog.list().unwrap();
    let again = plan(&segments, &Caps::default(), 0);
    assert_eq!(again, ["a.db"]);
    let ack = Recorder {
        seen: RefCell::new(Vec::new()),
        ack: true,
    };
    let out = purge(&dir, &catalog, &again, &ack).unwrap();
    assert!(matches!(out.as_slice(), [Outcome::Purged(_)]));
    assert!(!dir.join("a.db").exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(windows)]
#[test]
fn a_file_that_is_still_open_is_deferred_not_lost() {
    use std::os::windows::fs::OpenOptionsExt as _;

    let dir = scratch("open");
    make_files(&dir, &["a.db"]);
    let catalog = catalog_with(&["a.db"]);
    // A reader that opened the file the way SQLite does on Windows: without
    // FILE_SHARE_DELETE, so a delete fails while it is open.
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0x1 | 0x2)
        .open(dir.join("a.db"))
        .unwrap();
    let viewer = Recorder {
        seen: RefCell::new(Vec::new()),
        ack: true,
    };
    let out = purge(&dir, &catalog, &[String::from("a.db")], &viewer).unwrap();
    assert!(matches!(out.as_slice(), [Outcome::Deferred(_)]));
    assert!(dir.join("a.db").exists());
    assert!(catalog.get("a.db").unwrap().unwrap().purge_pending);
    drop(held);
    let out = purge(&dir, &catalog, &[String::from("a.db")], &viewer).unwrap();
    assert!(matches!(out.as_slice(), [Outcome::Purged(_)]));
    assert!(!dir.join("a.db").exists());
    std::fs::remove_dir_all(&dir).unwrap();
}
