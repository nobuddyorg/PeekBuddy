//! Zero-copy parser for `ReadDirectoryChanges*` result buffers (#44).
//!
//! The buffer holds a chain of variable-length records. Three layouts exist
//! ([`Class`]): the plain `FILE_NOTIFY_INFORMATION`, the extended
//! `FILE_NOTIFY_EXTENDED_INFORMATION` and the full
//! `FILE_NOTIFY_FULL_INFORMATION`. Every field is decoded by byte offset with
//! `from_le_bytes`, so there are no pointer casts, no alignment requirements
//! and no `unsafe`.
//!
//! # Validation
//!
//! [`Records`] parses only the `bytes_returned` prefix it is given and checks
//! every record before it is yielded:
//!
//! 1. the fixed header fits in the rest of the buffer
//!    ([`ErrorKind::Truncated`]);
//! 2. `FileNameLength` is even ([`ErrorKind::OddNameLength`]);
//! 3. the name fits in the rest of the buffer ([`ErrorKind::NameOutOfBounds`]);
//! 4. `NextEntryOffset` is 0 (the last record) or
//!    - a multiple of 4 ([`ErrorKind::MisalignedNext`]),
//!    - at least the size of the current record, header and name
//!      ([`ErrorKind::NextTooSmall`]),
//!    - below the end of the buffer ([`ErrorKind::NextPastEnd`]);
//! 5. at most `max(1, len / header size)` records are read
//!    ([`ErrorKind::TooManyRecords`]).
//!
//! A malformed record ends the iteration with a [`ParseError`] carrying its
//! byte offset; records yielded before it stay valid. The caller emits an
//! overflow gap and schedules a reconcile. `bytes_returned == 0` is an
//! overflow and is the caller's to handle: it yields no record and no error.
//!
//! Unknown actions are kept as [`Action::Unknown`]: raw is truth. Record order
//! is preserved, so a rename pair (old, new) stays adjacent.

use crate::event::Kind;

/// `FILE_ACTION_ADDED`.
pub const ACTION_ADDED: u32 = 1;
/// `FILE_ACTION_REMOVED`.
pub const ACTION_REMOVED: u32 = 2;
/// `FILE_ACTION_MODIFIED`.
pub const ACTION_MODIFIED: u32 = 3;
/// `FILE_ACTION_RENAMED_OLD_NAME`.
pub const ACTION_RENAMED_OLD_NAME: u32 = 4;
/// `FILE_ACTION_RENAMED_NEW_NAME`.
pub const ACTION_RENAMED_NEW_NAME: u32 = 5;

/// `FILE_NAME_FLAG_HARDLINK`: the name came from a hard link.
pub const NAME_FLAG_HARDLINK: u8 = 0;
/// `FILE_NAME_FLAG_NTFS`: the long (NTFS) name.
pub const NAME_FLAG_NTFS: u8 = 1;
/// `FILE_NAME_FLAG_DOS`: the 8.3 alias.
pub const NAME_FLAG_DOS: u8 = 2;
/// `FILE_NAME_FLAG_BOTH`: long name and alias are the same.
pub const NAME_FLAG_BOTH: u8 = 3;
/// `FILE_NAME_FLAGS_UNSPECIFIED`.
pub const NAME_FLAGS_UNSPECIFIED: u8 = 128;

/// `FILE_ATTRIBUTE_DIRECTORY`.
pub const ATTRIBUTE_DIRECTORY: u32 = 0x10;

