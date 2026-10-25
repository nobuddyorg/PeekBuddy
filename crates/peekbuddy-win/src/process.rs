//! Child processes with an unelevated token.
//!
//! Hosted CI runners run elevated with UAC off (F12). Tests that must see
//! the behaviour of a normal user start a child with a restricted token:
//! maximum privileges removed, the Administrators group set to deny-only and
//! the integrity level lowered to Medium. The runner probe (#34) showed that
//! a `LUA_TOKEN` child fails to start on hosted runners
//! (`STATUS_DLL_INIT_FAILED`), so that flag is not used.
//!
//! A child whose Administrators group is deny-only cannot attach to a
//! console: created with a console it dies with `STATUS_DLL_INIT_FAILED`
//! (0xC0000142). The runner probe (#34, run 39962290020) showed that
//! `DETACHED_PROCESS` fixes it for `cmd.exe` and Rust binaries alike, so the
//! child runs without a console and reports through its exit code.

use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};

use windows_sys::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::Security::{
    CreateRestrictedToken, DISABLE_MAX_PRIVILEGE, GetLengthSid, SID_AND_ATTRIBUTES,
    SetTokenInformation, TOKEN_ADJUST_DEFAULT, TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE,
    TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TokenIntegrityLevel, WinBuiltinAdministratorsSid,
    WinMediumLabelSid,
};
use windows_sys::Win32::System::Threading::{
    CreateProcessAsUserW, DETACHED_PROCESS, GetCurrentProcess, GetExitCodeProcess, INFINITE,
    OpenProcessToken, PROCESS_INFORMATION, STARTUPINFOW, WaitForSingleObject,
};

use crate::sid::Sid;

/// `SE_GROUP_INTEGRITY` from `winnt.h` (kept local to avoid the large
/// `Win32_System_SystemServices` feature).
const SE_GROUP_INTEGRITY: u32 = 0x20;

fn last_error() -> std::io::Error {
    std::io::Error::last_os_error()
}

fn check(ok: i32) -> std::io::Result<()> {
    if ok == 0 { Err(last_error()) } else { Ok(()) }
}

