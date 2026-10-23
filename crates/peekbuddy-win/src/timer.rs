//! High-resolution waitable timers for pacing (#25).
//!
//! `CREATE_WAITABLE_TIMER_HIGH_RESOLUTION` (Windows 10 1803 and later) waits
//! with sub-millisecond precision without `timeBeginPeriod`, so pacing never
//! raises the system timer rate.

use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};

use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
use windows_sys::Win32::System::Threading::{
    CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CreateWaitableTimerExW, INFINITE, SetWaitableTimer,
    TIMER_ALL_ACCESS, WaitForSingleObject,
};

/// A high-resolution one-shot timer owned by one thread.
#[derive(Debug)]
pub struct HighResTimer(OwnedHandle);

impl HighResTimer {
    /// Creates the timer.
    ///
    /// # Errors
    ///
    /// Returns the OS error, for example on Windows before 10 1803.
    pub fn new() -> std::io::Result<Self> {
        // SAFETY: no attributes, no name; a null result is handled below.
        let raw = unsafe {
            CreateWaitableTimerExW(
                std::ptr::null(),
                std::ptr::null(),
                CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                TIMER_ALL_ACCESS,
            )
        };
        if raw.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: CreateWaitableTimerExW succeeded, so `raw` is owned.
        Ok(Self(unsafe { OwnedHandle::from_raw_handle(raw) }))
    }

    /// Blocks for `ticks` of 100 ns.
    ///
    /// # Errors
    ///
    /// Returns the OS error when the timer cannot be set or waited on.
    pub fn sleep(&self, ticks: u64) -> std::io::Result<()> {
        // A negative due time is relative.
        let due = -i64::try_from(ticks).unwrap_or(i64::MAX);
        let raw = self.0.as_raw_handle();
        // SAFETY: `raw` is a live timer handle; `due` outlives the call; no
        // completion routine.
        let ok = unsafe { SetWaitableTimer(raw, &raw const due, 0, None, std::ptr::null(), 0) };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: `raw` is a live timer handle.
        if unsafe { WaitForSingleObject(raw, INFINITE) } == WAIT_OBJECT_0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }
}
