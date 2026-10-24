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

/// Options of [`DirHandle::open`](super::dir::DirHandle::open).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OpenOptions {
    /// Open a reparse point itself instead of following it (F18).
    pub reparse_point: bool,
    /// Do not recall a cloud placeholder (F18).
    pub no_recall: bool,
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
        assert_eq!(OpenOptions::default().flags(), 0x4200_0000);
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
}
