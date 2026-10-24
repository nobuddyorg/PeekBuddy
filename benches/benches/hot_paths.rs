//! Hot-path micro-benchmarks (#26). Every id here has a `bench.*` row in
//! `perf/budgets.toml`; `cargo xtask bench-ab` compares base and head.

use std::hint::black_box;

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use peekbuddy_core::event::{Event, Flags, Kind, NameKind, NameRef, RootIdx, Source, VolIdx};
use peekbuddy_core::filter::{CompileOptions, FilterSet, Pattern, Rule, Scratch, presets};
use peekbuddy_core::name::{CaseMode, WideStr};
use peekbuddy_core::notify::{Class, layout, parse};
use peekbuddy_core::perf::{CounterSnapshot, Milestone, MilestoneTable, Role};
use peekbuddy_store::interner::{Interner, NodeKey, TOP_LEVEL};
use peekbuddy_store::schema::{self, INSERT_EVENT};
use peekbuddy_win::metrics::process_list::{
    PROCESS_ENTRY_SIZE, PROCESS_ID_OFFSET, PROCESS_THREADS_OFFSET, THREAD_CONTEXT_SWITCHES_OFFSET,
    THREAD_ENTRY_SIZE, context_switches,
};
use peekbuddy_win::perf::{etw, record};
use rusqlite::{Connection, params};

static TABLE: MilestoneTable = MilestoneTable::new();

/// Milestone record plus its ETW event, with no trace session listening
/// (#21): the cost every milestone adds to a normal run.
fn perf(c: &mut Criterion) {
    let _registration = etw::register();
    let mut group = c.benchmark_group("perf");
    group.throughput(Throughput::Elements(1));
    let mut t = 0u64;
    group.bench_function("milestone_record", |b| {
        b.iter(|| {
            t += 1;
            record(&TABLE, black_box(Milestone::WatcherArmed), Role::Engine, t);
        });
    });
    let snapshot = CounterSnapshot::default();
    group.bench_function("counters_event", |b| {
        b.iter(|| etw::counters(black_box(&snapshot)));
    });
    group.finish();
}

/// A `SystemProcessInformation` buffer of `processes` processes with
/// `threads` threads each; pids are 4, 8, 12, ...
fn process_list(processes: u32, threads: u32) -> Vec<u8> {
    let threads_usize = usize::try_from(threads).unwrap_or(0);
    let size = PROCESS_ENTRY_SIZE + threads_usize * THREAD_ENTRY_SIZE;
    let mut out = vec![0u8; size * usize::try_from(processes).unwrap_or(0)];
    for (index, entry) in out.chunks_mut(size).enumerate() {
        let last = index + 1 == usize::try_from(processes).unwrap_or(0);
        let next = if last { 0 } else { size };
        let pid = u64::try_from(index + 1).unwrap_or(0) * 4;
        let put = |entry: &mut [u8], at: usize, bytes: &[u8]| {
            if let Some(slot) = entry.get_mut(at..at + bytes.len()) {
                slot.copy_from_slice(bytes);
            }
        };
        put(entry, 0, &u32::try_from(next).unwrap_or(0).to_le_bytes());
        put(entry, PROCESS_THREADS_OFFSET, &threads.to_le_bytes());
        put(entry, PROCESS_ID_OFFSET, &pid.to_le_bytes());
        for thread in 0..threads_usize {
            let at =
                PROCESS_ENTRY_SIZE + thread * THREAD_ENTRY_SIZE + THREAD_CONTEXT_SWITCHES_OFFSET;
            put(entry, at, &7u32.to_le_bytes());
        }
    }
    out
}

/// One context-switch sample walks the whole process list (#24); a busy
/// machine has a few hundred processes.
fn metrics(c: &mut Criterion) {
    let buffer = process_list(300, 20);
    let mut group = c.benchmark_group("metrics");
    group.throughput(Throughput::Bytes(
        u64::try_from(buffer.len()).unwrap_or(u64::MAX),
    ));
    group.bench_function("process_list_300", |b| {
        // The last process: the walk visits every entry.
        b.iter(|| context_switches(black_box(&buffer), black_box(1200)));
    });
    group.finish();
}

