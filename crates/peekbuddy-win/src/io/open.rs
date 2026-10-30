//! How capture opens a directory (#43): the `CreateFileW` flags, as plain
//! values that build and test on every host.
//!
//! The combined values are literals so that no operator in them can be
//! mutated unseen; a Windows test compares them with windows-sys.

/// `FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED`: open a directory for
/// overlapped I/O.
pub const BASE_FLAGS: u32 = 0x4200_0000;
/// `FILE_FLAG_OPEN_REPARSE_POINT`: open a link itself, not its target.
pub const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
/// `FILE_FLAG_OPEN_NO_RECALL`: never recall a cloud file by opening it.
pub const OPEN_NO_RECALL: u32 = 0x0010_0000;

/// `FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE`: a watch never
/// blocks others.
pub const SHARE_ALL: u32 = 0x0000_0007;

/// `FILE_LIST_DIRECTORY`, the access a directory watch needs.
pub const LIST_DIRECTORY: u32 = 0x0000_0001;

/// Access bits that read or change file data: write, append, execute and the
/// generic rights. `FILE_READ_DATA` (bit 0) is the same bit as
/// [`LIST_DIRECTORY`], so it is judged by what is opened, not by this mask.
pub const DATA_ACCESS: u32 = 0x0000_0002 // FILE_WRITE_DATA
    | 0x0000_0004 // FILE_APPEND_DATA
    | 0x0000_0020 // FILE_EXECUTE
    | 0x8000_0000 // GENERIC_READ
    | 0x4000_0000 // GENERIC_WRITE
    | 0x2000_0000 // GENERIC_EXECUTE
    | 0x1000_0000; // GENERIC_ALL

/// Why a capture-side open is refused (F18): no data is ever read or
/// recalled by watching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyError {
    /// The access mask asks for data access.
    DataAccess(u32),
    /// The flags lack `FILE_FLAG_OPEN_NO_RECALL`.
    WouldRecall,
}

impl std::fmt::Display for PolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DataAccess(bits) => write!(f, "the access {bits:#x} reads or changes data"),
            Self::WouldRecall => f.write_str("the flags would let a cloud file be recalled"),
        }
    }
}

impl std::error::Error for PolicyError {}

/// The capture-side open policy: attribute and listing access only, and
/// `FILE_FLAG_OPEN_NO_RECALL` always. [`DirHandle::open`](super::dir::DirHandle::open)
/// applies it to every open it makes.
///
/// # Errors
///
/// [`PolicyError`] for an access with data bits or flags without no-recall.
pub const fn check(access: u32, flags: u32) -> Result<(), PolicyError> {
    if access & DATA_ACCESS != 0 {
        return Err(PolicyError::DataAccess(access & DATA_ACCESS));
    }
    if flags & OPEN_NO_RECALL == 0 {
        return Err(PolicyError::WouldRecall);
    }
    Ok(())
}

/// Options of [`DirHandle::open`](super::dir::DirHandle::open).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenOptions {
    /// Open a reparse point itself instead of following it (F18).
    pub reparse_point: bool,
    /// Do not recall a cloud placeholder (F18). On by default; turning it off
    /// makes [`DirHandle::open`](super::dir::DirHandle::open) refuse.
    pub no_recall: bool,
}

impl Default for OpenOptions {
    fn default() -> Self {
        Self {
            reparse_point: false,
            no_recall: true,
        }
    }
}

impl OpenOptions {
    /// The flags and attributes for `CreateFileW`.
    #[must_use]
    pub const fn flags(self) -> u32 {
        let mut flags = BASE_FLAGS;
        if self.reparse_point {
            flags += OPEN_REPARSE_POINT;
        }
        if self.no_recall {
            flags += OPEN_NO_RECALL;
        }
        flags
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_follow_the_options() {
        assert_eq!(OpenOptions::default().flags(), 0x4210_0000);
        assert_eq!(
            OpenOptions {
                reparse_point: true,
                no_recall: false
            }
            .flags(),
            0x4220_0000
        );
        assert_eq!(
            OpenOptions {
                reparse_point: false,
                no_recall: true
            }
            .flags(),
            0x4210_0000
        );
        assert_eq!(
            OpenOptions {
                reparse_point: true,
                no_recall: true
            }
            .flags(),
            0x4230_0000
        );
    }

    #[test]
    fn the_literals_are_disjoint_bits() {
        let all = [BASE_FLAGS, OPEN_REPARSE_POINT, OPEN_NO_RECALL];
        for (i, a) in all.iter().enumerate() {
            for b in all.iter().skip(i + 1) {
                assert_eq!(a & b, 0);
            }
        }
        assert_eq!(SHARE_ALL, 1 | 2 | 4);
        assert_eq!(LIST_DIRECTORY, 1);
    }
    #[test]
    fn the_policy_refuses_data_access_and_a_missing_no_recall_flag() {
        let ok = OpenOptions::default().flags();
        assert_eq!(check(LIST_DIRECTORY, ok), Ok(()));
        // FILE_READ_ATTRIBUTES and SYNCHRONIZE are fine.
        assert_eq!(check(0x80 | 0x0010_0000, ok), Ok(()));
        for bits in [
            0x2,
            0x4,
            0x20,
            0x8000_0000,
            0x4000_0000,
            0x2000_0000,
            0x1000_0000,
        ] {
            assert_eq!(
                check(LIST_DIRECTORY | bits, ok),
                Err(PolicyError::DataAccess(bits)),
                "{bits:#x}"
            );
        }
        // Any flag set without FILE_FLAG_OPEN_NO_RECALL.
        for flags in [0, BASE_FLAGS, BASE_FLAGS + OPEN_REPARSE_POINT, 0x4200_0000] {
            assert_eq!(check(LIST_DIRECTORY, flags), Err(PolicyError::WouldRecall));
        }
        // Every combination the options can produce, with no_recall on, passes.
        for reparse_point in [false, true] {
            let flags = OpenOptions {
                reparse_point,
                no_recall: true,
            }
            .flags();
            assert_eq!(check(LIST_DIRECTORY, flags), Ok(()));
        }
    }
}
