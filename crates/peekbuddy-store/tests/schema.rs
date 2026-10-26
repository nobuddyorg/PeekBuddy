//! Schema v1: DDL, migrations, fixtures, names, ids and query plans (#53).
#![expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test helpers outside #[test] fns may unwrap and panic"
)]

use std::fmt::Write as _;
use std::path::PathBuf;

use peekbuddy_core::event::{Flags, GapReason, Kind, NameKind, Source, WatchState};
use peekbuddy_core::name::WideStr;
use peekbuddy_store::ids::{FileIdColumns, join, split};
use peekbuddy_store::interner::{Interner, NodeKey, TOP_LEVEL};
use peekbuddy_store::names::{from_columns, to_columns};
use peekbuddy_store::schema::{
    self, APPLICATION_ID, INSERT_EVENT, INSERT_NODE, SCHEMA_VERSION, StoreError, UPSERT_STATS,
};
use proptest::prelude::*;
use rusqlite::{Connection, params};

fn memory() -> Connection {
    let mut conn = Connection::open_in_memory().unwrap();
    schema::migrate(&mut conn).unwrap();
    conn
}

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/schema")
}

#[test]
fn a_new_segment_gets_the_current_schema() {
    let conn = memory();
    let version: i64 = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(version, i64::from(SCHEMA_VERSION));
    let app: i32 = conn
        .pragma_query_value(None, "application_id", |r| r.get(0))
        .unwrap();
    assert_eq!(app, APPLICATION_ID);
    let check: String = conn
        .pragma_query_value(None, "integrity_check", |r| r.get(0))
        .unwrap();
    assert_eq!(check, "ok");
}

#[test]
fn migrating_twice_changes_nothing() {
    let mut conn = memory();
    assert_eq!(schema::migrate(&mut conn).unwrap(), SCHEMA_VERSION);
    let tables: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE type = 'table'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(tables >= 8);
}

#[test]
fn a_newer_schema_is_refused() {
    let mut conn = memory();
    conn.execute(
        "UPDATE meta SET value = 99 WHERE key = 'schema_version'",
        [],
    )
    .unwrap();
    match schema::migrate(&mut conn) {
        Err(StoreError::TooNew {
            found: 99,
            supported,
        }) => assert_eq!(supported, SCHEMA_VERSION),
        other => panic!("{other:?}"),
    }
    let message = schema::migrate(&mut conn).unwrap_err().to_string();
    assert!(message.contains("newer"), "{message}");
}

#[test]
fn a_foreign_database_is_refused() {
    let mut conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE other (x)").unwrap();
    assert!(matches!(
        schema::migrate(&mut conn),
        Err(StoreError::NotASegment { application_id: 0 })
    ));
}

#[test]
fn a_missing_version_is_an_error() {
    let mut conn = memory();
    conn.execute("DELETE FROM meta", []).unwrap();
    assert!(matches!(
        schema::migrate(&mut conn),
        Err(StoreError::BadVersion)
    ));
}

#[test]
fn steps_run_in_order_and_each_bumps_the_version() {
    fn one(tx: &rusqlite::Transaction<'_>) -> rusqlite::Result<()> {
        tx.execute_batch(schema::SEGMENT_DDL)
    }
    fn two(tx: &rusqlite::Transaction<'_>) -> rusqlite::Result<()> {
        tx.execute_batch("CREATE TABLE extra (x INTEGER) STRICT")
    }
    let mut conn = Connection::open_in_memory().unwrap();
    assert_eq!(schema::migrate_with(&mut conn, &[one]).unwrap(), 1);
    assert_eq!(schema::migrate_with(&mut conn, &[one, two]).unwrap(), 2);
    let version: i64 = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(version, 2);
    conn.execute("INSERT INTO extra VALUES (1)", []).unwrap();
}

#[test]
fn a_failing_step_rolls_back_and_keeps_the_version() {
    fn good(tx: &rusqlite::Transaction<'_>) -> rusqlite::Result<()> {
        tx.execute_batch(schema::SEGMENT_DDL)
    }
    fn bad(tx: &rusqlite::Transaction<'_>) -> rusqlite::Result<()> {
        tx.execute_batch("CREATE TABLE half (x INTEGER) STRICT; SELECT * FROM missing")
    }
    let mut conn = Connection::open_in_memory().unwrap();
    schema::migrate_with(&mut conn, &[good]).unwrap();
    assert!(schema::migrate_with(&mut conn, &[good, bad]).is_err());
    let half: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name = 'half'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(half, 0);
    let version: i64 = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(version, 1);
}