/// A 64 KiB `ReadDirectoryChanges*` buffer of records with 8 to 64 unit
/// names (#44).
fn notify_buffer(class: Class) -> Vec<u8> {
    const SIZE: usize = 64 * 1024;
    let header = class.header_len();
    let name_field = match class {
        Class::Plain => layout::PLAIN_NAME_LENGTH,
        Class::Extended | Class::Full => layout::EXTENDED_NAME_LENGTH,
    };
    let mut buf = vec![0u8; SIZE];
    let (mut at, mut count, mut last) = (0usize, 0usize, 0usize);
    loop {
        let units = 8 + (count * 7) % 57;
        let size = (header + units * 2).next_multiple_of(4);
        if at + size > SIZE {
            break;
        }
        let put = |buf: &mut Vec<u8>, offset: usize, bytes: &[u8]| {
            if let Some(slot) = buf.get_mut(at + offset..at + offset + bytes.len()) {
                slot.copy_from_slice(bytes);
            }
        };
        put(
            &mut buf,
            layout::NEXT_ENTRY_OFFSET,
            &u32::try_from(size).unwrap_or(0).to_le_bytes(),
        );
        put(&mut buf, layout::ACTION, &3u32.to_le_bytes());
        put(
            &mut buf,
            name_field,
            &u32::try_from(units * 2).unwrap_or(0).to_le_bytes(),
        );
        for unit in 0..units {
            put(
                &mut buf,
                header + unit * 2,
                &(0x61u16 + u16::try_from(unit % 26).unwrap_or(0)).to_le_bytes(),
            );
        }
        last = at;
        at += size;
        count += 1;
    }
    if let Some(slot) = buf.get_mut(last..last + 4) {
        slot.fill(0);
    }
    buf
}

/// Zero-copy parse of a 64 KiB buffer, summing name units so the work is
/// not optimised away (#44).
fn notify_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("notify_parse");
    for (name, class) in [
        ("plain", Class::Plain),
        ("extended", Class::Extended),
        ("full", Class::Full),
    ] {
        let buf = notify_buffer(class);
        group.throughput(Throughput::Bytes(
            u64::try_from(buf.len()).unwrap_or(u64::MAX),
        ));
        group.bench_with_input(BenchmarkId::new(name, "64KiB"), &buf, |b, buf| {
            let mut units = [0u16; 128];
            b.iter(|| {
                let mut total = 0usize;
                for record in parse(black_box(buf), buf.len(), class).flatten() {
                    total += record.name.copy_into(&mut units).unwrap_or(0);
                }
                total
            });
        });
    }
    group.finish();
}

fn filter_event(is_dir: bool) -> Event {
    Event {
        ts: 1,
        qpc: 1,
        src_ts: 0,
        source: Source::Rdcw,
        kind: Kind::Modified,
        raw_mask: 3,
        source_info: 0,
        is_dir,
        size: 0,
        attrs: 0,
        reparse_tag: 0,
        file_id: 1,
        parent_file_id: 1,
        usn: 0,
        vol: VolIdx(0),
        root: RootIdx(0),
        name: NameRef::EMPTY,
        name_kind: NameKind::Long,
        flags: Flags::NONE,
    }
}

