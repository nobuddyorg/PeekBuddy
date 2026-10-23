//! Milestone and ETW writes (#21, F10) and process-metric samples (#24)
//! allocate nothing. `harness = false`: libtest itself allocates.

use std::alloc::System;
use std::hint::black_box;
use std::process::ExitCode;

use peekbuddy_alloc::{CountingAlloc, answer_nextest_list, measure};
use peekbuddy_core::perf::{CounterSnapshot, Counters, Milestone, MilestoneTable, Role};
use peekbuddy_win::perf::{etw, record};

#[global_allocator]
static ALLOC: CountingAlloc<System> = CountingAlloc::new(System);

static TABLE: MilestoneTable = MilestoneTable::new();
static COUNTERS: Counters = Counters::new();

/// Allocations of 200 metric samples after the sampler's buffer is sized.
#[cfg(windows)]
fn metrics_allocs() -> u64 {
    use peekbuddy_win::metrics::{
        ContextSwitches, cpu_times, current_process, handle_count, io, memory,
    };
    let pid = std::process::id();
    let mut switches = ContextSwitches::new();
    let mut seen = 0u64;
    let delta = measure(5, 200, || {
        let process = current_process();
        seen = seen.wrapping_add(switches.sample(pid).unwrap_or(0));
        black_box(memory(process).ok());
        black_box(cpu_times(process).ok());
        black_box(handle_count(process).ok());
        black_box(io(process).ok());
    });
    // A sampler that never saw this process measured nothing.
    if seen == 0 { u64::MAX } else { delta.allocs }
}

#[cfg(not(windows))]
fn metrics_allocs() -> u64 {
    0
}

fn main() -> ExitCode {
    if answer_nextest_list("alloc_perf") {
        return ExitCode::SUCCESS;
    }
    let _registration = etw::register();
    let mut t = 0u64;
    let milestones = measure(1_000, 100_000, || {
        t += 1;
        record(&TABLE, black_box(Milestone::WatcherArmed), Role::Engine, t);
    });
    let counters = measure(1_000, 100_000, || {
        COUNTERS.observe_ring_depth(black_box(t));
        etw::counters(&black_box(COUNTERS.snapshot()));
    });
    let empty = CounterSnapshot::default();
    let snapshot_only = measure(10, 1_000, || {
        black_box(black_box(&empty).events_received);
    });
    let ok = milestones.allocs == 0
        && counters.allocs == 0
        && snapshot_only.allocs == 0
        && metrics_allocs() == 0;
    let mut err = std::io::stderr();
    if !ok {
        let _ = std::io::Write::write_all(&mut err, b"alloc_perf: perf writes allocated\n");
    }
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
