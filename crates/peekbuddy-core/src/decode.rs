//! Raw OS values in words (#69): file attributes, reparse tags, RDCW actions
//! and USN reasons.
//!
//! The details pane shows what Windows reported, not only what PeekBuddy made
//! of it. Every decoder is a pure function and follows one rule: a bit or a
//! value it does not know is shown as hex, never dropped, so an evidence row
//! is never shortened by a table that is out of date.

/// `FILE_ATTRIBUTE_*` bits with their names, in bit order.
const ATTRIBUTES: &[(u32, &str)] = &[
    (0x0000_0001, "READONLY"),
    (0x0000_0002, "HIDDEN"),
    (0x0000_0004, "SYSTEM"),
    (0x0000_0010, "DIRECTORY"),
    (0x0000_0020, "ARCHIVE"),
    (0x0000_0040, "DEVICE"),
    (0x0000_0080, "NORMAL"),
    (0x0000_0100, "TEMPORARY"),
    (0x0000_0200, "SPARSE_FILE"),
    (0x0000_0400, "REPARSE_POINT"),
    (0x0000_0800, "COMPRESSED"),
    (0x0000_1000, "OFFLINE"),
    (0x0000_2000, "NOT_CONTENT_INDEXED"),
    (0x0000_4000, "ENCRYPTED"),
    (0x0000_8000, "INTEGRITY_STREAM"),
    (0x0001_0000, "VIRTUAL"),
    (0x0002_0000, "NO_SCRUB_DATA"),
    (0x0004_0000, "RECALL_ON_OPEN"),
    (0x0008_0000, "PINNED"),
    (0x0010_0000, "UNPINNED"),
    (0x0040_0000, "RECALL_ON_DATA_ACCESS"),
];

/// `FILE_ACTION_*` values of `ReadDirectoryChangesW`.
const ACTIONS: &[(u32, &str)] = &[
    (1, "FILE_ACTION_ADDED"),
    (2, "FILE_ACTION_REMOVED"),
    (3, "FILE_ACTION_MODIFIED"),
    (4, "FILE_ACTION_RENAMED_OLD_NAME"),
    (5, "FILE_ACTION_RENAMED_NEW_NAME"),
    (6, "FILE_ACTION_ADDED_STREAM"),
    (7, "FILE_ACTION_REMOVED_STREAM"),
    (8, "FILE_ACTION_MODIFIED_STREAM"),
    (9, "FILE_ACTION_REMOVED_BY_DELETE"),
    (10, "FILE_ACTION_ID_NOT_TUNNELLED"),
    (11, "FILE_ACTION_TUNNELLED_ID_COLLISION"),
];

/// `USN_REASON_*` bits.
const REASONS: &[(u32, &str)] = &[
    (0x0000_0001, "DATA_OVERWRITE"),
    (0x0000_0002, "DATA_EXTEND"),
    (0x0000_0004, "DATA_TRUNCATION"),
    (0x0000_0010, "NAMED_DATA_OVERWRITE"),
    (0x0000_0020, "NAMED_DATA_EXTEND"),
    (0x0000_0040, "NAMED_DATA_TRUNCATION"),
    (0x0000_0100, "FILE_CREATE"),
    (0x0000_0200, "FILE_DELETE"),
    (0x0000_0400, "EA_CHANGE"),
    (0x0000_0800, "SECURITY_CHANGE"),
    (0x0000_1000, "RENAME_OLD_NAME"),
    (0x0000_2000, "RENAME_NEW_NAME"),
    (0x0000_4000, "INDEXABLE_CHANGE"),
    (0x0000_8000, "BASIC_INFO_CHANGE"),
    (0x0001_0000, "HARD_LINK_CHANGE"),
    (0x0002_0000, "COMPRESSION_CHANGE"),
    (0x0004_0000, "ENCRYPTION_CHANGE"),
    (0x0008_0000, "OBJECT_ID_CHANGE"),
    (0x0010_0000, "REPARSE_POINT_CHANGE"),
    (0x0020_0000, "STREAM_CHANGE"),
    (0x0040_0000, "TRANSACTED_CHANGE"),
    (0x0080_0000, "INTEGRITY_CHANGE"),
    (0x8000_0000, "CLOSE"),
];