/// The content the committed fixtures hold.
fn fill_fixture(conn: &Connection) {
    let mut interner = Interner::new();
    conn.execute(
        "INSERT INTO vol (id, guid, serial, letter) VALUES (1, 'guid', 7, 'C')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO root (id, path, recursive, vol_id) VALUES (1, 'C:\\\\data', 1, 1)",
        [],
    )
    .unwrap();
    let units = |s: &str| s.encode_utf16().collect::<Vec<u16>>();
    let dir = units("src");
    let file = units("h\u{e9}llo.txt");
    let odd = vec![0x61_u16, 0xD800];
    let dir_id = interner
        .intern(
            conn,
            &NodeKey {
                vol_id: 1,
                parent_id: TOP_LEVEL,
                name: WideStr::new(&dir),
                name_kind: NameKind::Long,
                file_id: Some(10),
            },
        )
        .unwrap();
    for (n, name) in [&file, &odd].into_iter().enumerate() {
        let node = interner
            .intern(
                conn,
                &NodeKey {
                    vol_id: 1,
                    parent_id: dir_id,
                    name: WideStr::new(name),
                    name_kind: NameKind::Unknown,
                    file_id: None,
                },
            )
            .unwrap();
        let (tag, arg) = Kind::Created.to_wire();
        conn.prepare(INSERT_EVENT)
            .unwrap()
            .execute(params![
                i64::try_from(n + 1).unwrap(),
                133_000_000_000_000_000_i64,
                i64::from(tag) << 8 | i64::from(arg),
                i64::from(Source::Rdcw.tag()),
                1,
                node,
                None::<i64>,
                0,
                5,
                0x20,
                None::<i64>,
                None::<i64>,
                None::<i64>,
                None::<i64>,
                1,
                1,
                0,
                None::<i64>,
                None::<i64>,
            ])
            .unwrap();
    }
}

#[test]
#[ignore = "regenerates the committed fixtures: PEEKBUDDY_WRITE_FIXTURES=1 cargo test -p peekbuddy-store --test schema -- --ignored"]
fn write_fixtures() {
    assert_eq!(
        std::env::var("PEEKBUDDY_WRITE_FIXTURES").as_deref(),
        Ok("1")
    );
    let path = fixture_dir().join(format!("v{SCHEMA_VERSION}.db"));
    let _ = std::fs::remove_file(&path);
    let conn = schema::open(&path).unwrap();
    fill_fixture(&conn);
    conn.pragma_update(None, "journal_mode", "DELETE").unwrap();
}

#[test]
fn every_committed_fixture_migrates_and_keeps_its_rows() {
    let mut seen = 0;
    for entry in std::fs::read_dir(fixture_dir()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "db") {
            continue;
        }
        // Work on a copy: opening may write.
        let copy = std::env::temp_dir().join(format!(
            "peekbuddy-fixture-{}-{}",
            std::process::id(),
            path.file_name().unwrap().to_string_lossy()
        ));
        std::fs::copy(&path, &copy).unwrap();
        let conn = schema::open(&copy).unwrap();
        let check: String = conn
            .pragma_query_value(None, "integrity_check", |r| r.get(0))
            .unwrap();
        assert_eq!(check, "ok", "{}", path.display());
        let version: i64 = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_version'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(version, i64::from(SCHEMA_VERSION));
        let rows: Vec<(i64, String, Option<Vec<u8>>)> = conn
            .prepare("SELECT parent_id, name, name_w FROM node ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(
            rows,
            vec![
                (0, "src".to_owned(), None),
                (1, "h\u{e9}llo.txt".to_owned(), None),
                (1, "a\u{FFFD}".to_owned(), Some(vec![0x61, 0, 0x00, 0xD8])),
            ]
        );
        let events: i64 = conn
            .query_row("SELECT count(*) FROM event", [], |r| r.get(0))
            .unwrap();
        assert_eq!(events, 2);
        drop(conn);
        let _ = std::fs::remove_file(&copy);
        seen += 1;
    }
    assert!(seen >= 1, "no fixture committed");
}