/// One filter verdict over a name set that exercises the glob, extension and
/// regex matchers (#40).
fn filter(c: &mut Criterion) {
    let mut rules: Vec<Rule> = presets::all()
        .unwrap_or_default()
        .iter()
        .flat_map(|p| p.rules(peekbuddy_core::filter::Scope::Global))
        .collect();
    rules.push(Rule::exclude(Pattern::Regex(r"(?i)\.bak\d*$".into())));
    let Ok(set) = FilterSet::compile_with(
        &rules,
        &CompileOptions {
            default_case: CaseMode::Insensitive,
            overrides: Vec::new(),
        },
    ) else {
        return;
    };
    let names: Vec<Vec<u16>> = [
        "src\\main.rs",
        "node_modules\\x\\index.js",
        "Docs\\~$draft.docx",
        "crash.dmp",
        "backup.bak12",
        "a\\b\\c\\d\\e.txt",
    ]
    .iter()
    .map(|n| n.encode_utf16().collect())
    .collect();
    let event = filter_event(false);
    let mut scratch = Scratch::new();
    let mut group = c.benchmark_group("filter");
    group.throughput(Throughput::Elements(
        u64::try_from(names.len()).unwrap_or(1),
    ));
    group.bench_function("eval_6_names", |b| {
        b.iter(|| {
            let mut hidden = 0u32;
            for name in &names {
                hidden += u32::from(
                    !set.eval(black_box(&event), WideStr::new(name), &mut scratch)
                        .visible,
                );
            }
            hidden
        });
    });
    group.finish();
}

fn memory_store() -> Option<Connection> {
    let mut conn = Connection::open_in_memory().ok()?;
    schema::migrate(&mut conn).ok()?;
    Some(conn)
}

fn node_names(count: usize) -> Vec<Vec<u16>> {
    (0..count)
        .map(|i| format!("directory-{i:06}.txt").encode_utf16().collect())
        .collect()
}

/// The writer's two hot operations on an in-memory segment (#53, #52): the
/// interner hit path and a 1,000-event transaction.
fn store(c: &mut Criterion) {
    let Some(conn) = memory_store() else { return };
    let names = node_names(1_000);
    let mut interner = Interner::new();
    let mut ids = Vec::new();
    for name in &names {
        let id = interner
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
            .unwrap_or(0);
        ids.push(id);
    }
    let mut group = c.benchmark_group("store");
    group.throughput(Throughput::Elements(
        u64::try_from(names.len()).unwrap_or(1),
    ));
    group.bench_function("intern_hit_1000", |b| {
        b.iter(|| {
            let mut sum = 0i64;
            for name in &names {
                sum += interner
                    .lookup(TOP_LEVEL, WideStr::new(black_box(name)))
                    .unwrap_or(0);
            }
            sum
        });
    });
    group.bench_function("intern_miss_1000", |b| {
        b.iter_batched(
            || memory_store().map(|conn| (conn, Interner::new())),
            |setup| {
                let Some((conn, mut interner)) = setup else {
                    return;
                };
                let _ = conn.execute_batch("BEGIN");
                for name in &names {
                    let _ = interner.intern(
                        &conn,
                        &NodeKey {
                            vol_id: 1,
                            parent_id: TOP_LEVEL,
                            name: WideStr::new(name),
                            name_kind: NameKind::Long,
                            file_id: None,
                        },
                    );
                }
                let _ = conn.execute_batch("COMMIT");
            },
            BatchSize::LargeInput,
        );
    });
    let mut next_id = 0i64;
    group.bench_function("batch_insert_1000", |b| {
        b.iter(|| {
            let _ = conn.execute_batch("BEGIN");
            if let Ok(mut stmt) = conn.prepare_cached(INSERT_EVENT) {
                for i in 0..1_000usize {
                    next_id += 1;
                    let node = ids.get(i).copied().unwrap_or(1);
                    let _ = stmt.execute(params![
                        next_id,
                        133_000_000_000_000_000i64 + next_id,
                        0,
                        0,
                        3,
                        node,
                        None::<i64>,
                        0,
                        4096,
                        0x20,
                        None::<i64>,
                        next_id,
                        None::<i64>,
                        None::<i64>,
                        1,
                        1,
                        0,
                        None::<i64>,
                        None::<i64>,
                    ]);
                }
            }
            let _ = conn.execute_batch("COMMIT");
        });
    });
    group.finish();
}

criterion_group!(benches, perf, metrics, notify_parse, filter, store);
criterion_main!(benches);
