//! Mapping from raw OS values to [`Kind`].
//!
//! Pure functions over `const` values: no allocation, no panics, total for
//! every `u32`. The raw value is never replaced by the kind (F3). Core keeps
//! its own copies of the Win32 constants; a Windows test in `peekbuddy-win`
//! compares them with windows-sys =0.61.2.

use super::Kind;

/// `FILE_ACTION_*` values reported by `ReadDirectoryChangesExW`.
pub mod file_action {
    /// `FILE_ACTION_ADDED`.
    pub const ADDED: u32 = 1;
    /// `FILE_ACTION_REMOVED`.
    pub const REMOVED: u32 = 2;
    /// `FILE_ACTION_MODIFIED`.
    pub const MODIFIED: u32 = 3;
    /// `FILE_ACTION_RENAMED_OLD_NAME`.
    pub const RENAMED_OLD_NAME: u32 = 4;
    /// `FILE_ACTION_RENAMED_NEW_NAME`.
    pub const RENAMED_NEW_NAME: u32 = 5;
    /// `FILE_ACTION_ADDED_STREAM`.
    pub const ADDED_STREAM: u32 = 6;
    /// `FILE_ACTION_REMOVED_STREAM`.
    pub const REMOVED_STREAM: u32 = 7;
    /// `FILE_ACTION_MODIFIED_STREAM`.
    pub const MODIFIED_STREAM: u32 = 8;
    /// `FILE_ACTION_REMOVED_BY_DELETE`.
    pub const REMOVED_BY_DELETE: u32 = 9;
    /// `FILE_ACTION_ID_NOT_TUNNELLED`.
    pub const ID_NOT_TUNNELLED: u32 = 10;
    /// `FILE_ACTION_TUNNELLED_ID_COLLISION`.
    pub const TUNNELLED_ID_COLLISION: u32 = 11;
}

/// `USN_REASON_*` bits.
pub mod usn_reason {
    /// `USN_REASON_DATA_OVERWRITE`.
    pub const DATA_OVERWRITE: u32 = 0x0000_0001;
    /// `USN_REASON_DATA_EXTEND`.
    pub const DATA_EXTEND: u32 = 0x0000_0002;
    /// `USN_REASON_DATA_TRUNCATION`.
    pub const DATA_TRUNCATION: u32 = 0x0000_0004;
    /// `USN_REASON_NAMED_DATA_OVERWRITE`.
    pub const NAMED_DATA_OVERWRITE: u32 = 0x0000_0010;
    /// `USN_REASON_NAMED_DATA_EXTEND`.
    pub const NAMED_DATA_EXTEND: u32 = 0x0000_0020;
    /// `USN_REASON_NAMED_DATA_TRUNCATION`.
    pub const NAMED_DATA_TRUNCATION: u32 = 0x0000_0040;
    /// `USN_REASON_FILE_CREATE`.
    pub const FILE_CREATE: u32 = 0x0000_0100;
    /// `USN_REASON_FILE_DELETE`.
    pub const FILE_DELETE: u32 = 0x0000_0200;
    /// `USN_REASON_EA_CHANGE`.
    pub const EA_CHANGE: u32 = 0x0000_0400;
    /// `USN_REASON_SECURITY_CHANGE`.
    pub const SECURITY_CHANGE: u32 = 0x0000_0800;
    /// `USN_REASON_RENAME_OLD_NAME`.
    pub const RENAME_OLD_NAME: u32 = 0x0000_1000;
    /// `USN_REASON_RENAME_NEW_NAME`.
    pub const RENAME_NEW_NAME: u32 = 0x0000_2000;
    /// `USN_REASON_INDEXABLE_CHANGE`.
    pub const INDEXABLE_CHANGE: u32 = 0x0000_4000;
    /// `USN_REASON_BASIC_INFO_CHANGE`.
    pub const BASIC_INFO_CHANGE: u32 = 0x0000_8000;
    /// `USN_REASON_HARD_LINK_CHANGE`.
    pub const HARD_LINK_CHANGE: u32 = 0x0001_0000;
    /// `USN_REASON_COMPRESSION_CHANGE`.
    pub const COMPRESSION_CHANGE: u32 = 0x0002_0000;
    /// `USN_REASON_ENCRYPTION_CHANGE`.
    pub const ENCRYPTION_CHANGE: u32 = 0x0004_0000;
    /// `USN_REASON_OBJECT_ID_CHANGE`.
    pub const OBJECT_ID_CHANGE: u32 = 0x0008_0000;
    /// `USN_REASON_REPARSE_POINT_CHANGE`.
    pub const REPARSE_POINT_CHANGE: u32 = 0x0010_0000;
    /// `USN_REASON_STREAM_CHANGE`.
    pub const STREAM_CHANGE: u32 = 0x0020_0000;
    /// `USN_REASON_TRANSACTED_CHANGE`.
    pub const TRANSACTED_CHANGE: u32 = 0x0040_0000;
    /// `USN_REASON_INTEGRITY_CHANGE`.
    pub const INTEGRITY_CHANGE: u32 = 0x0080_0000;
    /// `USN_REASON_DESIRED_STORAGE_CLASS_CHANGE`.
    pub const DESIRED_STORAGE_CLASS_CHANGE: u32 = 0x0100_0000;
    /// `USN_REASON_CLOSE`.
    pub const CLOSE: u32 = 0x8000_0000;

