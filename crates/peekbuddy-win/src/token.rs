//! Facts about the current process token.

use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::Security::{
    CheckTokenMembership, GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation,
    TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TokenIntegrityLevel, WinBuiltinAdministratorsSid,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use crate::sid::Sid;

/// `SECURITY_MANDATORY_MEDIUM_RID`.
pub const MEDIUM_INTEGRITY: u32 = 0x2000;
/// `SECURITY_MANDATORY_HIGH_RID`.
pub const HIGH_INTEGRITY: u32 = 0x3000;

fn last_error() -> std::io::Error {
    std::io::Error::last_os_error()
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
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &raw mut raw) } == 0 {
        return Err(last_error());
    }
    // SAFETY: OpenProcessToken succeeded, so `raw` is an owned handle.
    let token = unsafe { OwnedHandle::from_raw_handle(raw) };
    // TOKEN_MANDATORY_LABEL followed by its SID; 8-byte aligned.
    let mut buffer = [0u64; 16];
    let mut needed = 0u32;
    let size = u32::try_from(size_of_val(&buffer)).unwrap_or(u32::MAX);
    // SAFETY: `buffer` is writable for `size` bytes; `needed` is a valid out pointer.
    let ok = unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenIntegrityLevel,
            buffer.as_mut_ptr().cast(),
            size,
            &raw mut needed,
        )
    };
    if ok == 0 {
        return Err(last_error());
    }
    // SAFETY: on success the buffer starts with a TOKEN_MANDATORY_LABEL and is
    // aligned for it.
    let sid = unsafe { (*buffer.as_ptr().cast::<TOKEN_MANDATORY_LABEL>()).Label.Sid };
    // SAFETY: `sid` points into `buffer`, which is still alive.
    let count_ptr = unsafe { GetSidSubAuthorityCount(sid) };
    // SAFETY: GetSidSubAuthorityCount returns a pointer into the valid SID.
    let count = unsafe { *count_ptr };
    let last = u32::from(count).checked_sub(1).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, "integrity SID has no RID")
    })?;
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
    if unsafe { CheckTokenMembership(std::ptr::null_mut(), admins.as_psid(), &raw mut member) } == 0
    {
        return Err(last_error());
    }
    Ok(member != 0)
}
