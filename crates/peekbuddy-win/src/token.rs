//! Facts about the current process token.

use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::Security::{
    CheckTokenMembership, CreateRestrictedToken, DISABLE_MAX_PRIVILEGE, GetSidSubAuthority,
    GetSidSubAuthorityCount, GetTokenInformation, SID_AND_ATTRIBUTES, TOKEN_ASSIGN_PRIMARY,
    TOKEN_DUPLICATE, TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TokenIntegrityLevel,
    WinBuiltinAdministratorsSid,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use crate::sid::Sid;

/// `SECURITY_MANDATORY_MEDIUM_RID`.
pub const MEDIUM_INTEGRITY: u32 = 0x2000;
/// `SECURITY_MANDATORY_HIGH_RID`.
pub const HIGH_INTEGRITY: u32 = 0x3000;

fn check(ok: i32) -> std::io::Result<()> {
    if ok == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Returns the integrity level RID of the current process, for example
/// [`MEDIUM_INTEGRITY`] or [`HIGH_INTEGRITY`].
///
/// # Errors
///
/// Returns the OS error when the token cannot be read.
pub fn integrity_rid() -> std::io::Result<u32> {
    let mut raw: HANDLE = std::ptr::null_mut();
    // SAFETY: GetCurrentProcess returns a pseudo-handle that needs no closing.
    let process = unsafe { GetCurrentProcess() };
    // SAFETY: `raw` is a valid out pointer.
    check(unsafe { OpenProcessToken(process, TOKEN_QUERY, &raw mut raw) })
        .map_err(|error| context("OpenProcessToken", &error))?;
    // SAFETY: OpenProcessToken succeeded, so `raw` is an owned handle.
    let token = unsafe { OwnedHandle::from_raw_handle(raw) };
    // TOKEN_MANDATORY_LABEL followed by its SID; 8-byte aligned.
    let mut buffer = [0u64; 16];
    let mut needed = 0u32;
    let size = u32::try_from(size_of_val(&buffer)).unwrap_or(u32::MAX);
    // SAFETY: `buffer` is writable for `size` bytes; `needed` is a valid out pointer.
    check(unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenIntegrityLevel,
            buffer.as_mut_ptr().cast(),
            size,
            &raw mut needed,
        )
    })?;
    // SAFETY: on success the buffer starts with a TOKEN_MANDATORY_LABEL and is
    // aligned for it.
    let sid = unsafe { (*buffer.as_ptr().cast::<TOKEN_MANDATORY_LABEL>()).Label.Sid };
    // SAFETY: `sid` points into `buffer`, which is still alive.
    let count_ptr = unsafe { GetSidSubAuthorityCount(sid) };
    // SAFETY: GetSidSubAuthorityCount returns a pointer into the valid SID.
    let count = unsafe { *count_ptr };
    // An integrity SID always has exactly one sub-authority, the RID.
    let last = u32::from(count).saturating_sub(1);
    // SAFETY: `last` is a valid sub-authority index of `sid`.
    let rid_ptr = unsafe { GetSidSubAuthority(sid, last) };
    // SAFETY: GetSidSubAuthority returns a pointer into the valid SID.
    Ok(unsafe { *rid_ptr })
}

/// Returns whether the current process acts as a member of
/// `BUILTIN\Administrators` (false when the group is deny-only).
///
/// # Errors
///
/// Returns the OS error when membership cannot be checked.
pub fn is_admin() -> std::io::Result<bool> {
    let mut admins = Sid::well_known(WinBuiltinAdministratorsSid)?;
    let mut member = 0;
    // SAFETY: a null token means the calling thread's effective token; the SID
    // is valid; `member` is a valid out pointer.
    check(unsafe {
        CheckTokenMembership(std::ptr::null_mut(), admins.as_psid(), &raw mut member)
    })?;
    Ok(member != 0)
}

/// An error that says which call failed.
fn context(call: &str, error: &std::io::Error) -> std::io::Error {
    std::io::Error::other(format!("{call}: {error}"))
}

/// A restricted copy of the current process token: no privileges (but the
/// right to bypass traverse checking) and `BUILTIN\Administrators` deny-only.
/// That is what a standard user's process holds, made from an elevated one.
/// The integrity level stays the parent's: a copy whose level was changed is
/// no longer "a restricted version of the caller's token" and could only be
/// assigned to a process with `SeAssignPrimaryTokenPrivilege`.
struct Restricted(OwnedHandle);

