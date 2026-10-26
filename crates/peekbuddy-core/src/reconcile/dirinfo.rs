//! Zero-copy parser for `FILE_ID_EXTD_DIR_INFO` buffers (#46).
//!
//! `GetFileInformationByHandleEx(FileIdExtdDirectoryInfo)` fills a buffer
//! with a chain of variable-length records, one per directory entry. The
//! parser follows the design of the notify parser ([`crate::notify`]): every
//! field is decoded by byte offset, nothing is cast, nothing is `unsafe`, and
//! every record is validated before it is yielded.
//!
//! # Validation
//!
//! 1. the fixed header fits in the rest of the buffer
//!    ([`ErrorKind::Truncated`]);
//! 2. `FileNameLength` is even ([`ErrorKind::OddNameLength`]);
//! 3. the name fits ([`ErrorKind::NameOutOfBounds`]);
//! 4. `NextEntryOffset` is 0 (the last record) or a multiple of 8
//!    ([`ErrorKind::MisalignedNext`]), at least the size of the current
//!    record ([`ErrorKind::NextTooSmall`]) and below the end of the buffer
//!    ([`ErrorKind::NextPastEnd`]);
//! 5. at most `max(1, len / header size)` records are read
//!    ([`ErrorKind::TooManyRecords`]).
//!
//! A malformed record ends the iteration with a [`ParseError`] carrying its
//! offset. Records before it stay valid. `.` and `..` are not skipped here;
//! [`Entry::is_dot`] tells the caller.

use crate::notify::Utf16Name;

/// Byte offsets of the layout, checked against `windows-sys` by a Windows
/// test.
pub mod layout {
    /// `NextEntryOffset`.
    pub const NEXT_ENTRY_OFFSET: usize = 0;
    /// `FileIndex`.
    pub const FILE_INDEX: usize = 4;
    /// `CreationTime`.
    pub const CREATION_TIME: usize = 8;
    /// `LastAccessTime`.
    pub const ACCESS_TIME: usize = 16;
    /// `LastWriteTime`.
    pub const WRITE_TIME: usize = 24;
    /// `ChangeTime`.
    pub const CHANGE_TIME: usize = 32;
    /// `EndOfFile`.
    pub const END_OF_FILE: usize = 40;
    /// `AllocationSize`.
    pub const ALLOCATION_SIZE: usize = 48;
    /// `FileAttributes`.
    pub const ATTRIBUTES: usize = 56;
    /// `FileNameLength`.
    pub const NAME_LENGTH: usize = 60;
    /// `EaSize`.
    pub const EA_SIZE: usize = 64;
    /// `ReparsePointTag`.
    pub const REPARSE_TAG: usize = 68;
    /// `FileId` (a 128-bit id, little endian).
    pub const FILE_ID: usize = 72;
    /// `FileName`.
    pub const NAME: usize = 88;
}

/// The size of the fixed header, where the name starts.
pub const HEADER_LEN: usize = layout::NAME;
/// Records are 8-byte aligned.
pub const ALIGN: usize = 8;

/// `FILE_ATTRIBUTE_DIRECTORY`.
pub const ATTR_DIRECTORY: u32 = 0x10;
/// `FILE_ATTRIBUTE_REPARSE_POINT`.
pub const ATTR_REPARSE_POINT: u32 = 0x400;

/// One directory entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry<'a> {
    /// The name, relative to the listed directory.
    pub name: Utf16Name<'a>,
    /// `CreationTime` (FILETIME ticks).
    pub creation_time: i64,
    /// `LastWriteTime`.
    pub write_time: i64,
    /// `ChangeTime`.
    pub change_time: i64,
    /// `EndOfFile`.
    pub size: i64,
    /// `FileAttributes`.
    pub attributes: u32,
    /// `ReparsePointTag`.
    pub reparse_tag: u32,
    /// `FileId`, 128 bits.
    pub file_id: u128,
}

impl Entry<'_> {
    /// Whether this is `.` or `..`.
    #[must_use]
    pub fn is_dot(&self) -> bool {
        let mut units = self.name.units();
        matches!(
            (units.next(), units.next(), units.next()),
            (Some(46), None, _) | (Some(46), Some(46), None)
        )
    }

    /// Whether the entry is a directory.
    #[must_use]
    pub const fn is_dir(&self) -> bool {
        self.attributes & ATTR_DIRECTORY != 0
    }

    /// Whether the entry is a reparse point (junction, symlink, placeholder).
    #[must_use]
    pub const fn is_reparse(&self) -> bool {
        self.attributes & ATTR_REPARSE_POINT != 0
    }
}

