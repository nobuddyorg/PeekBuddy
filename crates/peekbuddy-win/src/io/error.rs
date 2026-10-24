//! Win32 error codes as meaning (#43).
//!
//! The capture backends act on a handful of codes: an overflow, a watched
//! root that went away, a journal that is gone. [`WinError`] names them and
//! keeps every other code verbatim in [`WinError::Other`]: unknown codes are
//! never folded into a generic "other" that loses the number.

use std::fmt;

/// A Win32 error code, with the codes the backends act on named.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WinError {
    /// `ERROR_NOT_READY` (21): the device is not ready.
    NotReady,
    /// `ERROR_ACCESS_DENIED` (5): for a directory watch, the watched root was
    /// deleted.
    AccessDenied,
    /// `ERROR_INVALID_HANDLE` (6).
    InvalidHandle,
    /// `ERROR_INVALID_PARAMETER` (87): for example a buffer over 64 KiB on a
    /// network share.
    InvalidParameter,
    /// `ERROR_NOTIFY_CLEANUP` (745): the notification state was cleaned up;
    /// events may have been lost.
    NotifyCleanup,
    /// `ERROR_OPERATION_ABORTED` (995): the I/O was cancelled.
    OperationAborted,
    /// `ERROR_NOACCESS` (998): a misaligned buffer.
    NoAccess,
    /// `ERROR_NOTIFY_ENUM_DIR` (1022): the change buffer overflowed; the
    /// caller must enumerate (reconcile).
    NotifyEnumDir,
    /// `ERROR_DEVICE_NOT_CONNECTED` (1167).
    DeviceNotConnected,
    /// `ERROR_NOT_FOUND` (1168): `CancelIoEx` found no pending I/O, which
    /// means it already completed.
    NotFound,
    /// `ERROR_JOURNAL_DELETE_IN_PROGRESS` (1178).
    JournalDeleteInProgress,
    /// `ERROR_JOURNAL_NOT_ACTIVE` (1179).
    JournalNotActive,
    /// `ERROR_JOURNAL_ENTRY_DELETED` (1181, 0x49D): the journal entry the
    /// reader wanted was overwritten.
    JournalEntryDeleted,
    /// `ERROR_INVALID_USER_BUFFER` (1784).
    InvalidUserBuffer,
    /// `ERROR_CANT_ACCESS_FILE` (1920).
    CantAccessFile,
    /// Any other code, verbatim. `0` (success) lands here too.
    Other(u32),
}

impl WinError {
    /// The error for a Win32 code.
    #[must_use]
    pub const fn from_code(code: u32) -> Self {
        match code {
            5 => Self::AccessDenied,
            6 => Self::InvalidHandle,
            21 => Self::NotReady,
            87 => Self::InvalidParameter,
            745 => Self::NotifyCleanup,
            995 => Self::OperationAborted,
            998 => Self::NoAccess,
            1022 => Self::NotifyEnumDir,
            1167 => Self::DeviceNotConnected,
            1168 => Self::NotFound,
            1178 => Self::JournalDeleteInProgress,
            1179 => Self::JournalNotActive,
            1181 => Self::JournalEntryDeleted,
            1784 => Self::InvalidUserBuffer,
            1920 => Self::CantAccessFile,
            other => Self::Other(other),
        }
    }

    /// The Win32 code.
    #[must_use]
    pub const fn code(self) -> u32 {
        match self {
            Self::AccessDenied => 5,
            Self::InvalidHandle => 6,
            Self::NotReady => 21,
            Self::InvalidParameter => 87,
            Self::NotifyCleanup => 745,
            Self::OperationAborted => 995,
            Self::NoAccess => 998,
            Self::NotifyEnumDir => 1022,
            Self::DeviceNotConnected => 1167,
            Self::NotFound => 1168,
            Self::JournalDeleteInProgress => 1178,
            Self::JournalNotActive => 1179,
            Self::JournalEntryDeleted => 1181,
            Self::InvalidUserBuffer => 1784,
            Self::CantAccessFile => 1920,
            Self::Other(code) => code,
        }
    }

    /// The error of an I/O error; `None` when it carries no OS code.
    #[must_use]
    pub fn from_io(error: &std::io::Error) -> Option<Self> {
        error
            .raw_os_error()
            .map(|code| Self::from_code(code.cast_unsigned()))
    }
}

impl fmt::Display for WinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?} (Win32 error {})", self.code())
    }
}

impl std::error::Error for WinError {}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    /// Every named variant with its code, from the Win32 headers.
    const TABLE: [(u32, WinError); 15] = [
        (5, WinError::AccessDenied),
        (6, WinError::InvalidHandle),
        (21, WinError::NotReady),
        (87, WinError::InvalidParameter),
        (745, WinError::NotifyCleanup),
        (995, WinError::OperationAborted),
        (998, WinError::NoAccess),
        (1022, WinError::NotifyEnumDir),
        (1167, WinError::DeviceNotConnected),
        (1168, WinError::NotFound),
        (1178, WinError::JournalDeleteInProgress),
        (1179, WinError::JournalNotActive),
        (0x49D, WinError::JournalEntryDeleted),
        (1784, WinError::InvalidUserBuffer),
        (1920, WinError::CantAccessFile),
    ];

    #[test]
    fn every_named_code_maps_both_ways() {
        for (code, error) in TABLE {
            assert_eq!(WinError::from_code(code), error, "{code}");
            assert_eq!(error.code(), code, "{error:?}");
        }
    }

    #[test]
    fn neighbours_of_the_named_codes_stay_verbatim() {
        for (code, _) in TABLE {
            for near in [code.wrapping_sub(1), code + 1] {
                if TABLE.iter().any(|(c, _)| *c == near) {
                    continue;
                }
                assert_eq!(WinError::from_code(near), WinError::Other(near), "{near}");
            }
        }
        assert_eq!(WinError::from_code(0), WinError::Other(0));
        assert_eq!(WinError::from_code(u32::MAX), WinError::Other(u32::MAX));
    }

    #[test]
    fn io_errors_carry_their_code() {
        let error = std::io::Error::from_raw_os_error(1022);
        assert_eq!(WinError::from_io(&error), Some(WinError::NotifyEnumDir));
        let high = std::io::Error::from_raw_os_error(i32::MIN);
        assert_eq!(WinError::from_io(&high), Some(WinError::Other(0x8000_0000)));
        let none = std::io::Error::other("no code");
        assert_eq!(WinError::from_io(&none), None);
    }

    #[test]
    fn display_names_the_code() {
        assert_eq!(
            WinError::NotifyEnumDir.to_string(),
            "NotifyEnumDir (Win32 error 1022)"
        );
        assert_eq!(
            WinError::Other(12345).to_string(),
            "Other(12345) (Win32 error 12345)"
        );
    }

    proptest! {
        #[test]
        fn every_code_round_trips(code in any::<u32>()) {
            prop_assert_eq!(WinError::from_code(code).code(), code);
        }
    }
}
