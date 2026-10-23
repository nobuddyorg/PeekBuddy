//! Child processes with an unelevated token.
//!
//! Hosted CI runners run elevated with UAC off (F12). Tests that must see
//! the behaviour of a normal user start a child with a restricted token:
//! maximum privileges removed, the Administrators group set to deny-only and
//! the integrity level lowered to Medium. The runner probe (#34) showed that
//! a `LUA_TOKEN` child fails to start on hosted runners
//! (`STATUS_DLL_INIT_FAILED`), so that flag is not used.

use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};

use windows_sys::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::Security::{
    CreateRestrictedToken, CreateWellKnownSid, DISABLE_MAX_PRIVILEGE, GetLengthSid,
    SECURITY_MAX_SID_SIZE, SID_AND_ATTRIBUTES, SetTokenInformation, TOKEN_ADJUST_DEFAULT,
    TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE, TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TokenIntegrityLevel,
    WELL_KNOWN_SID_TYPE, WinBuiltinAdministratorsSid, WinMediumLabelSid,
};
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CreateProcessAsUserW, GetCurrentProcess, GetExitCodeProcess, INFINITE,
    OpenProcessToken, PROCESS_INFORMATION, STARTUPINFOW, WaitForSingleObject,
};

/// `SE_GROUP_INTEGRITY` from `winnt.h` (kept local to avoid the large
/// `Win32_System_SystemServices` feature).
const SE_GROUP_INTEGRITY: u32 = 0x20;

/// `SECURITY_MAX_SID_SIZE` in 4-byte words (SIDs need 4-byte alignment).
const SID_WORDS: usize = 17;
const _: () = assert!(SID_WORDS * 4 == SECURITY_MAX_SID_SIZE as usize);

/// A SID in a buffer of the maximum SID size.
struct Sid([u32; SID_WORDS]);

impl Sid {
    fn well_known(kind: WELL_KNOWN_SID_TYPE) -> std::io::Result<Self> {
        let mut sid = Self([0; SID_WORDS]);
        let mut size = SECURITY_MAX_SID_SIZE;
        // SAFETY: the buffer holds SECURITY_MAX_SID_SIZE bytes and `size` says
        // so; a null domain SID is allowed for these well-known types.
        let ok = unsafe {
            CreateWellKnownSid(
                kind,
                std::ptr::null_mut(),
                sid.0.as_mut_ptr().cast(),
                &raw mut size,
            )
        };
        if ok == 0 {
            return Err(last_error());
        }
        Ok(sid)
    }

    fn as_psid(&mut self) -> *mut core::ffi::c_void {
        self.0.as_mut_ptr().cast()
    }
}

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
    let startup = STARTUPINFOW {
        cb: u32::try_from(size_of::<STARTUPINFOW>()).unwrap_or(u32::MAX),
        ..STARTUPINFOW::default()
    };
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
            CREATE_NO_WINDOW,
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