/// Byte offsets of the fixed layouts, checked against `windows-sys` by a
/// Windows-only test.
pub mod layout {
    /// Plain: `NextEntryOffset`.
    pub const NEXT_ENTRY_OFFSET: usize = 0;
    /// All classes: `Action`.
    pub const ACTION: usize = 4;
    /// Plain: `FileNameLength` (u32).
    pub const PLAIN_NAME_LENGTH: usize = 8;
    /// Plain: `FileName`.
    pub const PLAIN_NAME: usize = 12;
    /// Extended and full: `CreationTime`.
    pub const CREATION_TIME: usize = 8;
    /// Extended and full: `LastModificationTime`.
    pub const LAST_WRITE_TIME: usize = 16;
    /// Extended and full: `LastChangeTime`.
    pub const CHANGE_TIME: usize = 24;
    /// Extended and full: `LastAccessTime`.
    pub const ACCESS_TIME: usize = 32;
    /// Extended and full: `AllocatedLength`.
    pub const ALLOCATED_LENGTH: usize = 40;
    /// Extended and full: `FileSize`.
    pub const FILE_SIZE: usize = 48;
    /// Extended and full: `FileAttributes`.
    pub const FILE_ATTRIBUTES: usize = 56;
    /// Extended and full: `ReparsePointTag`.
    pub const REPARSE_TAG: usize = 60;
    /// Extended and full: `FileId`.
    pub const FILE_ID: usize = 64;
    /// Extended and full: `ParentFileId`.
    pub const PARENT_FILE_ID: usize = 72;
    /// Extended and full: `FileNameLength` (u32 in extended, u16 in full).
    pub const EXTENDED_NAME_LENGTH: usize = 80;
    /// Full: `FileNameFlags`.
    pub const FULL_NAME_FLAGS: usize = 82;
    /// Extended and full: `FileName`.
    pub const EXTENDED_NAME: usize = 84;
}

/// Which information class the buffer holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// `FILE_NOTIFY_INFORMATION`.
    Plain,
    /// `FILE_NOTIFY_EXTENDED_INFORMATION`.
    Extended,
    /// `FILE_NOTIFY_FULL_INFORMATION`.
    Full,
}

impl Class {
    /// The size of the fixed header, which is where the name starts.
    #[must_use]
    pub const fn header_len(self) -> usize {
        match self {
            Self::Plain => layout::PLAIN_NAME,
            Self::Extended | Self::Full => layout::EXTENDED_NAME,
        }
    }
}

/// The raw action of a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// `FILE_ACTION_ADDED`.
    Added,
    /// `FILE_ACTION_REMOVED`.
    Removed,
    /// `FILE_ACTION_MODIFIED`.
    Modified,
    /// `FILE_ACTION_RENAMED_OLD_NAME`.
    RenamedOldName,
    /// `FILE_ACTION_RENAMED_NEW_NAME`.
    RenamedNewName,
    /// An action this parser does not know; never dropped.
    Unknown(u32),
}

impl Action {
    /// Decodes a raw action.
    #[must_use]
    pub const fn from_raw(raw: u32) -> Self {
        match raw {
            ACTION_ADDED => Self::Added,
            ACTION_REMOVED => Self::Removed,
            ACTION_MODIFIED => Self::Modified,
            ACTION_RENAMED_OLD_NAME => Self::RenamedOldName,
            ACTION_RENAMED_NEW_NAME => Self::RenamedNewName,
            other => Self::Unknown(other),
        }
    }

    /// The raw value.
    #[must_use]
    pub const fn raw(self) -> u32 {
        match self {
            Self::Added => ACTION_ADDED,
            Self::Removed => ACTION_REMOVED,
            Self::Modified => ACTION_MODIFIED,
            Self::RenamedOldName => ACTION_RENAMED_OLD_NAME,
            Self::RenamedNewName => ACTION_RENAMED_NEW_NAME,
            Self::Unknown(raw) => raw,
        }
    }

    /// The event kind this action maps to; unknown actions are [`Kind::Other`].
    #[must_use]
    pub const fn kind(self) -> Kind {
        match self {
            Self::Added => Kind::Created,
            Self::Removed => Kind::Deleted,
            Self::Modified => Kind::Modified,
            Self::RenamedOldName => Kind::RenamedFrom,
            Self::RenamedNewName => Kind::RenamedTo,
            Self::Unknown(_) => Kind::Other,
        }
    }
}

