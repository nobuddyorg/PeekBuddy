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

/// Makes standard output usable for a GUI-subsystem process.
///
/// Started with redirected output (a pipe or a file), the process already
/// has a valid standard output handle and nothing changes. Started from a
/// terminal it has none: this attaches to the parent's console and points
/// standard output and error at it. Started from Explorer there is no
/// console to attach to and output is dropped, which is why the commands also
/// offer a file option.
pub fn attach_output() {
    use std::os::windows::io::AsRawHandle as _;

    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Console::{
        GetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle,
    };

    // SAFETY: no preconditions.
    let current = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
    if !current.is_null() && current != INVALID_HANDLE_VALUE {
        return;
    }
    // SAFETY: no preconditions; fails without a parent console.
    if unsafe { AttachConsole(ATTACH_PARENT_PROCESS) } == 0 {
        return;
    }
    let Ok(console) = std::fs::OpenOptions::new().write(true).open("CONOUT$") else {
        return;
    };
    let handle = console.as_raw_handle();
    // The handle stays open for the life of the process.
    std::mem::forget(console);
    // SAFETY: `handle` is a live console output handle.
    unsafe { SetStdHandle(STD_OUTPUT_HANDLE, handle) };
    // SAFETY: as above.
    unsafe { SetStdHandle(STD_ERROR_HANDLE, handle) };
}
