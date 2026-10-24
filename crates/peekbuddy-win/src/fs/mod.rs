//! File-system calls std does not offer (#25): junctions, the archive
//! attribute and volume facts.
//!
//! The byte layouts are plain functions that build and test on every host;
//! the Win32 calls are Windows-only. Paths are passed to Win32 in the `\\?\`
//! form, so long paths (> 260) and lone surrogates (F17) work.

#[cfg(windows)]
mod win;

#[cfg(windows)]
pub use win::*;

/// `IO_REPARSE_TAG_MOUNT_POINT` (kept local to avoid the large
/// `Win32_System_SystemServices` feature).
pub const IO_REPARSE_TAG_MOUNT_POINT: u32 = 0xA000_0003;

/// `MAXIMUM_REPARSE_DATA_BUFFER_SIZE`.
const MAX_REPARSE_BYTES: usize = 16 * 1024;

/// `FILE_ATTRIBUTE_ARCHIVE`.
pub const ATTRIBUTE_ARCHIVE: u32 = 0x20;

/// `FILE_ATTRIBUTE_NORMAL`: valid only on its own.
pub const ATTRIBUTE_NORMAL: u32 = 0x80;

/// The `REPARSE_DATA_BUFFER` for a junction (mount point): `substitute` is
/// the NT path (`\??\C:\dir`), `print` the display path (`C:\dir`). Both
/// are stored NUL-terminated, substitute first. `None` when the buffer
/// would exceed `MAXIMUM_REPARSE_DATA_BUFFER_SIZE`.
#[must_use]
pub fn mount_point_buffer(substitute: &[u16], print: &[u16]) -> Option<Vec<u8>> {
    let substitute_bytes = u16::try_from(substitute.len().checked_mul(2)?).ok()?;
    let print_bytes = u16::try_from(print.len().checked_mul(2)?).ok()?;
    // The four offset/length fields, then both names with their NULs.
    let data_len = 8 + usize::from(substitute_bytes) + usize::from(print_bytes) + 4;
    if 8 + data_len > MAX_REPARSE_BYTES {
        return None;
    }
    let mut out = Vec::with_capacity(8 + data_len);
    out.extend_from_slice(&IO_REPARSE_TAG_MOUNT_POINT.to_le_bytes());
    out.extend_from_slice(&u16::try_from(data_len).ok()?.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&substitute_bytes.to_le_bytes());
    out.extend_from_slice(&(substitute_bytes + 2).to_le_bytes());
    out.extend_from_slice(&print_bytes.to_le_bytes());
    for unit in substitute.iter().chain(&[0]).chain(print).chain(&[0]) {
        out.extend_from_slice(&unit.to_le_bytes());
    }
    Some(out)
}

/// `attributes` with the archive bit flipped, in a form `SetFileAttributesW`
/// accepts: `FILE_ATTRIBUTE_NORMAL` only on its own, never 0.
#[must_use]
pub fn toggled_archive(attributes: u32) -> u32 {
    let flipped = (attributes & !ATTRIBUTE_NORMAL) ^ ATTRIBUTE_ARCHIVE;
    if flipped == 0 {
        ATTRIBUTE_NORMAL
    } else {
        flipped
    }
}

