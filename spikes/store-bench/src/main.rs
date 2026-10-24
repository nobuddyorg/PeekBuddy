//! Spike #35: SQLite ingest and query performance.
//!
//! `store-bench <experiment> [--rows N] [--dir PATH] [--reps N] [--label L]`
//! prints one JSON line per measured cell. Experiments: `commit` (Q1),
//! `checkpoint` (Q2), `size` (Q3), `query` (Q4, Q5, Q10), `readers` (Q6),
//! `memory` (Q9), `all`. Q7 and Q8 are matrix dimensions of the workflow
//! (runner label, `LIBSQLITE3_FLAGS` build).

use std::alloc::System;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use peekbuddy_alloc::{CountingAlloc, measure};
use peekbuddy_core::event::NameKind;
use peekbuddy_core::name::WideStr;
use peekbuddy_store::interner::{Interner, NodeKey, TOP_LEVEL};
use peekbuddy_store::schema::{self, INSERT_EVENT};
use rusqlite::{Connection, OpenFlags, params};

#[global_allocator]
static ALLOC: CountingAlloc<System> = CountingAlloc::new(System);

struct Config {
    rows: u64,
    dir: PathBuf,
    reps: usize,
    label: String,
    cache_kib: i64,
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

fn emit(cfg: &Config, experiment: &str, cell: &str, fields: &[(&str, String)]) {
    let mut line = format!(
        "{{\"experiment\":\"{experiment}\",\"cell\":\"{cell}\",\"label\":\"{}\"",
        cfg.label
    );
    for (key, value) in fields {
        line.push_str(&format!(",\"{key}\":{value}"));
    }
    line.push('}');
    println!("{line}");
}

fn num<T: std::fmt::Display>(v: T) -> String {
    v.to_string()
}

fn pct(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted.get(idx).copied().unwrap_or(0)
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(|a, b| a.total_cmp(b));
    values.get(values.len() / 2).copied().unwrap_or(0.0)
}

fn private_bytes() -> u64 {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::ProcessStatus::{
            GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX,
        };
        use windows_sys::Win32::System::Threading::GetCurrentProcess;
        let mut counters: PROCESS_MEMORY_COUNTERS_EX = unsafe { std::mem::zeroed() };
        counters.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
        let ok = unsafe {
            GetProcessMemoryInfo(
                GetCurrentProcess(),
                (&raw mut counters).cast(),
                counters.cb,
            )
        };
        if ok == 0 { 0 } else { counters.PrivateUsage as u64 }
    }
    #[cfg(not(windows))]
    {
        std::fs::read_to_string("/proc/self/statm")
            .ok()
            .and_then(|s| s.split_whitespace().nth(1).and_then(|v| v.parse::<u64>().ok()))
            .map_or(0, |pages| pages * 4096)
    }
}

fn file_size(path: &Path) -> u64 {
    std::fs::metadata(path).map_or(0, |m| m.len())
}

fn wal_path(db: &Path) -> PathBuf {
    let mut name = db.as_os_str().to_owned();
    name.push("-wal");
    PathBuf::from(name)
}

fn remove_db(db: &Path) {
    for suffix in ["", "-wal", "-shm"] {
        let mut name = db.as_os_str().to_owned();
        name.push(suffix);
        let _ = std::fs::remove_file(PathBuf::from(name));
    }
}

fn open_writer(db: &Path, page_size: u32, cache_kib: i64, autocheckpoint: u32) -> Connection {
    remove_db(db);
    let mut conn = Connection::open(db).expect("open");
    conn.pragma_update(None, "page_size", page_size).expect("page_size");
    schema::migrate(&mut conn).expect("migrate");
    tune(&conn, cache_kib, autocheckpoint);
    conn
}

fn tune(conn: &Connection, cache_kib: i64, autocheckpoint: u32) {
    conn.pragma_update(None, "journal_mode", "WAL").expect("wal");
    conn.pragma_update(None, "synchronous", "NORMAL").expect("sync");
    conn.pragma_update(None, "foreign_keys", "OFF").expect("fk");
    conn.pragma_update(None, "temp_store", "MEMORY").expect("temp");
    conn.pragma_update(None, "busy_timeout", 5000).expect("busy");
    conn.pragma_update(None, "mmap_size", 0).expect("mmap");
    conn.pragma_update(None, "wal_autocheckpoint", autocheckpoint).expect("ckpt");
    conn.pragma_update(None, "cache_size", -cache_kib).expect("cache");
}

