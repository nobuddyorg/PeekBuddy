//! A measurement of this process for the perf probe (#24, #161).

use peekbuddy_win::metrics::{
    ContextSwitches, cpu_times, current_process, gui_objects, handle_count, io, memory,
};
use peekbuddy_win::perf::since_process_create_us;

use crate::probe::Sample;

/// Samples the current process. Counters the OS cannot report read as 0;
/// `context_switches` is `None` when the system information is unavailable.
pub fn take(switches: &mut ContextSwitches) -> Sample {
    let process = current_process();
    let memory = memory(process).unwrap_or_default();
    let cpu = cpu_times(process).unwrap_or_default();
    let gui = gui_objects(process);
    let io = io(process).unwrap_or_default();
    Sample {
        at_us: since_process_create_us(),
        private_bytes: memory.private_bytes,
        working_set: memory.working_set,
        peak_working_set: memory.peak_working_set,
        page_faults: u64::from(memory.page_faults),
        kernel_ticks: cpu.kernel,
        user_ticks: cpu.user,
        handles: u64::from(handle_count(process).unwrap_or(0)),
        gdi_objects: u64::from(gui.gdi),
        user_objects: u64::from(gui.user),
        context_switches: switches.sample(std::process::id()),
        read_bytes: io.read_bytes,
        write_bytes: io.write_bytes,
        read_ops: io.read_ops,
        write_ops: io.write_ops,
    }
}

#[cfg(test)]
mod tests {
    use super::{ContextSwitches, take};

    #[test]
    fn a_sample_of_this_process_has_plausible_numbers() {
        let mut switches = ContextSwitches::new();
        let sample = take(&mut switches);
        assert!(sample.at_us > 0);
        assert!(sample.private_bytes > 0);
        assert!(sample.working_set > 0);
        assert!(sample.handles > 0);
        assert!(sample.user_ticks + sample.kernel_ticks > 0);
    }
}
