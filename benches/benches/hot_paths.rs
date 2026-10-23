//! Hot-path micro-benchmarks (#26). Every id here has a `bench.*` row in
//! `perf/budgets.toml`; `cargo xtask bench-ab` compares base and head.

use std::hint::black_box;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use peekbuddy_core::perf::{CounterSnapshot, Milestone, MilestoneTable, Role};
use peekbuddy_win::metrics::process_list::{
    PROCESS_ENTRY_SIZE, PROCESS_ID_OFFSET, PROCESS_THREADS_OFFSET, THREAD_CONTEXT_SWITCHES_OFFSET,
    THREAD_ENTRY_SIZE, context_switches,
};
use peekbuddy_win::perf::{etw, record};

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

criterion_group!(benches, perf, metrics);
criterion_main!(benches);