/// Reparse tags with their names.
const TAGS: &[(u32, &str)] = &[
    (0xA000_0003, "IO_REPARSE_TAG_MOUNT_POINT (junction)"),
    (0xA000_000C, "IO_REPARSE_TAG_SYMLINK"),
    (0x8000_0017, "IO_REPARSE_TAG_WOF"),
    (0x8000_001B, "IO_REPARSE_TAG_APPEXECLINK"),
    (0x9000_001C, "IO_REPARSE_TAG_PROJFS"),
    (0xA000_001D, "IO_REPARSE_TAG_LX_SYMLINK"),
    (0x9000_001A, "IO_REPARSE_TAG_CLOUD"),
];

/// The names of the set bits of `value` in `table`, then the bits no name
/// covers as one hex number.
fn flag_names(value: u32, table: &[(u32, &str)]) -> Vec<String> {
    let mut names: Vec<String> = table
        .iter()
        .filter(|(bit, _)| value & bit != 0)
        .map(|(_, name)| (*name).to_owned())
        .collect();
    let known: u32 = table.iter().fold(0, |all, (bit, _)| all | bit);
    let rest = value & !known;
    if rest != 0 {
        names.push(format!("{rest:#010x}"));
    }
    names
}

/// The `FILE_ATTRIBUTE_*` names in `attrs`; unknown bits as hex.
#[must_use]
pub fn attributes(attrs: u32) -> Vec<String> {
    flag_names(attrs, ATTRIBUTES)
}

/// The `USN_REASON_*` names in `mask`; unknown bits as hex.
#[must_use]
pub fn usn_reasons(mask: u32) -> Vec<String> {
    flag_names(mask, REASONS)
}

/// The `FILE_ACTION_*` name of an RDCW action, or its number as hex.
#[must_use]
pub fn rdcw_action(action: u32) -> String {
    ACTIONS
        .iter()
        .find(|(value, _)| *value == action)
        .map_or_else(|| format!("{action:#010x}"), |(_, name)| (*name).to_owned())
}

/// The name of a reparse tag, or the tag as hex. Cloud placeholder tags carry
/// the sync state in a nibble (`0x9000_?01A`): they read as the cloud tag.
#[must_use]
pub fn reparse_tag(tag: u32) -> String {
    if tag == 0 {
        return String::from("none");
    }
    let plain = TAGS.iter().find(|(value, _)| *value == tag);
    let cloud = crate::reparse::is_cloud(tag);
    match (plain, cloud) {
        (Some((_, name)), _) => (*name).to_owned(),
        (None, true) => format!("IO_REPARSE_TAG_CLOUD ({tag:#010x})"),
        (None, false) => format!("{tag:#010x}"),
    }
}

