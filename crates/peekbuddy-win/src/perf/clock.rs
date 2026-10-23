//! Microseconds since this process was created (F7: startup is measured from
//! process creation, not from `main`).

use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::System::SystemInformation::GetSystemTimePreciseAsFileTime;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

fn ticks(time: FILETIME) -> u64 {
    (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
}

/// Microseconds between process creation and now, in system time (100 ns
/// FILETIME ticks / 10). Returns 0 if the creation time cannot be read.
#[must_use]
pub fn since_process_create_us() -> u64 {
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: GetCurrentProcess returns a pseudo-handle that needs no closing.
    let process = unsafe { GetCurrentProcess() };
    // SAFETY: all out pointers are valid FILETIMEs on the stack.
    let ok = unsafe {
        GetProcessTimes(
            process,
            &raw mut creation,
            &raw mut exit,
            &raw mut kernel,
            &raw mut user,
        )
    };
    let mut now = FILETIME::default();
    // SAFETY: `now` is a valid out pointer.
    unsafe { GetSystemTimePreciseAsFileTime(&raw mut now) };
    if ok == 0 {
        return 0;
    }
    ticks(now).saturating_sub(ticks(creation)) / 10
}

#[cfg(test)]
mod tests {
    use super::since_process_create_us;

    #[test]
    fn is_positive_and_monotonic() {
        let first = since_process_create_us();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let second = since_process_create_us();
        assert!(first > 0);
        assert!(second >= first + 4_000, "{first} -> {second}");
        // A test process is not hours old.
        assert!(second < 3_600_000_000);
    }
}
