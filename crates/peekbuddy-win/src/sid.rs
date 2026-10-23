//! Well-known SIDs in aligned stack buffers.

use windows_sys::Win32::Security::{
    CreateWellKnownSid, SECURITY_MAX_SID_SIZE, WELL_KNOWN_SID_TYPE,
};

/// `SECURITY_MAX_SID_SIZE` in 4-byte words (SIDs need 4-byte alignment).
const SID_WORDS: usize = 17;
const _: () = assert!(SID_WORDS * 4 == SECURITY_MAX_SID_SIZE as usize);

/// A SID in a buffer of the maximum SID size.
pub(crate) struct Sid([u32; SID_WORDS]);

impl Sid {
    /// Builds a well-known SID that needs no domain.
    pub(crate) fn well_known(kind: WELL_KNOWN_SID_TYPE) -> std::io::Result<Self> {
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
            return Err(std::io::Error::last_os_error());
        }
        Ok(sid)
    }

    /// The SID as the `PSID` Win32 expects.
    pub(crate) fn as_psid(&mut self) -> *mut core::ffi::c_void {
        self.0.as_mut_ptr().cast()
    }
}
