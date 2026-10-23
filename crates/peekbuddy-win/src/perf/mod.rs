//! Performance instrumentation: ETW milestones and counters (#21).

pub mod etw;

#[cfg(windows)]
mod clock;

#[cfg(windows)]
pub use clock::since_process_create_us;

use peekbuddy_core::perf::{Milestone, MilestoneTable, Role};

/// Records `milestone` in `table` (read by `--perf-probe`, #24) and writes it
/// to ETW, with the same timestamp in both.
pub fn record(
    table: &MilestoneTable,
    milestone: Milestone,
    role: Role,
    since_process_create_us: u64,
) {
    table.record(milestone, since_process_create_us);
    etw::milestone(milestone, role, since_process_create_us);
}

#[cfg(test)]
mod tests {
    use super::record;
    use peekbuddy_core::perf::{Milestone, MilestoneTable, Role};

    #[test]
    fn record_updates_the_table() {
        let table = MilestoneTable::new();
        record(&table, Milestone::EngineReady, Role::Engine, 42);
        assert_eq!(table.first_us(Milestone::EngineReady), Some(42));
        assert_eq!(table.count(Milestone::EngineReady), 1);
    }
}
