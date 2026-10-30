//! Freezing a process for the tests (#50).
//!
//! The forced-overflow test needs the engine to stop reading while the churn
//! runs, so the watch buffer fills and the kernel reports an overflow. A
//! [`Suspended`] value suspends every thread of a process and resumes them
//! when it is dropped, so a failing test cannot leave a frozen engine behind.

use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};

use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
};
use windows_sys::Win32::System::Threading::{
    OpenThread, ResumeThread, SuspendThread, THREAD_SUSPEND_RESUME,
};

/// Every thread of a process, suspended until the value is dropped.
#[derive(Debug)]
pub struct Suspended {
    threads: Vec<OwnedHandle>,
}

impl Suspended {
    /// Threads that were suspended.
    #[must_use]
    pub fn threads(&self) -> usize {
        self.threads.len()
    }
}

impl Drop for Suspended {
    fn drop(&mut self) {
        for thread in &self.threads {
            // SAFETY: `thread` is an owned handle opened with
            // THREAD_SUSPEND_RESUME; resuming a thread this value suspended.
            unsafe { ResumeThread(thread.as_raw_handle()) };
        }
    }
}

/// Suspends all threads of process `pid`.
///
/// # Errors
///
/// The OS error when the thread snapshot cannot be taken.
pub fn suspend_process(pid: u32) -> std::io::Result<Suspended> {
    // SAFETY: plain call; the result is checked below.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: CreateToolhelp32Snapshot succeeded, so `snapshot` is an owned
    // handle.
    let snapshot = unsafe { OwnedHandle::from_raw_handle(snapshot) };

    let mut entry = THREADENTRY32 {
        dwSize: u32::try_from(size_of::<THREADENTRY32>()).unwrap_or(u32::MAX),
        ..THREADENTRY32::default()
    };
    let mut threads = Vec::new();
    // SAFETY: `entry` is a writable THREADENTRY32 with `dwSize` set.
    let mut more = unsafe { Thread32First(snapshot.as_raw_handle(), &raw mut entry) } != 0;
    while more {
        if entry.th32OwnerProcessID == pid {
            // SAFETY: plain call; a null result is skipped below.
            let raw = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
            if !raw.is_null() {
                // SAFETY: OpenThread succeeded, so `raw` is an owned handle.
                let thread = unsafe { OwnedHandle::from_raw_handle(raw) };
                // SAFETY: `thread` has THREAD_SUSPEND_RESUME access.
                if unsafe { SuspendThread(thread.as_raw_handle()) } != u32::MAX {
                    threads.push(thread);
                }
            }
        }
        // SAFETY: `entry` is a writable THREADENTRY32 with `dwSize` set.
        more = unsafe { Thread32Next(snapshot.as_raw_handle(), &raw mut entry) } != 0;
    }
    Ok(Suspended { threads })
}