/// Interns a directory tree of `nodes` nodes; returns the node ids.
fn build_nodes(conn: &Connection, nodes: usize, rng: &mut Rng) -> Vec<i64> {
    let mut interner = Interner::new();
    let mut ids: Vec<i64> = vec![TOP_LEVEL];
    let mut leaves = Vec::with_capacity(nodes);
    conn.execute_batch("BEGIN").expect("begin");
    for i in 0..nodes {
        let parent = ids.get(rng.below(ids.len() as u64) as usize).copied().unwrap_or(TOP_LEVEL);
        let len = 8 + rng.below(40) as usize;
        let token: String = (0..12).map(|_| char::from(b'a' + rng.below(26) as u8)).collect();
        let name: Vec<u16> = format!("{}-{i:06}-{token}", &"projectsourcecachebuild"[..len.min(20)])
            .encode_utf16()
            .collect();
        let id = interner
            .intern(
                conn,
                &NodeKey {
                    vol_id: 1,
                    parent_id: parent,
                    name: WideStr::new(&name),
                    name_kind: NameKind::Long,
                    file_id: Some(i as i64),
                },
            )
            .expect("intern");
        if i % 8 == 0 {
            ids.push(id);
        }
        leaves.push(id);
    }
    conn.execute_batch("COMMIT").expect("commit");
    leaves
}

fn insert_one(stmt: &mut rusqlite::CachedStatement<'_>, id: u64, node: i64, rng: &mut Rng) {
    let kind = match rng.below(1000) {
        0 => 9 << 8,
        1..=100 => 1 << 8,
        _ => 0,
    };
    stmt.execute(params![
        id as i64,
        133_000_000_000_000_000_i64 + id as i64 * 10_000,
        kind,
        0,
        3,
        node,
        None::<i64>,
        0,
        4096,
        0x20,
        None::<i64>,
        id as i64,
        None::<i64>,
        None::<i64>,
        1,
        (id % 8) as i64,
        0,
        None::<i64>,
        None::<i64>,
    ])
    .expect("insert");
}

struct Ev {
    id: u64,
    node: i64,
    at: Instant,
}

struct RunResult {
    elapsed: Duration,
    events: u64,
    latencies_us: Vec<u64>,
    max_commit_us: u64,
    commits: u64,
    peak_wal: u64,
}

/// One writer thread with group commit; the producer paces to `rate`
/// events/s (0 = unthrottled) for `total` events.
fn run_policy(
    db: &Path,
    nodes: &[i64],
    max_batch: usize,
    linger: Duration,
    rate: u64,
    total: u64,
    autocheckpoint: u32,
    cache_kib: i64,
    since_last: bool,
) -> RunResult {
    let conn = open_writer(db, 4096, cache_kib, autocheckpoint);
    let mut rng = Rng(0xDEAD_BEEF_1234_5678);
    let node_ids = build_nodes(&conn, 20_000, &mut rng);
    let _ = nodes;
    let (tx, rx) = mpsc::sync_channel::<Ev>(65_536);
    let node_ids = Arc::new(node_ids);
    let producer_nodes = Arc::clone(&node_ids);
    let producer = thread::spawn(move || {
        let mut rng = Rng(42);
        let started = Instant::now();
        for id in 1..=total {
            if rate > 0 {
                let due = started + Duration::from_nanos(id * 1_000_000_000 / rate);
                while Instant::now() < due {
                    std::hint::spin_loop();
                }
            }
            let node = producer_nodes
                .get(rng.below(producer_nodes.len() as u64) as usize)
                .copied()
                .unwrap_or(1);
            if tx.send(Ev { id, node, at: Instant::now() }).is_err() {
                break;
            }
        }
    });
    let wal = wal_path(db);
    let mut latencies = Vec::with_capacity(if rate > 0 { total as usize } else { 0 });
    let mut max_commit = 0_u64;
    let mut commits = 0_u64;
    let mut peak_wal = 0_u64;
    let started = Instant::now();
    let mut rng = Rng(7);
    let mut done = 0_u64;
    let mut closed = false;
    while !closed {
        let Ok(first) = rx.recv() else { break };
        let first_at = Instant::now();
        let mut pending: Vec<Instant> = Vec::with_capacity(max_batch);
        conn.execute_batch("BEGIN").expect("begin");
        {
            let mut stmt = conn.prepare_cached(INSERT_EVENT).expect("prepare");
            insert_one(&mut stmt, first.id, first.node, &mut rng);
            pending.push(first.at);
            // R1: the linger runs from the first pending event (the batch
            // deadline); `since_last` restarts it at every event, which is
            // the naive policy that never fires under steady arrivals.
            let mut last_at = first_at;
            while pending.len() < max_batch && first_at.elapsed() < Duration::from_millis(100) {
                let anchor = if since_last { last_at } else { first_at };
                let Some(remaining) = linger.checked_sub(anchor.elapsed()) else { break };
                let next = match rx.try_recv() {
                    Ok(ev) => Ok(ev),
                    Err(mpsc::TryRecvError::Empty) => rx.recv_timeout(remaining),
                    Err(mpsc::TryRecvError::Disconnected) => Err(mpsc::RecvTimeoutError::Disconnected),
                };
                match next {
                    Ok(ev) => {
                        insert_one(&mut stmt, ev.id, ev.node, &mut rng);
                        pending.push(ev.at);
                        last_at = Instant::now();
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => break,
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        closed = true;
                        break;
                    }
                }
            }
        }
        let commit_started = Instant::now();
        conn.execute_batch("COMMIT").expect("commit");
        let now = Instant::now();
        max_commit = max_commit.max((now - commit_started).as_micros() as u64);
        commits += 1;
        done += pending.len() as u64;
        if rate > 0 {
            latencies.extend(pending.iter().map(|at| (now - *at).as_micros() as u64));
        }
        peak_wal = peak_wal.max(file_size(&wal));
    }
    let elapsed = started.elapsed();
    let _ = producer.join();
    RunResult {
        elapsed,
        events: done,
        latencies_us: latencies,
        max_commit_us: max_commit,
        commits,
        peak_wal,
    }
}

