//! Ctrl+C for the headless engine.
//!
//! The exe uses the GUI subsystem, so it has no console of its own. In
//! headless mode it attaches to the parent's console (when there is one) and
//! turns Ctrl+C, Ctrl+Break and console close into `WM_CLOSE`, which takes
//! the one shutdown path.

use std::sync::atomic::{AtomicIsize, Ordering};

use windows_sys::Win32::Foundation::{HWND, TRUE};
use windows_sys::Win32::System::Console::{
    ATTACH_PARENT_PROCESS, AttachConsole, CTRL_BREAK_EVENT, CTRL_C_EVENT, CTRL_CLOSE_EVENT,
    SetConsoleCtrlHandler,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};

use crate::window::WindowHandle;

static TARGET: AtomicIsize = AtomicIsize::new(0);

unsafe extern "system" fn on_ctrl(kind: u32) -> i32 {
    if matches!(kind, CTRL_C_EVENT | CTRL_BREAK_EVENT | CTRL_CLOSE_EVENT) {
        let hwnd = TARGET.load(Ordering::Acquire) as HWND;
        if !hwnd.is_null() {
            // SAFETY: PostMessageW is thread-safe and fails cleanly on a
            // stale handle.
            unsafe { PostMessageW(hwnd, WM_CLOSE, 0, 0) };
        }
        return TRUE;
    }
    0
}

/// Attaches to the parent console if there is one and routes console control
/// events to `window` as `WM_CLOSE`.
///
/// # Errors
///
/// Returns the OS error when the handler cannot be installed. Having no
/// parent console is not an error.
pub fn close_window_on_ctrl_c(window: WindowHandle) -> std::io::Result<()> {
    TARGET.store(window.raw(), Ordering::Release);
    // SAFETY: no preconditions; fails when there is no parent console, which
    // is fine for a process started without a terminal.
    unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
    // SAFETY: `on_ctrl` is a valid handler for the life of the process.
    if unsafe { SetConsoleCtrlHandler(Some(on_ctrl), TRUE) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}
