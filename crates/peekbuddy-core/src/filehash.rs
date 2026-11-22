//! Hash on demand (#85): the SHA-256 of a file the user picked, never of a
//! file whose data is not on this machine.
//!
//! The viewer hashes the file at a row's path when the user asks, on a thread
//! of its own, with a fixed buffer and a cancel button. A cloud placeholder
//! (data in the cloud, [`is_placeholder`]) is refused, because reading it
//! would download it (F18). The reading and hashing are in
//! `peekbuddy_win::hash`; this module holds what they share with the viewer.

use crate::reparse::attr;

/// `FILE_ATTRIBUTE_RECALL_ON_OPEN`: opening the file fetches its data.
pub const RECALL_ON_OPEN: u32 = 0x0004_0000;

/// The attributes of a file whose data is not local: reading it would recall
/// it from the cloud or from offline storage.
pub const PLACEHOLDER: u32 = attr::OFFLINE | RECALL_ON_OPEN | attr::RECALL_ON_DATA_ACCESS;

/// The bytes read per step: the buffer is fixed, whatever the file's size.
pub const CHUNK: usize = 64 * 1024;

/// Whether a file with `attrs` must not be read.
#[must_use]
pub const fn is_placeholder(attrs: u32) -> bool {
    attrs & PLACEHOLDER != 0
}

/// How hashing a file ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The SHA-256 of the whole file.
    Done([u8; 32]),
    /// Stopped by the user; nothing was kept.
    Cancelled,
    /// A placeholder: not read, so not downloaded.
    Placeholder,
}

/// `digest` in lowercase hex, as `sha256sum` and `Get-FileHash` (which
/// prints uppercase) show it.
#[must_use]
pub fn hex(digest: &[u8; 32]) -> String {
    use std::fmt::Write as _;
    digest
        .iter()
        .fold(String::with_capacity(64), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

#[cfg(test)]
mod tests {
    use super::{PLACEHOLDER, hex, is_placeholder};
    use crate::reparse::attr;

    #[test]
    fn a_file_whose_data_is_not_local_is_a_placeholder() {
        assert!(is_placeholder(attr::OFFLINE));
        assert!(is_placeholder(0x0004_0000));
        assert!(is_placeholder(0x20 | attr::RECALL_ON_DATA_ACCESS));
        // Pinned or sparse alone: the data is here.
        assert!(!is_placeholder(0x20 | attr::PINNED | attr::SPARSE));
        assert_eq!(PLACEHOLDER, 0x0044_1000);
    }

    #[test]
    fn a_digest_is_written_as_64_lowercase_hex_digits() {
        let mut digest = [0_u8; 32];
        digest[0] = 0xAB;
        digest[31] = 0x0F;
        let text = hex(&digest);
        assert_eq!(text.len(), 64);
        assert!(text.starts_with("ab00"), "{text}");
        assert!(text.ends_with("000f"), "{text}");
    }
}
