//! Milestone and ETW writes allocate nothing (#21, F10). `harness = false`:
//! libtest itself allocates.

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
    let ok = milestones.allocs == 0 && counters.allocs == 0 && snapshot_only.allocs == 0;
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