/// A cell for a tab-separated row: tabs, line ends and other control
/// characters are replaced by their `\t`, `\r`, `\n` or `\u{..}` spelling, so a
/// name cannot start another column or another row. Backslashes stay as they
/// are: a Windows path pasted from a copied row must still be a path.
#[must_use]
pub fn tsv_cell(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            c if c.is_control() => {
                use std::fmt::Write as _;
                let _ = write!(out, "\\u{{{:04X}}}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{
        ACTIONS, ATTRIBUTES, REASONS, TAGS, attributes, rdcw_action, reparse_tag, tsv_cell,
        usn_reasons,
    };

    #[test]
    fn known_attribute_bits_have_names_and_unknown_ones_stay_as_hex() {
        assert_eq!(attributes(0), Vec::<String>::new());
        assert_eq!(attributes(0x10 | 0x20), ["DIRECTORY", "ARCHIVE"]);
        // The cloud-related bits the issue names.
        assert_eq!(
            attributes(0x1000 | 0x4_0000 | 0x8_0000 | 0x10_0000 | 0x40_0000),
            [
                "OFFLINE",
                "RECALL_ON_OPEN",
                "PINNED",
                "UNPINNED",
                "RECALL_ON_DATA_ACCESS"
            ]
        );
        // 0x8 and 0x2000_0000 are not attributes any table knows.
        assert_eq!(
            attributes(0x1 | 0x8 | 0x2000_0000),
            ["READONLY", "0x20000008"]
        );
    }

    #[test]
    fn the_tables_have_no_duplicate_values_and_single_bits_only_where_flags() {
        for table in [ATTRIBUTES, REASONS] {
            let mut seen = std::collections::HashSet::new();
            for (bit, name) in table {
                assert_eq!(bit.count_ones(), 1, "{name} is not one bit");
                assert!(seen.insert(*bit), "{name} repeats a bit");
            }
        }
        for table in [ACTIONS, TAGS] {
            let mut seen = std::collections::HashSet::new();
            for (value, name) in table {
                assert!(seen.insert(*value), "{name} repeats a value");
            }
        }
    }

    #[test]
    fn usn_reasons_are_named_by_bit_and_close_is_the_top_bit() {
        assert_eq!(usn_reasons(0x100 | 0x8000_0000), ["FILE_CREATE", "CLOSE"]);
        assert_eq!(usn_reasons(0x2), ["DATA_EXTEND"]);
        assert_eq!(usn_reasons(0x8), ["0x00000008"]);
    }

    #[test]
    fn rdcw_actions_and_reparse_tags_are_named_or_hex() {
        assert_eq!(rdcw_action(1), "FILE_ACTION_ADDED");
        assert_eq!(rdcw_action(9), "FILE_ACTION_REMOVED_BY_DELETE");
        assert_eq!(rdcw_action(99), "0x00000063");
        assert_eq!(reparse_tag(0), "none");
        assert_eq!(reparse_tag(0xA000_000C), "IO_REPARSE_TAG_SYMLINK");
        assert!(reparse_tag(0xA000_0003).contains("junction"));
        // A cloud tag with a sync-state nibble still reads as the cloud tag.
        assert!(reparse_tag(0x9000_601A).starts_with("IO_REPARSE_TAG_CLOUD"));
        assert_eq!(reparse_tag(0x1234_5678), "0x12345678");
    }

    #[test]
    fn a_cell_cannot_start_another_column_or_row() {
        assert_eq!(tsv_cell("a\tb"), "a\\tb");
        assert_eq!(tsv_cell("a\r\nb"), "a\\r\\nb");
        assert_eq!(
            tsv_cell("C:\\work\\a.txt"),
            "C:\\work\\a.txt",
            "paths stay pasteable"
        );
        assert_eq!(tsv_cell("bell\u{7}"), "bell\\u{0007}");
        assert_eq!(tsv_cell("plain ünï"), "plain ünï");
    }

    proptest! {
        #[test]
        fn a_cell_never_holds_a_tab_or_a_line_end(text in "\\PC{0,64}|[\\t\\r\\n a-z\\\\]{0,32}") {
            let cell = tsv_cell(&text);
            prop_assert!(!cell.contains(['\t', '\r', '\n']));
        }

        #[test]
        fn every_set_bit_is_either_named_or_in_the_hex_remainder(value in any::<u32>()) {
            let names = attributes(value);
            let named: u32 = ATTRIBUTES.iter().filter(|(bit, _)| value & bit != 0).fold(0, |a, (b, _)| a | b);
            let rest = value & !ATTRIBUTES.iter().fold(0, |a, (b, _)| a | b);
            prop_assert_eq!(names.len(), named.count_ones() as usize + usize::from(rest != 0));
        }
    }
}