/// Q1: batch caps and lingers; burst throughput and paced latency.
fn commit(cfg: &Config) {
    let db = cfg.dir.join("commit.db");
    let mut cells: Vec<(usize, u64, bool)> = Vec::new();
    for max_batch in [100_usize, 1_000, 10_000] {
        for linger_ms in [5_u64, 10, 25] {
            cells.push((max_batch, linger_ms, false));
        }
    }
    // The naive policy for comparison.
    cells.push((1_000, 5, true));
    for (max_batch, linger_ms, since_last) in cells {
        {
            let linger = Duration::from_millis(linger_ms);
            let mut burst = Vec::new();
            let mut p50 = Vec::new();
            let mut p99 = Vec::new();
            let mut commits_per_s = Vec::new();
            for _ in 0..cfg.reps {
                let b = run_policy(&db, &[], max_batch, linger, 0, 300_000, 0, 2_000, since_last);
                burst.push(b.events as f64 / b.elapsed.as_secs_f64());
                let mut p = run_policy(&db, &[], max_batch, linger, 10_000, 50_000, 0, 2_000, since_last);
                p.latencies_us.sort_unstable();
                p50.push(pct(&p.latencies_us, 0.50) as f64);
                p99.push(pct(&p.latencies_us, 0.99) as f64);
                commits_per_s.push(p.commits as f64 / p.elapsed.as_secs_f64());
            }
            emit(
                cfg,
                "commit",
                &format!(
                    "batch{max_batch}-linger{linger_ms}ms{}",
                    if since_last { "-since-last-event" } else { "" }
                ),
                &[
                    ("burst_events_per_s", num(median(&mut burst) as u64)),
                    ("paced10k_p50_us", num(median(&mut p50) as u64)),
                    ("paced10k_p99_us", num(median(&mut p99) as u64)),
                    ("paced10k_commits_per_s", num(median(&mut commits_per_s) as u64)),
                ],
            );
        }
    }
    remove_db(&db);
}

/// Q2: checkpoint policies under a sustained burst.
fn checkpoint(cfg: &Config) {
    let db = cfg.dir.join("checkpoint.db");
    for (name, autockpt) in [("auto1000", 1000_u32), ("auto4000", 4000), ("off-passive-at-end", 0)] {
        let mut worst = Vec::new();
        let mut peak = Vec::new();
        let mut rate = Vec::new();
        for _ in 0..cfg.reps {
            let r = run_policy(
                &db,
                &[],
                1_000,
                Duration::from_millis(5),
                0,
                (cfg.rows / 2).max(200_000),
                autockpt,
                2_000,
                false,
            );
            worst.push(r.max_commit_us as f64);
            peak.push(r.peak_wal as f64);
            rate.push(r.events as f64 / r.elapsed.as_secs_f64());
        }
        emit(
            cfg,
            "checkpoint",
            name,
            &[
                ("worst_commit_us", num(median(&mut worst) as u64)),
                ("peak_wal_bytes", num(median(&mut peak) as u64)),
                ("events_per_s", num(median(&mut rate) as u64)),
            ],
        );
    }
    remove_db(&db);
}