/// A borrowed UTF-16LE name inside the buffer. It is not NUL-terminated and
/// may hold unpaired surrogates.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Utf16Name<'a> {
    bytes: &'a [u8],
}

impl<'a> Utf16Name<'a> {
    /// A view over `bytes`; an odd trailing byte is ignored.
    #[must_use]
    pub const fn from_le_bytes(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    /// The length in UTF-16 units.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.bytes.len() / 2
    }

    /// Whether the name is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.bytes.len() < 2
    }

    /// The units, decoded on the fly.
    pub fn units(&self) -> impl ExactSizeIterator<Item = u16> + 'a {
        self.bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&pair| u16::from_le_bytes(pair))
    }

    /// Copies the units into `out`; `None` if `out` is too short.
    pub fn copy_into(&self, out: &mut [u16]) -> Option<usize> {
        let target = out.get_mut(..self.len())?;
        for (slot, unit) in target.iter_mut().zip(self.units()) {
            *slot = unit;
        }
        Some(target.len())
    }
}

impl std::fmt::Debug for Utf16Name<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text: String = char::decode_utf16(self.units())
            .map(|unit| unit.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect();
        write!(f, "{text:?}")
    }
}

/// The fields the extended and full classes add.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Extended {
    /// `CreationTime` (FILETIME ticks).
    pub creation_time: i64,
    /// `LastModificationTime`.
    pub last_write_time: i64,
    /// `LastChangeTime`.
    pub change_time: i64,
    /// `LastAccessTime`.
    pub last_access_time: i64,
    /// `AllocatedLength`.
    pub allocated_length: i64,
    /// `FileSize`.
    pub file_size: i64,
    /// `FileAttributes`.
    pub attributes: u32,
    /// `ReparsePointTag` (shares its slot with `EaSize`).
    pub reparse_tag: u32,
    /// `FileId`.
    pub file_id: i64,
    /// `ParentFileId`.
    pub parent_file_id: i64,
    /// `FileNameFlags`; only the full class has it.
    pub name_flags: Option<u8>,
}

impl Extended {
    /// Whether the attributes say directory.
    #[must_use]
    pub const fn is_dir(&self) -> bool {
        self.attributes & ATTRIBUTE_DIRECTORY != 0
    }
}

/// One parsed record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Record<'a> {
    /// The raw action.
    pub action: Action,
    /// The name, relative to the watched directory.
    pub name: Utf16Name<'a>,
    /// The extended fields, for the extended and full classes.
    pub extended: Option<Extended>,
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
    /// `NextEntryOffset` is not a multiple of 4.
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
            "malformed notify record at byte {}: {:?}",
            self.offset, self.kind
        )
    }
}

impl std::error::Error for ParseError {}

/// The iterator over the records of one buffer.
#[derive(Debug, Clone)]
pub struct Records<'a> {
    buf: &'a [u8],
    class: Class,
    /// The offset of the next record, `None` once the chain has ended.
    next: Option<usize>,
    /// How many more records are allowed.
    budget: usize,
    bad_length: bool,
}

/// Parses `buf[..bytes_returned]`.
///
/// `bytes_returned == 0` yields nothing: that is an overflow, which the
/// caller handles.
#[must_use]
pub fn parse(buf: &[u8], bytes_returned: usize, class: Class) -> Records<'_> {
    let (data, bad_length) = match buf.get(..bytes_returned) {
        Some(data) => (data, false),
        None => (buf, true),
    };
    Records {
        buf: data,
        class,
        next: (!data.is_empty() || bad_length).then_some(0),
        budget: (data.len() / class.header_len()).max(1),
        bad_length,
    }
}

impl<'a> Iterator for Records<'a> {
    type Item = Result<Record<'a>, ParseError>;

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
        let entry = self.buf.get(offset..).unwrap_or_default();
        match decode(entry, self.class) {
            Ok((record, next)) => {
                self.next = next.map(|step| offset.saturating_add(step));
                Some(Ok(record))
            }
            Err(kind) => Some(Err(ParseError { offset, kind })),
        }
    }
}