/// Why a record is malformed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// The fixed header does not fit.
    Truncated,
    /// `FileNameLength` is odd.
    OddNameLength,
    /// The name does not fit.
    NameOutOfBounds,
    /// `NextEntryOffset` is not a multiple of 8.
    MisalignedNext,
    /// `NextEntryOffset` is smaller than the current record.
    NextTooSmall,
    /// `NextEntryOffset` points at or past the end of the buffer.
    NextPastEnd,
    /// More records than `max(1, len / header size)`.
    TooManyRecords,
    /// `bytes_returned` is larger than the buffer.
    BadLength,
}

/// A malformed record: the iteration ends here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseError {
    /// The byte offset of the record.
    pub offset: usize,
    /// What is wrong with it.
    pub kind: ErrorKind,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "malformed directory record at byte {}: {:?}",
            self.offset, self.kind
        )
    }
}

impl std::error::Error for ParseError {}

/// The iterator over the entries of one buffer.
#[derive(Debug, Clone)]
pub struct Entries<'a> {
    buf: &'a [u8],
    next: Option<usize>,
    budget: usize,
    bad_length: bool,
}

/// Parses `buf[..bytes_returned]`; zero bytes yield nothing.
#[must_use]
pub fn parse(buf: &[u8], bytes_returned: usize) -> Entries<'_> {
    let (data, bad_length) = match buf.get(..bytes_returned) {
        Some(data) => (data, false),
        None => (buf, true),
    };
    Entries {
        buf: data,
        next: (!data.is_empty() || bad_length).then_some(0),
        budget: (data.len() / HEADER_LEN).max(1),
        bad_length,
    }
}

impl<'a> Iterator for Entries<'a> {
    type Item = Result<Entry<'a>, ParseError>;

    fn next(&mut self) -> Option<Self::Item> {
        let offset = self.next.take()?;
        if self.bad_length {
            self.bad_length = false;
            return Some(Err(ParseError {
                offset,
                kind: ErrorKind::BadLength,
            }));
        }
        let Some(budget) = self.budget.checked_sub(1) else {
            return Some(Err(ParseError {
                offset,
                kind: ErrorKind::TooManyRecords,
            }));
        };
        self.budget = budget;
        let record = self.buf.get(offset..).unwrap_or_default();
        match decode(record) {
            Ok((entry, next)) => {
                self.next = next.map(|step| offset.saturating_add(step));
                Some(Ok(entry))
            }
            Err(kind) => Some(Err(ParseError { offset, kind })),
        }
    }
}

fn le<const N: usize>(bytes: &[u8], at: usize) -> Option<[u8; N]> {
    bytes.get(at..)?.first_chunk::<N>().copied()
}

fn decode(record: &[u8]) -> Result<(Entry<'_>, Option<usize>), ErrorKind> {
    if record.len() < HEADER_LEN {
        return Err(ErrorKind::Truncated);
    }
    let u32_at = |at| le::<4>(record, at).map(u32::from_le_bytes);
    let i64_at = |at| le::<8>(record, at).map(i64::from_le_bytes);
    let (Some(next), Some(name_len), Some(attributes), Some(reparse_tag)) = (
        u32_at(layout::NEXT_ENTRY_OFFSET),
        u32_at(layout::NAME_LENGTH),
        u32_at(layout::ATTRIBUTES),
        u32_at(layout::REPARSE_TAG),
    ) else {
        return Err(ErrorKind::Truncated);
    };
    let (Some(creation_time), Some(write_time), Some(change_time), Some(size), Some(id)) = (
        i64_at(layout::CREATION_TIME),
        i64_at(layout::WRITE_TIME),
        i64_at(layout::CHANGE_TIME),
        i64_at(layout::END_OF_FILE),
        le::<16>(record, layout::FILE_ID),
    ) else {
        return Err(ErrorKind::Truncated);
    };
    let name_len = usize::try_from(name_len).map_err(|_| ErrorKind::NameOutOfBounds)?;
    if name_len % 2 != 0 {
        return Err(ErrorKind::OddNameLength);
    }
    let name_end = HEADER_LEN
        .checked_add(name_len)
        .ok_or(ErrorKind::NameOutOfBounds)?;
    let name = record
        .get(HEADER_LEN..name_end)
        .ok_or(ErrorKind::NameOutOfBounds)?;
    let next = usize::try_from(next).map_err(|_| ErrorKind::NextPastEnd)?;
    let step = if next == 0 {
        None
    } else {
        if next % ALIGN != 0 {
            return Err(ErrorKind::MisalignedNext);
        }
        if next < name_end {
            return Err(ErrorKind::NextTooSmall);
        }
        if next >= record.len() {
            return Err(ErrorKind::NextPastEnd);
        }
        Some(next)
    };
    Ok((
        Entry {
            name: Utf16Name::from_le_bytes(name),
            creation_time,
            write_time,
            change_time,
            size,
            attributes,
            reparse_tag,
            file_id: u128::from_le_bytes(id),
        },
        step,
    ))
}

#[cfg(test)]
mod tests;