fn dbstat_by_name(conn: &Connection) -> Vec<(String, i64)> {
    let mut stmt = conn
        .prepare("SELECT name, sum(pgsize) FROM dbstat GROUP BY name ORDER BY name")
        .expect("dbstat");
    stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
        .expect("rows")
        .map(Result::unwrap)
        .collect()
}

fn fill_events(conn: &Connection, nodes: &[i64], rows: u64, rng: &mut Rng) {
    let mut id = 0_u64;
    while id < rows {
        conn.execute_batch("BEGIN").expect("begin");
        {
            let mut stmt = conn.prepare_cached(INSERT_EVENT).expect("prepare");
            for _ in 0..100_000.min(rows - id) {
                id += 1;
                let node = nodes.get(rng.below(nodes.len() as u64) as usize).copied().unwrap_or(1);
                insert_one(&mut stmt, id, node, rng);
            }
        }
        conn.execute_batch("COMMIT").expect("commit");
    }
}

/// Q3: bytes per event for page sizes, index sets and inline paths.
fn size(cfg: &Config) {
    let db = cfg.dir.join("size.db");
    let rows = cfg.rows.min(500_000);
    for page_size in [4096_u32, 8192] {
        for variant in ["interned-2idx", "interned-no-idx", "interned-node-idx", "interned-root-idx", "inline-path"] {
            let conn = open_writer(&db, page_size, 2_000, 1000);
            let mut rng = Rng(5);
            let nodes = build_nodes(&conn, 50_000, &mut rng);
            if variant == "inline-path" {
                conn.execute_batch(
                    "CREATE TABLE event_inline (id INTEGER PRIMARY KEY, ts INTEGER NOT NULL, kind INTEGER NOT NULL, \
                     source INTEGER NOT NULL, raw_mask INTEGER NOT NULL, path TEXT NOT NULL, is_dir INTEGER NOT NULL, \
                     size INTEGER, attrs INTEGER, file_id INTEGER, vol_id INTEGER NOT NULL, root_id INTEGER NOT NULL, \
                     flags INTEGER NOT NULL) STRICT; CREATE INDEX event_inline_root ON event_inline(root_id, id);",
                )
                .expect("inline");
                conn.execute_batch("BEGIN").expect("begin");
                {
                    let mut stmt = conn
                        .prepare("INSERT INTO event_inline VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)")
                        .expect("prepare");
                    for id in 1..=rows {
                        let n = nodes.get(rng.below(nodes.len() as u64) as usize).copied().unwrap_or(1);
                        let path = format!(
                            "C:\\Users\\dev\\projects\\peekbuddy\\crates\\core\\src\\dir{}\\file-{n:06}.rs",
                            n % 40
                        );
                        stmt.execute(params![id as i64, id as i64, 0, 0, 3, path, 0, 4096, 0x20, id as i64, 1, (id % 8) as i64, 0])
                            .expect("insert");
                    }
                }
                conn.execute_batch("COMMIT").expect("commit");
            } else {
                match variant {
                    "interned-no-idx" => conn.execute_batch("DROP INDEX event_node; DROP INDEX event_root;").expect("drop"),
                    "interned-node-idx" => conn.execute_batch("DROP INDEX event_root;").expect("drop"),
                    "interned-root-idx" => conn.execute_batch("DROP INDEX event_node;").expect("drop"),
                    _ => {}
                }
                fill_events(&conn, &nodes, rows, &mut rng);
            }
            let stat = dbstat_by_name(&conn);
            let get = |name: &str| stat.iter().find(|(n, _)| n == name).map_or(0, |(_, b)| *b);
            let table = if variant == "inline-path" { "event_inline" } else { "event" };
            let table_bytes = get(table);
            let idx_bytes: i64 = stat
                .iter()
                .filter(|(n, _)| n.starts_with("event_") && n != "event_inline")
                .map(|(_, b)| *b)
                .sum();
            let total = table_bytes + idx_bytes;
            let node_bytes = get("node") + get("node_key");
            emit(
                cfg,
                "size",
                &format!("page{page_size}-{variant}"),
                &[
                    ("rows", num(rows)),
                    ("bytes_per_event", num(total as f64 / rows as f64)),
                    ("table_bytes_per_event", num(table_bytes as f64 / rows as f64)),
                    ("index_bytes_per_event", num(idx_bytes as f64 / rows as f64)),
                    ("node_bytes_per_event", num(node_bytes as f64 / rows as f64)),
                ],
            );
            drop(conn);
        }
    }
    remove_db(&db);
}