    /// Every defined bit (literal; a test checks it against the members).
    pub const ALL: u32 = 0x81FF_FF77;

    /// The plain data bits.
    pub const DATA: u32 = 0x0000_0007;
    /// The named-data and stream bits.
    pub const STREAM: u32 = 0x0020_0070;
    /// The link bits.
    pub const LINK: u32 = 0x0011_0000;
    /// Metadata bits that map to `AttributesChanged` ("and the rest").
    pub const ATTRIBUTES: u32 = 0x01CE_C400;
}

/// The kind of an RDCW `FILE_ACTION_*` value. Unknown values give
/// [`Kind::Other`]; the caller keeps the raw value in `raw_mask`.
#[must_use]
pub const fn rdcw_kind(action: u32) -> Kind {
    use file_action::{
        ADDED, ADDED_STREAM, MODIFIED, MODIFIED_STREAM, REMOVED, REMOVED_BY_DELETE, REMOVED_STREAM,
        RENAMED_NEW_NAME, RENAMED_OLD_NAME,
    };
    match action {
        ADDED => Kind::Created,
        REMOVED | REMOVED_BY_DELETE => Kind::Deleted,
        MODIFIED => Kind::Modified,
        RENAMED_OLD_NAME => Kind::RenamedFrom,
        RENAMED_NEW_NAME => Kind::RenamedTo,
        ADDED_STREAM | REMOVED_STREAM | MODIFIED_STREAM => Kind::StreamChanged,
        // ID_NOT_TUNNELLED, TUNNELLED_ID_COLLISION and anything unknown.
        _ => Kind::Other,
    }
}

/// The primary kind of a USN reason mask.
///
/// One record is one row (F3); the full mask stays in `raw_mask`. Precedence,
/// highest first: `FILE_DELETE`, `FILE_CREATE`, `RENAME_OLD_NAME`,
/// `RENAME_NEW_NAME`, the data bits, `SECURITY_CHANGE`, the stream and
/// named-data bits, the hard-link and reparse bits, then the remaining
/// metadata bits. A mask with only `CLOSE` or undefined bits gives
/// [`Kind::Other`].
#[must_use]
pub const fn usn_kind(mask: u32) -> Kind {
    use usn_reason::{
        ATTRIBUTES, DATA, FILE_CREATE, FILE_DELETE, LINK, RENAME_NEW_NAME, RENAME_OLD_NAME,
        SECURITY_CHANGE, STREAM,
    };
    if mask & FILE_DELETE != 0 {
        Kind::Deleted
    } else if mask & FILE_CREATE != 0 {
        Kind::Created
    } else if mask & RENAME_OLD_NAME != 0 {
        Kind::RenamedFrom
    } else if mask & RENAME_NEW_NAME != 0 {
        Kind::RenamedTo
    } else if mask & DATA != 0 {
        Kind::Modified
    } else if mask & SECURITY_CHANGE != 0 {
        Kind::SecurityChanged
    } else if mask & STREAM != 0 {
        Kind::StreamChanged
    } else if mask & LINK != 0 {
        Kind::LinkChanged
    } else if mask & ATTRIBUTES != 0 {
        Kind::AttributesChanged
    } else {
        Kind::Other
    }
}

#[cfg(test)]
mod tests {
    use super::usn_reason as r;
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn rdcw_table() {
        let table = [
            (0, Kind::Other),
            (1, Kind::Created),
            (2, Kind::Deleted),
            (3, Kind::Modified),
            (4, Kind::RenamedFrom),
            (5, Kind::RenamedTo),
            (6, Kind::StreamChanged),
            (7, Kind::StreamChanged),
            (8, Kind::StreamChanged),
            (9, Kind::Deleted),
            (10, Kind::Other),
            (11, Kind::Other),
            (12, Kind::Other),
            (u32::MAX, Kind::Other),
        ];
        for (action, kind) in table {
            assert_eq!(rdcw_kind(action), kind, "action {action}");
        }
    }

