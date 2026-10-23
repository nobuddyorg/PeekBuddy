//! Child processes with a restricted (unelevated) token.
//!
//! Hosted CI runners run elevated with UAC off (F12). Tests that must see
//! the behaviour of a normal user start a child with a restricted token:
//! maximum privileges removed and the token marked as a LUA (limited) token.

use std::os::windows::io::{FromRawHandle as _, OwnedHandle};

use windows_sys::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::Security::{
    CreateRestrictedToken, DISABLE_MAX_PRIVILEGE, LUA_TOKEN, TOKEN_ADJUST_DEFAULT,
    TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE, TOKEN_QUERY,
};
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CreateProcessAsUserW, GetCurrentProcess, GetExitCodeProcess, INFINITE,
    OpenProcessToken, PROCESS_INFORMATION, STARTUPINFOW, WaitForSingleObject,
};

fn last_error() -> std::io::Error {
    std::io::Error::last_os_error()
}

fn restricted_token() -> std::io::Result<OwnedHandle> {
    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: GetCurrentProcess returns a pseudo-handle that needs no closing.
    let process = unsafe { GetCurrentProcess() };
    // SAFETY: `token` is a valid out pointer; the access mask is a constant.
    let ok = unsafe {
        OpenProcessToken(
            process,
            TOKEN_QUERY | TOKEN_DUPLICATE | TOKEN_ASSIGN_PRIMARY | TOKEN_ADJUST_DEFAULT,
            &raw mut token,
        )
    };
    if ok == 0 {
        return Err(last_error());
    }
    // SAFETY: OpenProcessToken succeeded, so `token` is an owned handle.
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut restricted: HANDLE = std::ptr::null_mut();
    // SAFETY: `token` is a live token handle; all SID and privilege lists are
    // empty (count 0, null pointer); `restricted` is a valid out pointer.
    let ok = unsafe {
        CreateRestrictedToken(
            std::os::windows::io::AsRawHandle::as_raw_handle(&token),
            DISABLE_MAX_PRIVILEGE | LUA_TOKEN,
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
            &raw mut restricted,
        )
    };
    if ok == 0 {
        return Err(last_error());
    }
    // SAFETY: CreateRestrictedToken succeeded, so `restricted` is owned.
    Ok(unsafe { OwnedHandle::from_raw_handle(restricted) })
}

/// Runs `command_line` with a restricted token, waits for it and returns its
/// exit code.
///
/// # Errors
///
/// Returns the OS error when the token cannot be created or the process
/// cannot be started.
pub fn run_restricted(command_line: &str) -> std::io::Result<u32> {
    let token = restricted_token()?;
    let mut command: Vec<u16> = command_line.encode_utf16().chain(Some(0)).collect();
    let startup = STARTUPINFOW {
        cb: u32::try_from(size_of::<STARTUPINFOW>()).unwrap_or(u32::MAX),
        ..STARTUPINFOW::default()
    };
    let mut info = PROCESS_INFORMATION::default();
    // SAFETY: `token` is a primary token derived from our own; `command` is a
    // mutable NUL-terminated UTF-16 buffer that outlives the call; the
    // startup and process-information structs are valid for the call.
    let ok = unsafe {
        CreateProcessAsUserW(
            std::os::windows::io::AsRawHandle::as_raw_handle(&token),
            std::ptr::null(),
            command.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            CREATE_NO_WINDOW,
            std::ptr::null(),
            std::ptr::null(),
            &raw const startup,
            &raw mut info,
        )
    };
    if ok == 0 {
        return Err(last_error());
    }
    // SAFETY: CreateProcessAsUserW succeeded, so both handles are owned.
    let process = unsafe { OwnedHandle::from_raw_handle(info.hProcess) };
    // SAFETY: as above.
    drop(unsafe { OwnedHandle::from_raw_handle(info.hThread) });
    let raw = std::os::windows::io::AsRawHandle::as_raw_handle(&process);
    // SAFETY: `raw` is a live process handle owned by `process`.
    let waited = unsafe { WaitForSingleObject(raw, INFINITE) };
    if waited != WAIT_OBJECT_0 {
        return Err(last_error());
    }
    let mut code = 0u32;
    // SAFETY: `raw` is live; `code` is a valid out pointer.
    let ok = unsafe { GetExitCodeProcess(raw, &raw mut code) };
    if ok == 0 {
        return Err(last_error());
    }
    Ok(code)
}