#[test]
fn ddl_snapshot() {
    let conn = memory();
    let ddl: Vec<String> = conn
        .prepare("SELECT sql FROM sqlite_schema WHERE sql IS NOT NULL AND name NOT LIKE 'node_fts_%' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    insta::assert_snapshot!(ddl.join(";\n\n"));
}

#[test]
fn enum_discriminants_snapshot() {
    let mut out = String::new();
    for source in Source::ALL {
        writeln!(out, "source {source:?} = {}", source.tag()).unwrap();
    }
    for kind in NameKind::ALL {
        writeln!(out, "name_kind {kind:?} = {}", kind.tag()).unwrap();
    }
    let kinds = [
        Kind::Created,
        Kind::Modified,
        Kind::Deleted,
        Kind::RenamedFrom,
        Kind::RenamedTo,
        Kind::AttributesChanged,
        Kind::SecurityChanged,
        Kind::StreamChanged,
        Kind::LinkChanged,
        Kind::ReconciledCreated,
        Kind::ReconciledModified,
        Kind::ReconciledDeleted,
        Kind::Other,
        Kind::EngineStarted,
        Kind::EngineStopped,
        Kind::SegmentRotated,
        Kind::RetentionPurged,
        Kind::SettingsChanged,
        Kind::ReconciledRenamed,
        Kind::ReconcileStarted,
        Kind::ReconcileFinished,
        Kind::BaselineEstablished,
        Kind::AccessDenied,
    ];
    for kind in kinds {
        let (tag, arg) = kind.to_wire();
        writeln!(
            out,
            "kind {kind:?} = {}",
            i64::from(tag) << 8 | i64::from(arg)
        )
        .unwrap();
    }
    for reason in GapReason::ALL {
        let (tag, arg) = Kind::Overflow(reason).to_wire();
        writeln!(
            out,
            "kind Overflow({reason:?}) = {}",
            i64::from(tag) << 8 | i64::from(arg)
        )
        .unwrap();
    }
    for state in WatchState::ALL {
        let (tag, arg) = Kind::WatchStateChanged(state).to_wire();
        writeln!(
            out,
            "kind WatchStateChanged({state:?}) = {}",
            i64::from(tag) << 8 | i64::from(arg)
        )
        .unwrap();
    }
    for bit in 0..16_u32 {
        if let Some(flags) = Flags::from_bits(1 << bit) {
            writeln!(out, "flags bit {bit} = {flags:?}").unwrap();
        }
    }
    insta::assert_snapshot!(out);
}

#[test]
fn query_plan_snapshots() {
    let conn = memory();
    let mut out = String::new();
    for (label, sql) in [
        ("insert event", INSERT_EVENT),
        ("insert node", INSERT_NODE),
        ("upsert stats", UPSERT_STATS),
        ("select node", schema::SELECT_NODE),
        (
            "events by node",
            "SELECT id FROM event WHERE node_id = ?1 ORDER BY id",
        ),
        (
            "events by root",
            "SELECT id FROM event WHERE root_id = ?1 ORDER BY id",
        ),
    ] {
        writeln!(out, "-- {label}").unwrap();
        let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
        let n = stmt.parameter_count();
        let params: Vec<&dyn rusqlite::ToSql> = vec![&rusqlite::types::Null; n];
        let mut rows = stmt.query(params.as_slice()).unwrap();
        while let Some(row) = rows.next().unwrap() {
            let detail: String = row.get(3).unwrap();
            writeln!(out, "{detail}").unwrap();
        }
    }
    insta::assert_snapshot!(out);
}

#[test]
fn stats_upsert_accumulates() {
    let conn = memory();
    for (n, lo, hi) in [(3, 10, 12), (2, 5, 20)] {
        conn.execute(UPSERT_STATS, params![0, 1, 0, n, lo, hi])
            .unwrap();
    }
    let row: (i64, i64, i64) = conn
        .query_row("SELECT n, min_id, max_id FROM stats", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .unwrap();
    assert_eq!(row, (5, 5, 20));
}

#[test]
fn hour_buckets_round_down_even_before_the_epoch() {
    use schema::{TICKS_PER_HOUR, hour_bucket};
    assert_eq!(hour_bucket(0), 0);
    assert_eq!(hour_bucket(TICKS_PER_HOUR - 1), 0);
    assert_eq!(hour_bucket(TICKS_PER_HOUR), TICKS_PER_HOUR);
    assert_eq!(hour_bucket(5 * TICKS_PER_HOUR + 7), 5 * TICKS_PER_HOUR);
    assert_eq!(hour_bucket(-1), -TICKS_PER_HOUR);
}

proptest! {
    #![proptest_config(ProptestConfig { max_shrink_iters: 64, ..ProptestConfig::default() })]

    #[test]
    fn names_survive_a_store_and_load(units in prop::collection::vec(any::<u16>(), 1..40)) {
        // The reserved characters never reach a node name.
        prop_assume!(units.iter().all(|&u| u != 0 && u != u16::from(b'\\') && u != u16::from(b'/')));
        let conn = memory();
        let mut interner = Interner::new();
        let id = interner
            .intern(&conn, &NodeKey {
                vol_id: 1,
                parent_id: TOP_LEVEL,
                name: WideStr::new(&units),
                name_kind: NameKind::Long,
                file_id: None,
            })
            .unwrap();
        let (text, wide): (String, Option<Vec<u8>>) = conn
            .query_row("SELECT name, name_w FROM node WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        let back = from_columns(&text, wide.as_deref()).unwrap();
        prop_assert_eq!(back.into_units(), units);
    }

    #[test]
    fn file_ids_round_trip(id in any::<u128>()) {
        let columns = split(id);
        prop_assert_eq!(join(columns), id);
        let conn = memory();
        conn.execute("INSERT INTO node (id, vol_id, parent_id, name, name_kind, file_id) VALUES (1, 1, 0, 'x', 0, ?1)", [columns.lo]).unwrap();
        let lo: i64 = conn.query_row("SELECT file_id FROM node", [], |r| r.get(0)).unwrap();
        prop_assert_eq!(join(FileIdColumns { lo, hi: columns.hi }), id);
    }
}

#[test]
fn names_differing_only_in_lone_surrogates_are_different_nodes() {
    let conn = memory();
    let mut interner = Interner::new();
    let a = vec![0x61_u16, 0xD800];
    let b = vec![0x61_u16, 0xD801];
    let c = vec![0x61_u16, 0xFFFD];
    let mut ids = Vec::new();
    for name in [&a, &b, &c] {
        ids.push(
            interner
                .intern(
                    &conn,
                    &NodeKey {
                        vol_id: 1,
                        parent_id: TOP_LEVEL,
                        name: WideStr::new(name),
                        name_kind: NameKind::Long,
                        file_id: None,
                    },
                )
                .unwrap(),
        );
    }
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 3);
    assert_eq!(
        to_columns(WideStr::new(&a)).text,
        to_columns(WideStr::new(&b)).text
    );
}

#[test]
fn the_interner_hits_in_memory_and_finds_nodes_after_a_restart() {
    let conn = memory();
    let name = "report.docx".encode_utf16().collect::<Vec<_>>();
    let key = NodeKey {
        vol_id: 1,
        parent_id: TOP_LEVEL,
        name: WideStr::new(&name),
        name_kind: NameKind::Long,
        file_id: None,
    };
    let mut first = Interner::new();
    assert_eq!(first.lookup(TOP_LEVEL, key.name), None);
    let id = first.intern(&conn, &key).unwrap();
    assert_eq!(first.lookup(TOP_LEVEL, key.name), Some(id));
    assert_eq!(first.intern(&conn, &key).unwrap(), id);
    // `LIKE` on an external-content table can fall back to reading `node`,
    // so count the index's own per-row bookkeeping instead.
    let fts_rows = |conn: &Connection| -> i64 {
        conn.query_row("SELECT count(*) FROM node_fts_docsize", [], |r| r.get(0))
            .unwrap()
    };
    assert_eq!(fts_rows(&conn), 1, "a new node is indexed once");
    assert_eq!(first.stats().entries, 1);
    assert_eq!(first.stats().name_bytes, name.len() * 2);
    // A new process starts cold and still gets the same id, and the search
    // index has exactly one row.
    let mut cold = Interner::new();
    assert_eq!(cold.intern(&conn, &key).unwrap(), id);
    let nodes: i64 = conn
        .query_row("SELECT count(*) FROM node", [], |r| r.get(0))
        .unwrap();
    assert_eq!(nodes, 1);
    let hits: i64 = conn
        .query_row(
            "SELECT count(*) FROM node_fts WHERE name LIKE '%PORT.d%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(hits, 1);
    // A quoted phrase of several trigrams works with `detail='full'` (it is
    // rejected with `'none'` and with `'column'`) and is the indexed search
    // path (#35).
    let phrase_hits: i64 = conn
        .query_row(
            "SELECT count(*) FROM node_fts WHERE node_fts MATCH '\"port.d\"'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(phrase_hits, 1);
    cold.clear();
    assert_eq!(cold.stats().entries, 0);
    assert_eq!(cold.lookup(TOP_LEVEL, key.name), None);
}

#[test]
fn the_same_name_under_different_parents_is_different() {
    let conn = memory();
    let mut interner = Interner::new();
    let name = "a.txt".encode_utf16().collect::<Vec<_>>();
    let upper = "A.txt".encode_utf16().collect::<Vec<_>>();
    let mut ids = Vec::new();
    for (parent, n) in [(0, &name), (1, &name), (0, &upper)] {
        ids.push(
            interner
                .intern(
                    &conn,
                    &NodeKey {
                        vol_id: 1,
                        parent_id: parent,
                        name: WideStr::new(n),
                        name_kind: NameKind::Long,
                        file_id: None,
                    },
                )
                .unwrap(),
        );
    }
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 3, "case-preserving, byte-exact, per parent");
}

/// Bytes per event for a realistic mix, from `dbstat`. The 1M-event churn
/// corpus gate (#25, #35) replaces this smaller synthetic mix.
#[test]
fn bytes_per_event_stay_small() {
    const EVENTS: usize = 40_000;
    let path = std::env::temp_dir().join(format!("peekbuddy-bytes-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let conn = schema::open(&path).unwrap();
    conn.execute_batch("BEGIN").unwrap();
    let mut interner = Interner::new();
    let mut parents = vec![TOP_LEVEL];
    let mut insert = conn.prepare(INSERT_EVENT).unwrap();
    for i in 0..EVENTS {
        let name: Vec<u16> = format!("project-{}\\file-{}.txt", i % 50, i % 5_000)
            .encode_utf16()
            .collect();
        let parent = parents[i % parents.len()];
        let node = interner
            .intern(
                &conn,
                &NodeKey {
                    vol_id: 1,
                    parent_id: parent,
                    name: WideStr::new(&name),
                    name_kind: NameKind::Long,
                    file_id: Some(i64::try_from(i).unwrap()),
                },
            )
            .unwrap();
        if i % 400 == 0 {
            parents.push(node);
        }
        let (tag, arg) = Kind::Modified.to_wire();
        insert
            .execute(params![
                i64::try_from(i + 1).unwrap(),
                133_000_000_000_000_000_i64 + i64::try_from(i).unwrap() * 10_000,
                i64::from(tag) << 8 | i64::from(arg),
                i64::from(Source::Rdcw.tag()),
                3,
                node,
                None::<i64>,
                0,
                4096,
                0x20,
                None::<i64>,
                i64::try_from(i).unwrap(),
                None::<i64>,
                None::<i64>,
                1,
                1,
                0,
                None::<i64>,
                None::<i64>,
            ])
            .unwrap();
    }
    drop(insert);
    conn.execute_batch("COMMIT").unwrap();
    let total: i64 = conn
        .query_row(
            "SELECT sum(pgsize) FROM dbstat WHERE name IN ('event', 'event_node', 'event_root')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let per_event = total / i64::try_from(EVENTS).unwrap();
    drop(conn);
    let _ = std::fs::remove_file(&path);
    // Provisional ceiling until #35 fixes the target (estimate 90-110 B; measured 72).
    assert!(per_event <= 100, "{per_event} B per event");
    assert!(
        per_event >= 20,
        "{per_event} B per event looks too small to be true"
    );
}

#[test]
fn errors_expose_their_source() {
    use std::error::Error;
    let sqlite = StoreError::from(rusqlite::Error::QueryReturnedNoRows);
    assert!(sqlite.source().is_some());
    assert!(sqlite.to_string().contains("SQLite"));
    for other in [
        StoreError::TooNew {
            found: 2,
            supported: 1,
        },
        StoreError::NotASegment { application_id: 5 },
        StoreError::BadVersion,
    ] {
        assert!(other.source().is_none());
    }
    assert!(
        StoreError::NotASegment { application_id: 5 }
            .to_string()
            .contains("0x5")
    );
}

/// Why `node_fts` is `detail='full'` (#35): only `full` answers a `MATCH`
/// phrase of more than one trigram, which is how substring search uses the
/// index. The spike's first numbers for `column` were errors counted as rows.
#[test]
fn only_detail_full_answers_a_multi_trigram_phrase() {
    for (detail, works) in [("none", false), ("column", false), ("full", true)] {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(&format!(
            "CREATE TABLE node(id INTEGER PRIMARY KEY, name TEXT);\
             CREATE VIRTUAL TABLE f USING fts5(name, content='node', content_rowid='id', \
             tokenize='trigram case_sensitive 0 remove_diacritics 1', detail='{detail}');\
             INSERT INTO node VALUES (1, 'report.docx');\
             INSERT INTO f(rowid, name) VALUES (1, 'report.docx');"
        ))
        .unwrap();
        let hits = |text: &str| -> rusqlite::Result<i64> {
            conn.query_row(
                "SELECT count(*) FROM f WHERE f MATCH ?1",
                [format!("\"{text}\"")],
                |r| r.get(0),
            )
        };
        assert_eq!(hits("por").unwrap(), 1, "one trigram works with {detail}");
        assert_eq!(hits("port.d").is_ok(), works, "a phrase with {detail}");
    }
}
