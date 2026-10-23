//! The Win32 calls behind [`super`].

use std::os::windows::io::{AsRawHandle as _, BorrowedHandle, FromRawHandle as _, OwnedHandle};

use windows_sys::Wdk::System::SystemInformation::{
    NtQuerySystemInformation, SystemProcessInformation,
};
use windows_sys::Win32::Foundation::{FILETIME, STATUS_INFO_LENGTH_MISMATCH};
use windows_sys::Win32::System::ProcessStatus::{
    K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
};
use windows_sys::Win32::System::Threading::{
    GR_GDIOBJECTS, GR_USEROBJECTS, GetCurrentProcess, GetGuiResources, GetProcessHandleCount,
    GetProcessIoCounters, GetProcessTimes, IO_COUNTERS, IsWow64Process2, OpenProcess,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::System::WindowsProgramming::{
    QueryProcessCycleTime, SYSTEM_PROCESS_INFORMATION, SYSTEM_THREAD_INFORMATION,
};

use super::process_list::{
    PROCESS_ENTRY_SIZE, PROCESS_ID_OFFSET, PROCESS_THREADS_OFFSET, THREAD_CONTEXT_SWITCHES_OFFSET,
    THREAD_ENTRY_SIZE,
};
use super::{Arch, CpuTimes, GuiObjects, Io, Memory, filetime_ticks};

// The byte parser in `process_list` must match the real structs.
const _: () = {
    assert!(size_of::<SYSTEM_PROCESS_INFORMATION>() == PROCESS_ENTRY_SIZE);
    assert!(
        std::mem::offset_of!(SYSTEM_PROCESS_INFORMATION, NumberOfThreads) == PROCESS_THREADS_OFFSET
    );
    assert!(std::mem::offset_of!(SYSTEM_PROCESS_INFORMATION, UniqueProcessId) == PROCESS_ID_OFFSET);
    assert!(size_of::<SYSTEM_THREAD_INFORMATION>() == THREAD_ENTRY_SIZE);
    assert!(
        std::mem::offset_of!(SYSTEM_THREAD_INFORMATION, Reserved3)
            == THREAD_CONTEXT_SWITCHES_OFFSET
    );
};

/// Widens a `usize` byte count.
fn bytes(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn check(ok: i32) -> std::io::Result<()> {
    if ok == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// The pseudo-handle of the current process (needs no closing).
#[must_use]
pub fn current_process() -> BorrowedHandle<'static> {
    // SAFETY: GetCurrentProcess has no preconditions.
    let raw = unsafe { GetCurrentProcess() };
    // SAFETY: the pseudo-handle is valid for the whole life of the process;
    // `BorrowedHandle` explicitly allows it.
    unsafe { BorrowedHandle::borrow_raw(raw) }
}

/// Opens process `pid` with `PROCESS_QUERY_LIMITED_INFORMATION`, enough for
/// every reader in this module.
///
/// # Errors
///
/// Returns the OS error, for example when the process does not exist.
pub fn open_process(pid: u32) -> std::io::Result<OwnedHandle> {
    // SAFETY: plain call; a null result is handled below.
    let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if raw.is_null() {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: OpenProcess succeeded, so `raw` is an owned process handle.
    Ok(unsafe { OwnedHandle::from_raw_handle(raw) })
}

/// Private Bytes, Working Set and page faults (`K32GetProcessMemoryInfo`).
///
/// # Errors
///
/// Returns the OS error when the counters cannot be read.
pub fn memory(process: BorrowedHandle<'_>) -> std::io::Result<Memory> {
    let mut counters = PROCESS_MEMORY_COUNTERS_EX::default();
    let size = u32::try_from(size_of::<PROCESS_MEMORY_COUNTERS_EX>()).unwrap_or(u32::MAX);
    // SAFETY: `counters` is a writable PROCESS_MEMORY_COUNTERS_EX of `size`
    // bytes; the EX struct starts with the base struct.
    check(unsafe {
        K32GetProcessMemoryInfo(
            process.as_raw_handle(),
            (&raw mut counters).cast::<PROCESS_MEMORY_COUNTERS>(),
            size,
        )
    })?;
    Ok(Memory {
        private_bytes: bytes(counters.PrivateUsage),
        working_set: bytes(counters.WorkingSetSize),
        peak_working_set: bytes(counters.PeakWorkingSetSize),
        page_faults: counters.PageFaultCount,
    })
}

/// Creation, kernel and user time (`GetProcessTimes`).
///
/// # Errors
///
/// Returns the OS error when the times cannot be read.
pub fn cpu_times(process: BorrowedHandle<'_>) -> std::io::Result<CpuTimes> {
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: all out pointers are valid FILETIMEs on the stack.
    check(unsafe {
        GetProcessTimes(
            process.as_raw_handle(),
            &raw mut creation,
            &raw mut exit,
            &raw mut kernel,
            &raw mut user,
        )
    })?;
    let join = |time: FILETIME| filetime_ticks(time.dwLowDateTime, time.dwHighDateTime);
    Ok(CpuTimes {
        creation: join(creation),
        kernel: join(kernel),
        user: join(user),
    })
}

/// CPU cycles of all threads (`QueryProcessCycleTime`). Only comparable on
/// the same machine in the same frequency state.
///
/// # Errors
///
/// Returns the OS error when the cycle time cannot be read.
pub fn cycle_time(process: BorrowedHandle<'_>) -> std::io::Result<u64> {
    let mut cycles = 0u64;
    // SAFETY: `cycles` is a valid out pointer.
    check(unsafe { QueryProcessCycleTime(process.as_raw_handle(), &raw mut cycles) })?;
    Ok(cycles)
}

/// Number of open kernel handles (`GetProcessHandleCount`).
///
/// # Errors
///
/// Returns the OS error when the count cannot be read.
pub fn handle_count(process: BorrowedHandle<'_>) -> std::io::Result<u32> {
    let mut count = 0u32;
    // SAFETY: `count` is a valid out pointer.
    check(unsafe { GetProcessHandleCount(process.as_raw_handle(), &raw mut count) })?;
    Ok(count)
}

/// GDI and USER objects (`GetGuiResources`). A process without a GUI
/// thread reports zero for both.
#[must_use]
pub fn gui_objects(process: BorrowedHandle<'_>) -> GuiObjects {
    // SAFETY: plain calls on a valid process handle.
    let gdi = unsafe { GetGuiResources(process.as_raw_handle(), GR_GDIOBJECTS) };
    // SAFETY: as above.
    let user = unsafe { GetGuiResources(process.as_raw_handle(), GR_USEROBJECTS) };
    GuiObjects { gdi, user }
}

/// Read, write and other I/O since start (`GetProcessIoCounters`).
///
/// # Errors
///
/// Returns the OS error when the counters cannot be read.
pub fn io(process: BorrowedHandle<'_>) -> std::io::Result<Io> {
    let mut counters = IO_COUNTERS::default();
    // SAFETY: `counters` is a valid out pointer.
    check(unsafe { GetProcessIoCounters(process.as_raw_handle(), &raw mut counters) })?;
    Ok(Io {
        read_bytes: counters.ReadTransferCount,
        write_bytes: counters.WriteTransferCount,
        other_bytes: counters.OtherTransferCount,
        read_ops: counters.ReadOperationCount,
        write_ops: counters.WriteOperationCount,
        other_ops: counters.OtherOperationCount,
    })
}

/// The process's and the machine's architecture (`IsWow64Process2`).
///
/// # Errors
///
/// Returns the OS error when the architecture cannot be read.
pub fn arch(process: BorrowedHandle<'_>) -> std::io::Result<Arch> {
    let mut own = 0u16;
    let mut native = 0u16;
    // SAFETY: both out pointers are valid.
    check(unsafe { IsWow64Process2(process.as_raw_handle(), &raw mut own, &raw mut native) })?;
    Ok(Arch {
        process: own,
        native,
    })
}

/// Samples context switches of one process from the system process list.
///
/// `NtQuerySystemInformation` is documented as subject to change, so a
/// failure is reported as "unavailable" (`None`), never as an error. The
/// buffer is kept between samples: after the first sample that fits, a
/// sample allocates nothing (#23).
#[derive(Debug, Default)]
pub struct ContextSwitches {
    /// `u64` elements keep the buffer 8-byte aligned for the kernel.
    buffer: Vec<u64>,
}

impl ContextSwitches {
    /// Creates a sampler with no buffer yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Context switches summed over all threads of `pid`, or `None` when the
    /// list cannot be read or does not contain `pid`.
    pub fn sample(&mut self, pid: u32) -> Option<u64> {
        // Grow and retry: the list changes between the size query and the
        // read, so a few rounds may be needed.
        for _ in 0..8 {
            let capacity = self.buffer.len().checked_mul(8)?;
            let length = u32::try_from(capacity).ok()?;
            let mut needed = 0u32;
            // SAFETY: `buffer` is writable for `length` bytes (or empty with
            // length 0); `needed` is a valid out pointer.
            let status = unsafe {
                NtQuerySystemInformation(
                    SystemProcessInformation,
                    self.buffer.as_mut_ptr().cast(),
                    length,
                    &raw mut needed,
                )
            };
            if status == STATUS_INFO_LENGTH_MISMATCH {
                // Headroom for processes started before the next call.
                let words = usize::try_from(needed)
                    .ok()?
                    .div_ceil(8)
                    .checked_add(8192)?;
                self.buffer.resize(words, 0);
                continue;
            }
            // Success is STATUS_SUCCESS (0); any other status is treated as
            // "unavailable".
            if status != 0 {
                return None;
            }
            // SAFETY: the buffer holds `capacity` initialised bytes and stays
            // borrowed for the slice's lifetime.
            let bytes =
                unsafe { std::slice::from_raw_parts(self.buffer.as_ptr().cast::<u8>(), capacity) };
            // On success `needed` is the number of bytes written.
            let filled = bytes.get(..usize::try_from(needed).ok()?)?;
            return super::process_list::context_switches(filled, pid);
        }
        None
    }
}