    #[test]
    fn usn_precedence_rules() {
        let table = [
            (
                r::FILE_DELETE | r::FILE_CREATE | r::DATA_EXTEND,
                Kind::Deleted,
            ),
            (
                r::FILE_CREATE | r::RENAME_NEW_NAME | r::DATA_EXTEND,
                Kind::Created,
            ),
            (
                r::RENAME_OLD_NAME | r::RENAME_NEW_NAME | r::DATA_EXTEND,
                Kind::RenamedFrom,
            ),
            (
                r::RENAME_NEW_NAME | r::DATA_OVERWRITE | r::SECURITY_CHANGE,
                Kind::RenamedTo,
            ),
            (
                r::DATA_TRUNCATION | r::SECURITY_CHANGE | r::STREAM_CHANGE,
                Kind::Modified,
            ),
            (
                r::SECURITY_CHANGE | r::STREAM_CHANGE | r::BASIC_INFO_CHANGE,
                Kind::SecurityChanged,
            ),
            (
                r::NAMED_DATA_EXTEND | r::HARD_LINK_CHANGE | r::BASIC_INFO_CHANGE,
                Kind::StreamChanged,
            ),
            (r::STREAM_CHANGE | r::CLOSE, Kind::StreamChanged),
            (
                r::REPARSE_POINT_CHANGE | r::BASIC_INFO_CHANGE,
                Kind::LinkChanged,
            ),
            (r::HARD_LINK_CHANGE, Kind::LinkChanged),
            (r::BASIC_INFO_CHANGE | r::CLOSE, Kind::AttributesChanged),
            (r::INTEGRITY_CHANGE, Kind::AttributesChanged),
            (r::DESIRED_STORAGE_CLASS_CHANGE, Kind::AttributesChanged),
            (r::CLOSE, Kind::Other),
            (0, Kind::Other),
        ];
        for (mask, kind) in table {
            assert_eq!(usn_kind(mask), kind, "mask {mask:#010x}");
        }
    }

    #[test]
    fn every_single_defined_bit_maps_to_a_documented_kind() {
        for bit in 0..32 {
            let mask = 1_u32 << bit;
            let kind = usn_kind(mask);
            if mask & r::ALL == 0 {
                assert_eq!(kind, Kind::Other, "undefined bit {bit}");
            } else if mask == r::CLOSE {
                assert_eq!(kind, Kind::Other);
            } else {
                assert_ne!(kind, Kind::Other, "defined bit {bit} must classify");
            }
        }
    }

    fn or_all(bits: &[u32]) -> u32 {
        bits.iter().fold(0, |acc, b| acc | b)
    }

    #[test]
    fn group_masks_are_the_union_of_their_members() {
        assert_eq!(
            r::DATA,
            or_all(&[r::DATA_OVERWRITE, r::DATA_EXTEND, r::DATA_TRUNCATION])
        );
        assert_eq!(
            r::STREAM,
            or_all(&[
                r::NAMED_DATA_OVERWRITE,
                r::NAMED_DATA_EXTEND,
                r::NAMED_DATA_TRUNCATION,
                r::STREAM_CHANGE,
            ])
        );
        assert_eq!(
            r::LINK,
            or_all(&[r::HARD_LINK_CHANGE, r::REPARSE_POINT_CHANGE])
        );
        assert_eq!(
            r::ATTRIBUTES,
            or_all(&[
                r::BASIC_INFO_CHANGE,
                r::EA_CHANGE,
                r::INDEXABLE_CHANGE,
                r::COMPRESSION_CHANGE,
                r::ENCRYPTION_CHANGE,
                r::OBJECT_ID_CHANGE,
                r::TRANSACTED_CHANGE,
                r::INTEGRITY_CHANGE,
                r::DESIRED_STORAGE_CLASS_CHANGE,
            ])
        );
    }

    #[test]
    fn all_is_the_union_of_every_defined_bit() {
        let every = [
            r::DATA_OVERWRITE,
            r::DATA_EXTEND,
            r::DATA_TRUNCATION,
            r::NAMED_DATA_OVERWRITE,
            r::NAMED_DATA_EXTEND,
            r::NAMED_DATA_TRUNCATION,
            r::FILE_CREATE,
            r::FILE_DELETE,
            r::EA_CHANGE,
            r::SECURITY_CHANGE,
            r::RENAME_OLD_NAME,
            r::RENAME_NEW_NAME,
            r::INDEXABLE_CHANGE,
            r::BASIC_INFO_CHANGE,
            r::HARD_LINK_CHANGE,
            r::COMPRESSION_CHANGE,
            r::ENCRYPTION_CHANGE,
            r::OBJECT_ID_CHANGE,
            r::REPARSE_POINT_CHANGE,
            r::STREAM_CHANGE,
            r::TRANSACTED_CHANGE,
            r::INTEGRITY_CHANGE,
            r::DESIRED_STORAGE_CLASS_CHANGE,
            r::CLOSE,
        ];
        assert_eq!(r::ALL, or_all(&every));
        for bit in every {
            assert_eq!(bit.count_ones(), 1);
        }
    }

    proptest! {
        #[test]
        fn mappings_are_total(x in any::<u32>()) {
            let _ = rdcw_kind(x);
            let _ = usn_kind(x);
        }

        #[test]
        fn delete_and_create_win(x in any::<u32>()) {
            prop_assert_eq!(usn_kind(x | r::FILE_DELETE), Kind::Deleted);
            prop_assert_eq!(usn_kind((x | r::FILE_CREATE) & !r::FILE_DELETE), Kind::Created);
        }

        #[test]
        fn close_never_changes_the_kind(x in any::<u32>()) {
            prop_assert_eq!(usn_kind(x | r::CLOSE), usn_kind(x));
        }
    }
}