fn time_query(conn: &Connection, sql: &str, runs: usize, anchors: &[i64]) -> (u64, u64, usize) {
    let mut stmt = conn.prepare(sql).expect("prepare query");
    let mut micros = Vec::with_capacity(runs);
    let mut last_rows = 0;
    for i in 0..runs {
        let anchor = anchors.get(i % anchors.len()).copied().unwrap_or(i64::MAX);
        let started = Instant::now();
        let mut rows = stmt.query(params![anchor]).expect("query");
        let mut n = 0;
        while rows.next().expect("row").is_some() {
            n += 1;
        }
        micros.push(started.elapsed().as_micros() as u64);
        last_rows = n;
    }
    micros.sort_unstable();
    (pct(&micros, 0.5), pct(&micros, 0.99), last_rows)
}

/// Q4, Q5, Q10: pages, FTS, open cost and sealing on a large segment.
fn query(cfg: &Config) {
    let db = cfg.dir.join("query.db");
    let conn = open_writer(&db, 4096, 16_000, 1000);
    let mut rng = Rng(11);
    let nodes = build_nodes(&conn, 200_000, &mut rng);
    let fill_started = Instant::now();
    fill_events(&conn, &nodes, cfg.rows, &mut rng);
    let fill_s = fill_started.elapsed().as_secs_f64();
    conn.pragma_update(None, "wal_checkpoint", "TRUNCATE").ok();
    emit(
        cfg,
        "query",
        "fill",
        &[
            ("rows", num(cfg.rows)),
            ("events_per_s", num((cfg.rows as f64 / fill_s) as u64)),
            ("db_bytes", num(file_size(&db))),
        ],
    );
    let anchors: Vec<i64> = (0..200).map(|_| (1 + rng.below(cfg.rows)) as i64).collect();
    let subtree: Vec<String> = nodes.iter().take(50).map(ToString::to_string).collect();
    let shapes: [(&str, String); 5] = [
        ("newest-all", "SELECT * FROM event WHERE id < ?1 ORDER BY id DESC LIMIT 200".to_owned()),
        ("root", "SELECT * FROM event WHERE root_id = 3 AND id < ?1 ORDER BY id DESC LIMIT 200".to_owned()),
        ("kind-common", "SELECT * FROM event WHERE kind = 256 AND id < ?1 ORDER BY id DESC LIMIT 200".to_owned()),
        ("kind-rare", "SELECT * FROM event WHERE kind = 2304 AND id < ?1 ORDER BY id DESC LIMIT 200".to_owned()),
        (
            "subtree-50-nodes",
            format!(
                "SELECT * FROM event WHERE node_id IN ({}) AND id < ?1 ORDER BY id DESC LIMIT 200",
                subtree.join(",")
            ),
        ),
    ];
    for (name, sql) in &shapes {
        let (p50, p99, rows) = time_query(&conn, sql, 200, &anchors);
        emit(cfg, "query", name, &[("p50_us", num(p50)), ("p99_us", num(p99)), ("rows", num(rows))]);
    }
    // FTS over node names; LIKE is the only substring path with detail=none.
    let stat = dbstat_by_name(&conn);
    let fts_bytes: i64 = stat.iter().filter(|(n, _)| n.starts_with("node_fts")).map(|(_, b)| *b).sum();
    let name_bytes: i64 = conn
        .query_row("SELECT sum(length(name)) FROM node", [], |r| r.get(0))
        .expect("name bytes");
    emit(
        cfg,
        "query",
        "fts-size",
        &[("fts_bytes", num(fts_bytes)), ("name_bytes", num(name_bytes)), ("ratio", num(fts_bytes as f64 / name_bytes as f64))],
    );
    // Needles are slices of real names' random tokens, so they are selective.
    let sample: Vec<String> = (0..20)
        .map(|_| {
            let id = nodes.get(rng.below(nodes.len() as u64) as usize).copied().unwrap_or(1);
            conn.query_row("SELECT name FROM node WHERE id = ?1", [id], |r| r.get::<_, String>(0))
                .expect("name")
        })
        .collect();
    for (name, len) in [("fts-like-3", 3_usize), ("fts-like-5", 5), ("fts-like-10", 10)] {
        for (table, column) in [("node_fts", "name"), ("node", "name")] {
            let mut micros = Vec::new();
            let mut found_total = 0;
            let mut plan = String::new();
            for full in &sample {
                let token = full.rsplit('-').next().unwrap_or(full);
                let needle = &token[..len.min(token.len())];
                let sql = format!("SELECT rowid FROM {table} WHERE {column} LIKE '%{needle}%' LIMIT 200");
                if plan.is_empty() {
                    plan = conn
                        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
                        .and_then(|mut st| st.query_row([], |r| r.get::<_, String>(3)))
                        .unwrap_or_default();
                }
                let mut stmt = conn.prepare(&sql).expect("like");
                let started = Instant::now();
                let mut rows = stmt.query([]).expect("q");
                let mut found = 0;
                while rows.next().expect("r").is_some() {
                    found += 1;
                }
                micros.push(started.elapsed().as_micros() as u64);
                found_total += found;
            }
            micros.sort_unstable();
            emit(
                cfg,
                "query",
                &format!("{name}-{table}"),
                &[
                    ("p50_us", num(pct(&micros, 0.5))),
                    ("p99_us", num(pct(&micros, 0.99))),
                    ("found_per_query", num(found_total / sample.len())),
                    ("plan", format!("\"{}\"", plan.replace('"', "'"))),
                ],
            );
        }
    }
    // The product index has detail='none'; compare detail='full' and 'column'.
    for detail in ["column", "full"] {
        let table = format!("node_fts_{detail}");
        conn.execute_batch(&format!(
            "CREATE VIRTUAL TABLE {table} USING fts5(name, content='node', content_rowid='id', \
             tokenize='trigram case_sensitive 0 remove_diacritics 1', detail='{detail}'); \
             INSERT INTO {table}({table}) VALUES ('rebuild');"
        ))
        .expect("fts variant");
        let stat = dbstat_by_name(&conn);
        let bytes: i64 = stat.iter().filter(|(n, _)| n.starts_with(&table)).map(|(_, b)| *b).sum();
        for (name, len) in [("3", 3_usize), ("5", 5), ("10", 10)] {
            for (mode, template) in [("like", "name LIKE '%NEEDLE%'"), ("match", "name MATCH '\"NEEDLE\"'")] {
                let mut micros = Vec::new();
                let mut found_total = 0;
                let mut failed = false;
                let mut plan = String::new();
                for full in &sample {
                    let token = full.rsplit('-').next().unwrap_or(full);
                    let needle = &token[..len.min(token.len())];
                    let sql = format!(
                        "SELECT rowid FROM {table} WHERE {} LIMIT 200",
                        template.replace("NEEDLE", needle)
                    );
                    if plan.is_empty() {
                        plan = conn
                            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
                            .and_then(|mut st| st.query_row([], |r| r.get::<_, String>(3)))
                            .unwrap_or_default();
                    }
                    let started = Instant::now();
                    // Errors (an FTS5 phrase with detail != full) surface as items of the
                    // iterator, so collect them instead of counting them as rows.
                    let result = conn.prepare(&sql).and_then(|mut st| {
                        st.query_map([], |_| Ok(()))
                            .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
                            .map(|rows| rows.len())
                    });
                    match result {
                        Ok(n) => found_total += n,
                        Err(_) => failed = true,
                    }
                    micros.push(started.elapsed().as_micros() as u64);
                }
                micros.sort_unstable();
                emit(
                    cfg,
                    "query",
                    &format!("fts-{detail}-{mode}-{name}"),
                    &[
                        ("index_bytes", num(bytes)),
                        ("p50_us", num(pct(&micros, 0.5))),
                        ("p99_us", num(pct(&micros, 0.99))),
                        ("found_per_query", num(found_total / sample.len())),
                        ("error", num(failed)),
                        ("plan", format!("\"{}\"", plan.replace('"', "'"))),
                    ],
                );
            }
        }
    }
    drop(conn);
    // Q5: open cost on a warm OS cache (cold needs privileges and is not measured).
    let mut open_us = Vec::new();
    for _ in 0..5 {
        let started = Instant::now();
        let c = Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_ONLY).expect("ro");
        let _ = c
            .query_row("SELECT count(*) FROM (SELECT id FROM event ORDER BY id DESC LIMIT 200)", [], |r| r.get::<_, i64>(0))
            .expect("first page");
        open_us.push(started.elapsed().as_micros() as u64);
    }
    open_us.sort_unstable();
    emit(cfg, "query", "open-warm", &[("p50_us", num(pct(&open_us, 0.5))), ("max_us", num(open_us.last().copied().unwrap_or(0)))]);
    // Q10: sealing with VACUUM INTO.
    let conn = Connection::open(&db).expect("open");
    let sealed = cfg.dir.join("sealed.db");
    let _ = std::fs::remove_file(&sealed);
    let started = Instant::now();
    conn.execute("VACUUM INTO ?1", [sealed.to_string_lossy().as_ref()]).expect("vacuum into");
    emit(
        cfg,
        "query",
        "vacuum-into",
        &[("seconds", num(started.elapsed().as_secs_f64())), ("before_bytes", num(file_size(&db))), ("after_bytes", num(file_size(&sealed)))],
    );
    drop(conn);
    let _ = std::fs::remove_file(&sealed);
    remove_db(&db);
}

