//! ETW smoke test for CI (#21): registers the provider, records the engine
//! milestones and one counter event, then exits. CI records it with
//! `perf/peekbuddy.wprp` and checks the ETL for the events.

use peekbuddy_core::perf::{CounterSnapshot, Milestone, MilestoneTable, Role};
use peekbuddy_win::perf::{etw, record};

#[cfg(windows)]
fn now(_step: u64) -> u64 {
    peekbuddy_win::perf::since_process_create_us()
}

#[cfg(not(windows))]
fn now(step: u64) -> u64 {
    step
}

fn main() {
    let _registration = etw::register();
    let table = MilestoneTable::new();
    for (step, milestone) in (1..).zip([
        Milestone::ProcessStart,
        Milestone::SettingsLoaded,
        Milestone::StoreOpened,
        Milestone::WatcherArmed,
        Milestone::TrayShown,
        Milestone::EngineReady,
    ]) {
        record(&table, milestone, Role::Engine, now(step));
    }
    etw::counters(&CounterSnapshot {
        events_received: 1,
        rows_persisted: 1,
        ..CounterSnapshot::default()
    });
}