impl Restricted {
    fn new() -> std::io::Result<Self> {
        let mut admins = Sid::well_known(WinBuiltinAdministratorsSid)?;
        let mut raw: HANDLE = std::ptr::null_mut();
        // SAFETY: GetCurrentProcess returns a pseudo-handle that needs no closing.
        let process = unsafe { GetCurrentProcess() };
        // SAFETY: `raw` is a valid out pointer.
        check(unsafe {
            OpenProcessToken(
                process,
                TOKEN_DUPLICATE | TOKEN_ASSIGN_PRIMARY | TOKEN_QUERY,
                &raw mut raw,
            )
        })
        .map_err(|error| context("OpenProcessToken", &error))?;
        // SAFETY: OpenProcessToken succeeded, so `raw` is an owned handle.
        let current = unsafe { OwnedHandle::from_raw_handle(raw) };
        let disable = SID_AND_ATTRIBUTES {
            Sid: admins.as_psid(),
            Attributes: 0,
        };
        let mut restricted: HANDLE = std::ptr::null_mut();
        // SAFETY: `current` is a live token handle; `disable` names one valid
        // SID that outlives the call; no restricting SIDs; `restricted` is a
        // valid out pointer.
        check(unsafe {
            CreateRestrictedToken(
                current.as_raw_handle(),
                DISABLE_MAX_PRIVILEGE,
                1,
                &raw const disable,
                0,
                std::ptr::null(),
                0,
                std::ptr::null(),
                &raw mut restricted,
            )
        })
        .map_err(|error| context("CreateRestrictedToken", &error))?;
        // SAFETY: CreateRestrictedToken succeeded, so `restricted` is owned.
        Ok(Self(unsafe { OwnedHandle::from_raw_handle(restricted) }))
    }
}

/// Runs `program` with `args` as a child of this process under a restricted
/// copy of its token (see [`Restricted`]) and waits at most `timeout` for it;
/// returns its exit code. A child that does not end in time is terminated and
/// reported as `ERROR_TIMEOUT`. The child inherits the environment and the
/// current directory and no handles.
///
/// # Errors
///
/// Returns the OS error when the token cannot be made or the process cannot
/// be created or waited for.
pub fn run_restricted(
    program: &std::path::Path,
    args: &[&str],
    timeout: std::time::Duration,
) -> std::io::Result<u32> {
    use std::fmt::Write as _;
    use std::os::windows::ffi::OsStrExt as _;

    use windows_sys::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Threading::{
        CREATE_UNICODE_ENVIRONMENT, CreateProcessAsUserW, GetExitCodeProcess, PROCESS_INFORMATION,
        STARTUPINFOW, TerminateProcess, WaitForSingleObject,
    };

    let token = Restricted::new()?;
    let application: Vec<u16> = program
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut line = format!("\"{}\"", program.display());
    for arg in args {
        let _ = write!(line, " \"{arg}\"");
    }
    let mut command: Vec<u16> = line.encode_utf16().chain(std::iter::once(0)).collect();
    let startup = STARTUPINFOW {
        cb: u32::try_from(size_of::<STARTUPINFOW>()).unwrap_or(0),
        ..STARTUPINFOW::default()
    };
    let mut info = PROCESS_INFORMATION::default();
    // SAFETY: `token.0` is a primary token handle; `application` and
    // `command` are NUL-terminated and `command` is writable; no security
    // attributes, no handle inheritance, the environment and directory of
    // this process; `startup` and `info` are valid for the call.
    check(unsafe {
        CreateProcessAsUserW(
            token.0.as_raw_handle(),
            application.as_ptr(),
            command.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            CREATE_UNICODE_ENVIRONMENT,
            std::ptr::null(),
            std::ptr::null(),
            &raw const startup,
            &raw mut info,
        )
    })
    .map_err(|error| context("CreateProcessAsUserW", &error))?;
    // SAFETY: CreateProcessAsUserW succeeded, so the process handle is owned.
    let process = unsafe { OwnedHandle::from_raw_handle(info.hProcess) };
    // SAFETY: as above for the thread handle, which is not needed.
    drop(unsafe { OwnedHandle::from_raw_handle(info.hThread) });
    let millis = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
    // SAFETY: `process` is a live process handle.
    let waited = unsafe { WaitForSingleObject(process.as_raw_handle(), millis) };
    if waited == WAIT_TIMEOUT {
        // SAFETY: `process` is live; the child is ours.
        unsafe { TerminateProcess(process.as_raw_handle(), 1) };
        return Err(std::io::Error::from_raw_os_error(1460));
    }
    if waited != WAIT_OBJECT_0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut code = 0u32;
    // SAFETY: `process` is live and has ended; `code` is a valid out pointer.
    check(unsafe { GetExitCodeProcess(process.as_raw_handle(), &raw mut code) })?;
    Ok(code)
}