/// Q6: a read-only viewer connection while the writer is hot.
fn readers(cfg: &Config) {
    let db = cfg.dir.join("readers.db");
    for (name, mmap, pin_snapshot, reader) in [
        ("no-reader", 0_i64, false, false),
        ("reader-mmap0", 0, false, true),
        ("reader-mmap256m", 256 * 1024 * 1024, false, true),
        ("reader-pins-snapshot", 0, true, true),
    ] {
        let writer = open_writer(&db, 4096, 2_000, 1000);
        let mut rng = Rng(3);
        let nodes = build_nodes(&writer, 20_000, &mut rng);
        fill_events(&writer, &nodes, 200_000, &mut rng);
        let stop = Arc::new(AtomicBool::new(false));
        let reads = Arc::new(AtomicU64::new(0));
        let micros = Arc::new(std::sync::Mutex::new(Vec::<u64>::new()));
        let handle = reader.then(|| {
            let (stop, reads, micros, db) = (Arc::clone(&stop), Arc::clone(&reads), Arc::clone(&micros), db.clone());
            thread::spawn(move || {
                let conn = Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_ONLY).expect("ro");
                conn.pragma_update(None, "mmap_size", mmap).expect("mmap");
                if pin_snapshot {
                    conn.execute_batch("BEGIN").expect("begin");
                    let _ = conn.query_row("SELECT count(*) FROM (SELECT id FROM event LIMIT 1)", [], |r| r.get::<_, i64>(0));
                }
                let mut rng = Rng(9);
                while !stop.load(Ordering::Relaxed) {
                    let anchor = 1 + rng.below(200_000) as i64;
                    let started = Instant::now();
                    let _ = conn
                        .prepare_cached("SELECT * FROM event WHERE id < ?1 ORDER BY id DESC LIMIT 200")
                        .and_then(|mut s| s.query_map([anchor], |_| Ok(())).map(|rows| rows.count()));
                    micros.lock().expect("lock").push(started.elapsed().as_micros() as u64);
                    reads.fetch_add(1, Ordering::Relaxed);
                    if pin_snapshot {
                        thread::sleep(Duration::from_millis(5));
                    }
                }
            })
        });
        let wal = wal_path(&db);
        let mut peak_wal = 0;
        let started = Instant::now();
        let mut id = 200_000_u64;
        let target = 600_000_u64;
        while id < target {
            writer.execute_batch("BEGIN").expect("begin");
            {
                let mut stmt = writer.prepare_cached(INSERT_EVENT).expect("prepare");
                for _ in 0..1000 {
                    id += 1;
                    let node = nodes.get(rng.below(nodes.len() as u64) as usize).copied().unwrap_or(1);
                    insert_one(&mut stmt, id, node, &mut rng);
                }
            }
            writer.execute_batch("COMMIT").expect("commit");
            peak_wal = peak_wal.max(file_size(&wal));
        }
        let elapsed = started.elapsed();
        stop.store(true, Ordering::Relaxed);
        if let Some(h) = handle {
            let _ = h.join();
        }
        let mut m = micros.lock().expect("lock").clone();
        m.sort_unstable();
        emit(
            cfg,
            "readers",
            name,
            &[
                ("writer_events_per_s", num(((target - 200_000) as f64 / elapsed.as_secs_f64()) as u64)),
                ("reader_pages", num(reads.load(Ordering::Relaxed))),
                ("reader_p50_us", num(pct(&m, 0.5))),
                ("reader_p99_us", num(pct(&m, 0.99))),
                ("peak_wal_bytes", num(peak_wal)),
            ],
        );
        drop(writer);
    }
    remove_db(&db);
}

