//! Reads the `NtQuerySystemInformation(SystemProcessInformation)` buffer.
//!
//! Pure byte parsing, so it builds and tests on Linux and under Miri. The
//! layout is the 64-bit one (x64 and arm64); `metrics::context_switches`
//! checks these offsets against the `windows-sys` structs at compile time.
//! `windows-sys` names the per-thread context-switch count `Reserved3`.

/// Size of `SYSTEM_PROCESS_INFORMATION`; its thread array follows it.
pub const PROCESS_ENTRY_SIZE: usize = 0x100;
/// Offset of `SYSTEM_PROCESS_INFORMATION.NumberOfThreads`.
pub const PROCESS_THREADS_OFFSET: usize = 0x04;
/// Offset of `SYSTEM_PROCESS_INFORMATION.UniqueProcessId`.
pub const PROCESS_ID_OFFSET: usize = 0x50;
/// Size of `SYSTEM_THREAD_INFORMATION`.
pub const THREAD_ENTRY_SIZE: usize = 0x50;
/// Offset of `SYSTEM_THREAD_INFORMATION.ContextSwitches` (`Reserved3`).
pub const THREAD_CONTEXT_SWITCHES_OFFSET: usize = 0x40;

fn u32_at(buffer: &[u8], offset: usize) -> Option<u32> {
    let bytes = buffer.get(offset..offset.checked_add(4)?)?;
    Some(u32::from_le_bytes(bytes.try_into().ok()?))
}

fn u64_at(buffer: &[u8], offset: usize) -> Option<u64> {
    let bytes = buffer.get(offset..offset.checked_add(8)?)?;
    Some(u64::from_le_bytes(bytes.try_into().ok()?))
}

/// Sums the context switches of every thread of process `pid`.
///
/// Returns `None` when the process is not in the buffer or the buffer is
/// truncated.
#[must_use]
pub fn context_switches(buffer: &[u8], pid: u32) -> Option<u64> {
    let mut entry = 0usize;
    loop {
        let next = u32_at(buffer, entry)?;
        if u64_at(buffer, entry.checked_add(PROCESS_ID_OFFSET)?)? == u64::from(pid) {
            let threads = u32_at(buffer, entry.checked_add(PROCESS_THREADS_OFFSET)?)?;
            let first = entry.checked_add(PROCESS_ENTRY_SIZE)?;
            let mut total = 0u64;
            for thread in 0..usize::try_from(threads).ok()? {
                let at = thread
                    .checked_mul(THREAD_ENTRY_SIZE)?
                    .checked_add(first)?
                    .checked_add(THREAD_CONTEXT_SWITCHES_OFFSET)?;
                total = total.saturating_add(u64::from(u32_at(buffer, at)?));
            }
            return Some(total);
        }
        if next == 0 {
            return None;
        }
        entry = entry.checked_add(usize::try_from(next).ok()?)?;
    }
}

#[cfg(test)]
#[expect(
    clippy::indexing_slicing,
    reason = "tests build buffers of known size; a panic is a test failure"
)]
mod tests {
    use super::{
        PROCESS_ENTRY_SIZE, PROCESS_ID_OFFSET, PROCESS_THREADS_OFFSET,
        THREAD_CONTEXT_SWITCHES_OFFSET, THREAD_ENTRY_SIZE, context_switches,
    };

    /// Builds a buffer of processes `(pid, per-thread switches)`.
    fn buffer(processes: &[(u32, &[u32])]) -> Vec<u8> {
        let mut out = Vec::new();
        for (index, (pid, threads)) in processes.iter().enumerate() {
            let start = out.len();
            let size = PROCESS_ENTRY_SIZE + threads.len() * THREAD_ENTRY_SIZE;
            out.resize(start + size, 0xAA);
            let next = if index + 1 == processes.len() {
                0
            } else {
                size
            };
            out[start..start + 4].copy_from_slice(&u32::try_from(next).unwrap().to_le_bytes());
            let count = u32::try_from(threads.len()).unwrap().to_le_bytes();
            out[start + PROCESS_THREADS_OFFSET..][..4].copy_from_slice(&count);
            out[start + PROCESS_ID_OFFSET..][..8].copy_from_slice(&u64::from(*pid).to_le_bytes());
            for (thread, switches) in threads.iter().enumerate() {
                let at = start
                    + PROCESS_ENTRY_SIZE
                    + thread * THREAD_ENTRY_SIZE
                    + THREAD_CONTEXT_SWITCHES_OFFSET;
                out[at..at + 4].copy_from_slice(&switches.to_le_bytes());
            }
        }
        out
    }

    #[test]
    fn sums_the_threads_of_the_wanted_process() {
        let list = buffer(&[(0, &[]), (4, &[10, 20]), (1234, &[1, 2, 3]), (99, &[7])]);
        assert_eq!(context_switches(&list, 1234), Some(6));
        assert_eq!(context_switches(&list, 4), Some(30));
        assert_eq!(context_switches(&list, 99), Some(7));
        assert_eq!(context_switches(&list, 0), Some(0));
    }

    #[test]
    fn a_missing_process_is_none() {
        let list = buffer(&[(4, &[1]), (8, &[2])]);
        assert_eq!(context_switches(&list, 5), None);
    }

    #[test]
    fn a_truncated_buffer_is_none() {
        let list = buffer(&[(4, &[1]), (8, &[2, 3])]);
        // Cut into the last thread's counter (the bytes after it are padding).
        let cut = list.len() - (THREAD_ENTRY_SIZE - THREAD_CONTEXT_SWITCHES_OFFSET) + 3;
        assert_eq!(context_switches(&list[..cut], 8), None);
        assert_eq!(context_switches(&list[..=cut], 8), Some(5));
        assert_eq!(context_switches(&[], 8), None);
        assert_eq!(context_switches(&list[..3], 4), None);
    }

    #[test]
    fn large_counts_do_not_overflow() {
        let list = buffer(&[(4, &[u32::MAX, u32::MAX])]);
        assert_eq!(context_switches(&list, 4), Some(2 * u64::from(u32::MAX)));
    }
}