fn unelevated_token() -> std::io::Result<OwnedHandle> {
    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: GetCurrentProcess returns a pseudo-handle that needs no closing.
    let process = unsafe { GetCurrentProcess() };
    // SAFETY: `token` is a valid out pointer; the access mask is a constant.
    check(unsafe {
        OpenProcessToken(
            process,
            TOKEN_QUERY | TOKEN_DUPLICATE | TOKEN_ASSIGN_PRIMARY | TOKEN_ADJUST_DEFAULT,
            &raw mut token,
        )
    })?;
    // SAFETY: OpenProcessToken succeeded, so `token` is an owned handle.
    let token = unsafe { OwnedHandle::from_raw_handle(token) };

    let mut admins = Sid::well_known(WinBuiltinAdministratorsSid)?;
    let disable = [SID_AND_ATTRIBUTES {
        Sid: admins.as_psid(),
        Attributes: 0,
    }];
    let mut restricted: HANDLE = std::ptr::null_mut();
    // SAFETY: `token` is live; `disable` holds one valid SID that outlives the
    // call; no privileges or restricting SIDs are passed; `restricted` is a
    // valid out pointer.
    check(unsafe {
        CreateRestrictedToken(
            token.as_raw_handle(),
            DISABLE_MAX_PRIVILEGE,
            1,
            disable.as_ptr(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
            &raw mut restricted,
        )
    })?;
    // SAFETY: CreateRestrictedToken succeeded, so `restricted` is owned.
    let restricted = unsafe { OwnedHandle::from_raw_handle(restricted) };

    let mut medium = Sid::well_known(WinMediumLabelSid)?;
    let psid = medium.as_psid();
    // SAFETY: `psid` points to the valid SID built above.
    let sid_len = unsafe { GetLengthSid(psid) };
    let label = TOKEN_MANDATORY_LABEL {
        Label: SID_AND_ATTRIBUTES {
            Sid: psid,
            Attributes: SE_GROUP_INTEGRITY,
        },
    };
    let size = u32::try_from(size_of::<TOKEN_MANDATORY_LABEL>())
        .unwrap_or(u32::MAX)
        .saturating_add(sid_len);
    // SAFETY: `restricted` is a token we own with full access; `label` points
    // to a valid SID that outlives the call.
    check(unsafe {
        SetTokenInformation(
            restricted.as_raw_handle(),
            TokenIntegrityLevel,
            (&raw const label).cast(),
            size,
        )
    })?;
    Ok(restricted)
}

/// A default `STARTUPINFOW` with `cb` set, as the API contract requires.
fn startup_info() -> STARTUPINFOW {
    STARTUPINFOW {
        cb: u32::try_from(size_of::<STARTUPINFOW>()).unwrap_or(u32::MAX),
        ..STARTUPINFOW::default()
    }
}

/// Runs `command_line` as an unelevated child (Administrators deny-only,
/// Medium integrity), waits for it and returns its exit code.
///
/// # Errors
///
/// Returns the OS error when the token cannot be created or the process
/// cannot be started.
pub fn run_restricted(command_line: &str) -> std::io::Result<u32> {
    let token = unelevated_token()?;
    let mut command: Vec<u16> = command_line.encode_utf16().chain(Some(0)).collect();
    let startup = startup_info();
    let mut info = PROCESS_INFORMATION::default();
    // SAFETY: `token` is a primary token derived from our own; `command` is a
    // mutable NUL-terminated UTF-16 buffer that outlives the call; the
    // startup and process-information structs are valid for the call.
    check(unsafe {
        CreateProcessAsUserW(
            token.as_raw_handle(),
            std::ptr::null(),
            command.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            DETACHED_PROCESS,
            std::ptr::null(),
            std::ptr::null(),
            &raw const startup,
            &raw mut info,
        )
    })?;
    // SAFETY: CreateProcessAsUserW succeeded, so both handles are owned.
    let process = unsafe { OwnedHandle::from_raw_handle(info.hProcess) };
    // SAFETY: as above.
    drop(unsafe { OwnedHandle::from_raw_handle(info.hThread) });
    let raw = process.as_raw_handle();
    // SAFETY: `raw` is a live process handle owned by `process`.
    let waited = unsafe { WaitForSingleObject(raw, INFINITE) };
    if waited != WAIT_OBJECT_0 {
        return Err(last_error());
    }
    let mut code = 0u32;
    // SAFETY: `raw` is live; `code` is a valid out pointer.
    check(unsafe { GetExitCodeProcess(raw, &raw mut code) })?;
    Ok(code)
}

/// Lower-case base names of the modules loaded in process `pid`, such as
/// `kernel32.dll`. The engine's module list is checked against the GPU and UI
/// libraries it must never load (F1, #59).
///
/// # Errors
///
/// Returns the OS error when the process cannot be opened or enumerated.
pub fn module_names(pid: u32) -> std::io::Result<Vec<String>> {
    use windows_sys::Win32::System::ProcessStatus::{
        EnumProcessModulesEx, GetModuleBaseNameW, LIST_MODULES_ALL,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ,
    };

    // SAFETY: plain integer arguments; a null result is checked below.
    let raw = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid) };
    if raw.is_null() {
        return Err(last_error());
    }
    // SAFETY: OpenProcess succeeded, so `raw` is an owned handle.
    let process = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut modules = vec![std::ptr::null_mut(); 1024];
    let mut needed = 0u32;
    let capacity =
        u32::try_from(modules.len() * size_of::<*mut core::ffi::c_void>()).unwrap_or(u32::MAX);
    // SAFETY: `modules` has `capacity` bytes of writable space; `needed` is a
    // valid out pointer; `process` is live.
    check(unsafe {
        EnumProcessModulesEx(
            process.as_raw_handle(),
            modules.as_mut_ptr(),
            capacity,
            &raw mut needed,
            LIST_MODULES_ALL,
        )
    })?;
    let count = (needed as usize / size_of::<*mut core::ffi::c_void>()).min(modules.len());
    let mut names = Vec::with_capacity(count);
    for module in modules.iter().take(count) {
        let mut buffer = [0u16; 260];
        // SAFETY: `buffer` has 260 writable UTF-16 units; `module` came from
        // the enumeration above.
        let length = unsafe {
            GetModuleBaseNameW(process.as_raw_handle(), *module, buffer.as_mut_ptr(), 260)
        } as usize;
        if let Some(units) = buffer.get(..length) {
            names.push(String::from_utf16_lossy(units).to_lowercase());
        }
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::{STARTUPINFOW, startup_info};

    #[test]
    fn startup_info_carries_its_size() {
        // CreateProcessAsUserW tolerates a zero `cb` today, so only this
        // test notices if it goes missing.
        let size = u32::try_from(size_of::<STARTUPINFOW>()).unwrap();
        assert_eq!(startup_info().cb, size);
    }
}