/// Q9: memory at two cache sizes and Rust-side allocations per batch.
fn memory(cfg: &Config) {
    let db = cfg.dir.join("memory.db");
    // One cache size per process: freed pages are not returned to the OS, so a
    // later cell would start from the earlier cell's high-water mark.
    {
        let cache_kib = cfg.cache_kib;
        let before = private_bytes();
        let conn = open_writer(&db, 4096, cache_kib, 1000);
        let mut rng = Rng(21);
        let nodes = build_nodes(&conn, 50_000, &mut rng);
        fill_events(&conn, &nodes, 1_000_000, &mut rng);
        let after_burst = private_bytes();
        conn.pragma_update(None, "wal_checkpoint", "TRUNCATE").ok();
        let _ = conn.query_row("PRAGMA shrink_memory", [], |_| Ok(()));
        let after_shrink = private_bytes();
        emit(
            cfg,
            "memory",
            &format!("cache{cache_kib}k"),
            &[
                ("private_before", num(before)),
                ("private_after_burst", num(after_burst)),
                ("private_after_shrink", num(after_shrink)),
                ("delta_after_burst", num(after_burst.saturating_sub(before))),
            ],
        );
        {
            // Rust-side allocations per 1,000-event batch, warm.
            let mut id = 1_000_000_u64;
            let delta = measure(5, 20, || {
                conn.execute_batch("BEGIN").expect("begin");
                {
                    let mut stmt = conn.prepare_cached(INSERT_EVENT).expect("prepare");
                    for _ in 0..1000 {
                        id += 1;
                        let node = nodes.get((id % nodes.len() as u64) as usize).copied().unwrap_or(1);
                        insert_one(&mut stmt, id, node, &mut Rng(id));
                    }
                }
                conn.execute_batch("COMMIT").expect("commit");
            });
            emit(
                cfg,
                "memory",
                "allocs-per-1000-batch",
                &[("allocs", num(delta.allocs / 20)), ("bytes", num(delta.bytes / 20))],
            );
        }
        drop(conn);
    }
    remove_db(&db);
}

