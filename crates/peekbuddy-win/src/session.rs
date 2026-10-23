//! The Windows session this process runs in.

use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows_sys::Win32::System::Threading::GetCurrentProcessId;

/// Returns the Terminal Services session id of the current process.
///
/// Session 0 is the non-interactive services session; tray and window tests
/// need a session above 0.
///
/// # Errors
///
/// Returns the OS error when the lookup fails.
pub fn current_session_id() -> std::io::Result<u32> {
    let mut id = 0u32;
    // SAFETY: GetCurrentProcessId has no preconditions.
    let pid = unsafe { GetCurrentProcessId() };
    // SAFETY: `id` is a valid, writable u32 for the duration of the call.
    let ok = unsafe { ProcessIdToSessionId(pid, &raw mut id) };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(id)
}