fn le_u32(bytes: &[u8], at: usize) -> Option<u32> {
    bytes
        .get(at..)?
        .first_chunk()
        .copied()
        .map(u32::from_le_bytes)
}

fn le_u16(bytes: &[u8], at: usize) -> Option<u16> {
    bytes
        .get(at..)?
        .first_chunk()
        .copied()
        .map(u16::from_le_bytes)
}

fn le_i64(bytes: &[u8], at: usize) -> Option<i64> {
    bytes
        .get(at..)?
        .first_chunk()
        .copied()
        .map(i64::from_le_bytes)
}

/// Decodes the record at the start of `entry`. On success also returns the
/// step to the next record (`None` for the last).
fn decode(entry: &[u8], class: Class) -> Result<(Record<'_>, Option<usize>), ErrorKind> {
    let header = class.header_len();
    if entry.len() < header {
        return Err(ErrorKind::Truncated);
    }
    let next_entry = le_u32(entry, layout::NEXT_ENTRY_OFFSET).ok_or(ErrorKind::Truncated)?;
    let action = le_u32(entry, layout::ACTION).ok_or(ErrorKind::Truncated)?;
    let name_len = match class {
        Class::Plain => le_u32(entry, layout::PLAIN_NAME_LENGTH),
        Class::Extended => le_u32(entry, layout::EXTENDED_NAME_LENGTH),
        Class::Full => le_u16(entry, layout::EXTENDED_NAME_LENGTH).map(u32::from),
    }
    .ok_or(ErrorKind::Truncated)?;
    if !name_len.is_multiple_of(2) {
        return Err(ErrorKind::OddNameLength);
    }
    let name_len = usize::try_from(name_len).map_err(|_| ErrorKind::NameOutOfBounds)?;
    let name = entry
        .get(header..)
        .and_then(|rest| rest.get(..name_len))
        .ok_or(ErrorKind::NameOutOfBounds)?;
    let extended = match class {
        Class::Plain => None,
        Class::Extended | Class::Full => Some(decode_extended(entry, class)?),
    };
    let record = Record {
        action: Action::from_raw(action),
        name: Utf16Name { bytes: name },
        extended,
    };
    if next_entry == 0 {
        return Ok((record, None));
    }
    let step = usize::try_from(next_entry).map_err(|_| ErrorKind::NextPastEnd)?;
    if !step.is_multiple_of(4) {
        return Err(ErrorKind::MisalignedNext);
    }
    if step < header.saturating_add(name_len) {
        return Err(ErrorKind::NextTooSmall);
    }
    if step >= entry.len() {
        return Err(ErrorKind::NextPastEnd);
    }
    Ok((record, Some(step)))
}

fn decode_extended(entry: &[u8], class: Class) -> Result<Extended, ErrorKind> {
    let field = |at| le_i64(entry, at).ok_or(ErrorKind::Truncated);
    let word = |at| le_u32(entry, at).ok_or(ErrorKind::Truncated);
    let name_flags = match class {
        Class::Full => Some(
            *entry
                .get(layout::FULL_NAME_FLAGS)
                .ok_or(ErrorKind::Truncated)?,
        ),
        Class::Plain | Class::Extended => None,
    };
    Ok(Extended {
        creation_time: field(layout::CREATION_TIME)?,
        last_write_time: field(layout::LAST_WRITE_TIME)?,
        change_time: field(layout::CHANGE_TIME)?,
        last_access_time: field(layout::ACCESS_TIME)?,
        allocated_length: field(layout::ALLOCATED_LENGTH)?,
        file_size: field(layout::FILE_SIZE)?,
        attributes: word(layout::FILE_ATTRIBUTES)?,
        reparse_tag: word(layout::REPARSE_TAG)?,
        file_id: field(layout::FILE_ID)?,
        parent_file_id: field(layout::PARENT_FILE_ID)?,
        name_flags,
    })
}

#[cfg(test)]
mod tests;