fn main() {
    let mut args = std::env::args().skip(1);
    let experiment = args.next().unwrap_or_else(|| "all".to_owned());
    let mut cfg = Config {
        rows: 1_000_000,
        dir: std::env::temp_dir(),
        reps: 3,
        label: std::env::var("BENCH_LABEL").unwrap_or_else(|_| "local".to_owned()),
        cache_kib: 2_000,
    };
    while let Some(flag) = args.next() {
        let value = args.next().unwrap_or_default();
        match flag.as_str() {
            "--rows" => cfg.rows = value.parse().unwrap_or(cfg.rows),
            "--dir" => cfg.dir = PathBuf::from(value),
            "--reps" => cfg.reps = value.parse().unwrap_or(cfg.reps),
            "--label" => cfg.label = value,
            "--cache-kib" => cfg.cache_kib = value.parse().unwrap_or(cfg.cache_kib),
            _ => {}
        }
    }
    let _ = std::fs::create_dir_all(&cfg.dir);
    emit(
        &cfg,
        "env",
        "sqlite",
        &[("version", format!("\"{}\"", rusqlite::version())), ("cpus", num(thread::available_parallelism().map_or(0, usize::from)))],
    );
    match experiment.as_str() {
        "commit" => commit(&cfg),
        "checkpoint" => checkpoint(&cfg),
        "size" => size(&cfg),
        "query" => query(&cfg),
        "readers" => readers(&cfg),
        "memory" => memory(&cfg),
        _ => {
            size(&cfg);
            checkpoint(&cfg);
            commit(&cfg);
            readers(&cfg);
            memory(&cfg);
            query(&cfg);
        }
    }
}