/// The `\\?\` form of an absolute path: drive paths get `\\?\`, UNC paths
/// `\\?\UNC\`, paths already in device form stay as they are.
#[must_use]
pub fn verbatim(absolute: &[u16]) -> Vec<u16> {
    let backslash = u16::from(b'\\');
    let wide = |text: &str| text.encode_utf16().collect::<Vec<u16>>();
    let mut out = Vec::with_capacity(absolute.len() + 8);
    if absolute.starts_with(&wide(r"\\?\")) || absolute.starts_with(&wide(r"\??\")) {
        out.extend_from_slice(absolute);
    } else if let Some(rest) = absolute.strip_prefix(&[backslash, backslash][..]) {
        out.extend(wide(r"\\?\UNC\"));
        out.extend_from_slice(rest);
    } else {
        out.extend(wide(r"\\?\"));
        out.extend_from_slice(absolute);
    }
    out
}

/// Whether `short_path` (from `GetShortPathNameW`) ends in a name other
/// than `long`: the file got an 8.3 name.
#[must_use]
pub fn has_short_name(short_path: &[u16], long: &str) -> bool {
    let name = short_path
        .rsplit(|&unit| unit == u16::from(b'\\'))
        .next()
        .unwrap_or_default();
    !name.iter().copied().eq(long.encode_utf16())
}

/// `FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE`. A literal, so
/// that no operator in it can be mutated unseen; a Windows test compares it
/// with windows-sys.
pub const SHARE_ALL: u32 = 0x0000_0007;

/// `FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT |
/// FILE_FLAG_OPEN_NO_RECALL`: open directories, do not follow reparse points,
/// do not recall cloud files (F18). A literal, checked on Windows like
/// [`SHARE_ALL`].
pub const NO_FOLLOW_NO_RECALL: u32 = 0x0230_0000;

/// How `OpenFileById` wants a file id (#39).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileIdKind {
    /// A 64-bit id (`FileIdType`), as NTFS reports it.
    Narrow(i64),
    /// A 128-bit id (`ExtendedFileIdType`), as `ReFS` reports it.
    Extended([u8; 16]),
}

/// The descriptor kind for a widened file id: ids that fit a positive `i64`
/// are narrow, everything else is passed as the full 128 bits.
#[must_use]
pub fn file_id_kind(id: u128) -> FileIdKind {
    i64::try_from(id).map_or_else(
        |_| FileIdKind::Extended(id.to_le_bytes()),
        FileIdKind::Narrow,
    )
}

/// `FILE_CS_FLAG_CASE_SENSITIVE_DIR`.
const FILE_CS_FLAG_CASE_SENSITIVE_DIR: u32 = 1;

/// The case rule for the `Flags` of `FILE_CASE_SENSITIVE_INFO`.
#[must_use]
pub fn case_mode_from_flags(flags: u32) -> peekbuddy_core::name::CaseMode {
    if flags & FILE_CS_FLAG_CASE_SENSITIVE_DIR == 0 {
        peekbuddy_core::name::CaseMode::Insensitive
    } else {
        peekbuddy_core::name::CaseMode::Sensitive
    }
}

/// Whether a failed case-sensitivity query only means that the volume has no
/// per-directory case sensitivity (`ERROR_INVALID_FUNCTION`,
/// `ERROR_NOT_SUPPORTED` or `ERROR_INVALID_PARAMETER`): such a directory is
/// case-insensitive.
#[must_use]
pub const fn means_no_case_sensitivity(os_error: i32) -> bool {
    matches!(os_error, 1 | 50 | 87)
}

/// The junction names for an absolute target: the NT substitute name
/// (`\??\` + path) and the print name (the path without any `\\?\`).
#[must_use]
pub fn junction_names(absolute_target: &[u16]) -> (Vec<u16>, Vec<u16>) {
    let device: Vec<u16> = r"\\?\".encode_utf16().collect();
    let print = absolute_target
        .strip_prefix(&device[..])
        .unwrap_or(absolute_target)
        .to_vec();
    let mut substitute: Vec<u16> = r"\??\".encode_utf16().collect();
    substitute.extend_from_slice(&print);
    (substitute, print)
}

#[cfg(test)]
#[expect(
    clippy::indexing_slicing,
    reason = "tests index buffers of known size; a panic is a test failure"
)]
mod tests {
    use peekbuddy_core::name::CaseMode;

    use super::{
        ATTRIBUTE_ARCHIVE, ATTRIBUTE_NORMAL, FileIdKind, IO_REPARSE_TAG_MOUNT_POINT,
        case_mode_from_flags, file_id_kind, has_short_name, junction_names,
        means_no_case_sensitivity, mount_point_buffer, toggled_archive, verbatim,
    };

    fn w(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    fn u16_at(bytes: &[u8], at: usize) -> u16 {
        u16::from_le_bytes([bytes[at], bytes[at + 1]])
    }

    #[test]
    fn mount_point_layout() {
        let buffer = mount_point_buffer(&w(r"\??\C:\t"), &w(r"C:\t")).unwrap();
        // Header (8) + offsets (8) + "\??\C:\t" NUL (18) + "C:\t" NUL (10).
        assert_eq!(buffer.len(), 44);
        assert_eq!(
            u32::from_le_bytes(buffer[..4].try_into().unwrap()),
            IO_REPARSE_TAG_MOUNT_POINT
        );
        assert_eq!(u16_at(&buffer, 4), 36, "ReparseDataLength");
        assert_eq!(u16_at(&buffer, 6), 0, "Reserved");
        assert_eq!(u16_at(&buffer, 8), 0, "SubstituteNameOffset");
        assert_eq!(u16_at(&buffer, 10), 16, "SubstituteNameLength");
        assert_eq!(u16_at(&buffer, 12), 18, "PrintNameOffset");
        assert_eq!(u16_at(&buffer, 14), 8, "PrintNameLength");
        let units: Vec<u16> = buffer[16..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .collect();
        let mut expected = w(r"\??\C:\t");
        expected.push(0);
        expected.extend(w(r"C:\t"));
        expected.push(0);
        assert_eq!(units, expected);
    }

    #[test]
    fn mount_point_size_limit() {
        // 16 KiB total: 16 bytes of fields, 4 of NULs, the rest names.
        let fits = vec![u16::from(b'a'); (16 * 1024 - 20) / 4];
        assert!(mount_point_buffer(&fits, &fits).is_some());
        let mut too_long = fits.clone();
        too_long.push(u16::from(b'a'));
        assert!(mount_point_buffer(&too_long, &fits).is_none());
        assert!(mount_point_buffer(&vec![0; 40_000], &[]).is_none());
    }

    #[test]
    fn archive_toggle() {
        assert_eq!(toggled_archive(ATTRIBUTE_NORMAL), ATTRIBUTE_ARCHIVE);
        assert_eq!(toggled_archive(ATTRIBUTE_ARCHIVE), ATTRIBUTE_NORMAL);
        // Read-only (0x1) stays, archive flips both ways.
        assert_eq!(toggled_archive(0x1), 0x21);
        assert_eq!(toggled_archive(0x21), 0x1);
        assert_eq!(toggled_archive(0), ATTRIBUTE_ARCHIVE);
    }

    #[test]
    fn verbatim_prefixes() {
        assert_eq!(verbatim(&w(r"C:\a\b")), w(r"\\?\C:\a\b"));
        assert_eq!(verbatim(&w(r"\\srv\share\a")), w(r"\\?\UNC\srv\share\a"));
        assert_eq!(verbatim(&w(r"\\?\C:\a")), w(r"\\?\C:\a"));
        assert_eq!(verbatim(&w(r"\??\C:\a")), w(r"\??\C:\a"));
        // Lone surrogates pass through untouched.
        assert_eq!(verbatim(&[0x43, 0x3A, 0x5C, 0xD800]).last(), Some(&0xD800));
    }

    #[test]
    fn short_names_are_told_apart_by_the_last_component() {
        let long = "PeekBuddy probe.tmp";
        assert!(!has_short_name(&w(r"C:\t\PeekBuddy probe.tmp"), long));
        assert!(has_short_name(&w(r"C:\t\PEEKBU~1.TMP"), long));
        // Only the last component counts.
        assert!(!has_short_name(
            &w(r"C:\PROGRA~1\PeekBuddy probe.tmp"),
            long
        ));
        assert!(!has_short_name(&w("PeekBuddy probe.tmp"), long));
    }

    #[test]
    fn junction_target_names() {
        assert_eq!(
            junction_names(&w(r"C:\t\d1")),
            (w(r"\??\C:\t\d1"), w(r"C:\t\d1"))
        );
        assert_eq!(
            junction_names(&w(r"\\?\C:\t\d1")),
            (w(r"\??\C:\t\d1"), w(r"C:\t\d1"))
        );
    }

    #[test]
    fn file_ids_pick_the_narrow_form_when_they_fit() {
        assert_eq!(file_id_kind(0), FileIdKind::Narrow(0));
        assert_eq!(file_id_kind(5), FileIdKind::Narrow(5));
        assert_eq!(
            file_id_kind(u128::from(i64::MAX.unsigned_abs())),
            FileIdKind::Narrow(i64::MAX)
        );
        let wide = 1_u128 << 63;
        assert_eq!(file_id_kind(wide), FileIdKind::Extended(wide.to_le_bytes()));
        let refs = (7_u128 << 64) | 9;
        assert_eq!(file_id_kind(refs), FileIdKind::Extended(refs.to_le_bytes()));
        assert_eq!(file_id_kind(u128::MAX), FileIdKind::Extended([0xFF; 16]));
    }

    #[test]
    fn case_flags_and_unsupported_errors() {
        assert_eq!(case_mode_from_flags(0), CaseMode::Insensitive);
        assert_eq!(case_mode_from_flags(1), CaseMode::Sensitive);
        assert_eq!(case_mode_from_flags(0xFFFF_FFFE), CaseMode::Insensitive);
        assert_eq!(case_mode_from_flags(0xFFFF_FFFF), CaseMode::Sensitive);
        for code in [1, 50, 87] {
            assert!(means_no_case_sensitivity(code));
        }
        for code in [0, 2, 3, 5, 122] {
            assert!(!means_no_case_sensitivity(code));
        }
    }
}
