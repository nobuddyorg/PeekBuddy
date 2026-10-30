//! Decoding a buffer of `FSCTL_READ_USN_JOURNAL` output (#47).
//!
//! The buffer starts with the next USN (8 bytes) and then holds records, each
//! starting with `RecordLength`, `MajorVersion` and `MinorVersion`. Version 2
//! records have 64-bit file reference numbers, version 3 records 128-bit ones;
//! version 4 records describe data ranges and are skipped (each follows a
//! version 3 record). The decoder reads from a byte slice, never indexes, and
//! stops at the first record that does not fit, so a damaged buffer yields
//! the records before the damage and an error, not a panic.
//!
//! Names are UTF-16LE bytes in the buffer. The unprivileged read returns
//! them empty (spike 0001), which the caller sees as an empty name.

use std::fmt;

/// The bytes before the first record: the next USN to read from.
pub const HEADER_LEN: usize = 8;

/// Size of the common header (`RecordLength`, `MajorVersion`, `MinorVersion`).
const COMMON_LEN: usize = 8;
/// Fixed part of a version 2 record, before the name.
const V2_FIXED: usize = 60;
/// Fixed part of a version 3 record, before the name.
const V3_FIXED: usize = 76;

/// `FILE_ATTRIBUTE_DIRECTORY`.
pub const ATTRIBUTE_DIRECTORY: u32 = 0x10;

/// One journal record, borrowed from the buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Record<'a> {
    /// Record format: 2 or 3.
    pub major: u16,
    /// File reference number; version 2 numbers are widened (the sequence
    /// number stays in the top 16 bits of the low half).
    pub frn: u128,
    /// The parent directory's reference number, widened the same way.
    pub parent: u128,
    /// The record's USN.
    pub usn: i64,
    /// FILETIME of the change.
    pub timestamp: i64,
    /// `USN_REASON_*` bits.
    pub reason: u32,
    /// `USN_SOURCE_*` bits.
    pub source_info: u32,
    /// `FILE_ATTRIBUTE_*` bits.
    pub attributes: u32,
    name: &'a [u8],
}

impl Record<'_> {
    /// The name as UTF-16 units; empty when the read withheld it.
    pub fn name(&self) -> impl Iterator<Item = u16> + '_ {
        self.name
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
    }

    /// The name as an owned vector of UTF-16 units.
    #[must_use]
    pub fn name_units(&self) -> Vec<u16> {
        self.name().collect()
    }

    /// Whether the name is empty (withheld, or a record without one).
    #[must_use]
    pub const fn name_withheld(&self) -> bool {
        self.name.is_empty()
    }

    /// Whether the subject is a directory.
    #[must_use]
    pub const fn is_dir(&self) -> bool {
        self.attributes & ATTRIBUTE_DIRECTORY != 0
    }
}

/// Why a buffer could not be decoded further.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// Fewer than 8 bytes: no next USN.
    NoHeader,
    /// A record is shorter than its header or runs past the buffer.
    BadLength {
        /// Offset of the record in the buffer.
        at: usize,
    },
    /// A major version other than 2, 3 or 4.
    UnknownVersion {
        /// Offset of the record in the buffer.
        at: usize,
        /// The major version found.
        major: u16,
    },
    /// The name does not lie inside the record.
    BadName {
        /// Offset of the record in the buffer.
        at: usize,
    },
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoHeader => write!(f, "journal buffer shorter than its header"),
            Self::BadLength { at } => write!(f, "bad record length at offset {at}"),
            Self::UnknownVersion { at, major } => {
                write!(f, "unknown record version {major} at offset {at}")
            }
            Self::BadName { at } => write!(f, "name outside the record at offset {at}"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// The records of one buffer.
#[derive(Debug, Clone)]
pub struct Records<'a> {
    buf: &'a [u8],
    at: usize,
    next_usn: i64,
    failed: bool,
    skipped_v4: u64,
}

/// Starts decoding `buf`.
///
/// # Errors
///
/// [`DecodeError::NoHeader`] when `buf` is shorter than 8 bytes.
pub fn records(buf: &[u8]) -> Result<Records<'_>, DecodeError> {
    let head = buf.get(..HEADER_LEN).ok_or(DecodeError::NoHeader)?;
    let next_usn = i64::from_le_bytes(array(head).ok_or(DecodeError::NoHeader)?);
    Ok(Records {
        buf,
        at: HEADER_LEN,
        next_usn,
        failed: false,
        skipped_v4: 0,
    })
}

impl Records<'_> {
    /// The USN to continue reading from (the buffer's first 8 bytes).
    #[must_use]
    pub const fn next_usn(&self) -> i64 {
        self.next_usn
    }

    /// Version 4 (range) records skipped so far.
    #[must_use]
    pub const fn skipped_v4(&self) -> u64 {
        self.skipped_v4
    }
}

impl<'a> Iterator for Records<'a> {
    type Item = Result<Record<'a>, DecodeError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.failed || self.at >= self.buf.len() {
                return None;
            }
            let at = self.at;
            let rest = self.buf.get(at..)?;
            let Some(length) = read_u32(rest, 0).and_then(|len| usize::try_from(len).ok()) else {
                self.failed = true;
                return Some(Err(DecodeError::BadLength { at }));
            };
            let Some(record) = (length >= COMMON_LEN).then(|| rest.get(..length)).flatten() else {
                self.failed = true;
                return Some(Err(DecodeError::BadLength { at }));
            };
            // Records start on 8-byte boundaries; the length is rounded up.
            self.at = at.saturating_add(length.next_multiple_of(8));
            let major = read_u16(record, 4).unwrap_or(0);
            match major {
                2 | 3 => {
                    let parsed = if major == 2 {
                        parse_v2(record, at)
                    } else {
                        parse_v3(record, at)
                    };
                    if parsed.is_err() {
                        self.failed = true;
                    }
                    return Some(parsed);
                }
                4 => self.skipped_v4 += 1,
                _ => {
                    self.failed = true;
                    return Some(Err(DecodeError::UnknownVersion { at, major }));
                }
            }
        }
    }
}

fn parse_v2(record: &[u8], at: usize) -> Result<Record<'_>, DecodeError> {
    let bad = DecodeError::BadLength { at };
    if record.len() < V2_FIXED {
        return Err(bad);
    }
    let field = |value: Option<u64>| value.ok_or(bad);
    Ok(Record {
        major: 2,
        frn: u128::from(field(read_u64(record, 8))?),
        parent: u128::from(field(read_u64(record, 16))?),
        usn: field(read_u64(record, 24))?.cast_signed(),
        timestamp: field(read_u64(record, 32))?.cast_signed(),
        reason: read_u32(record, 40).ok_or(bad)?,
        source_info: read_u32(record, 44).ok_or(bad)?,
        attributes: read_u32(record, 52).ok_or(bad)?,
        name: name_of(record, 56, 58, at)?,
    })
}

fn parse_v3(record: &[u8], at: usize) -> Result<Record<'_>, DecodeError> {
    let bad = DecodeError::BadLength { at };
    if record.len() < V3_FIXED {
        return Err(bad);
    }
    Ok(Record {
        major: 3,
        frn: read_u128(record, 8).ok_or(bad)?,
        parent: read_u128(record, 24).ok_or(bad)?,
        usn: read_u64(record, 40).ok_or(bad)?.cast_signed(),
        timestamp: read_u64(record, 48).ok_or(bad)?.cast_signed(),
        reason: read_u32(record, 56).ok_or(bad)?,
        source_info: read_u32(record, 60).ok_or(bad)?,
        attributes: read_u32(record, 68).ok_or(bad)?,
        name: name_of(record, 72, 74, at)?,
    })
}

/// The name bytes: length at `len_at`, offset at `off_at`, both `u16`.
fn name_of(record: &[u8], len_at: usize, off_at: usize, at: usize) -> Result<&[u8], DecodeError> {
    let bad = DecodeError::BadName { at };
    let length = usize::from(read_u16(record, len_at).ok_or(bad)?);
    let offset = usize::from(read_u16(record, off_at).ok_or(bad)?);
    if length == 0 {
        return Ok(&[]);
    }
    if length % 2 != 0 {
        return Err(bad);
    }
    let end = offset.checked_add(length).ok_or(bad)?;
    record.get(offset..end).ok_or(bad)
}

fn array<const N: usize>(bytes: &[u8]) -> Option<[u8; N]> {
    bytes.get(..N)?.try_into().ok()
}

fn read_u16(buf: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(array(buf.get(at..)?)?))
}

fn read_u32(buf: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(array(buf.get(at..)?)?))
}

fn read_u64(buf: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(array(buf.get(at..)?)?))
}

fn read_u128(buf: &[u8], at: usize) -> Option<u128> {
    Some(u128::from_le_bytes(array(buf.get(at..)?)?))
}

/// Encoders for tests and for the recorded streams of the scoping tests:
/// the inverse of the decoder, so round trips can be checked.
#[cfg(test)]
pub(crate) mod encode {
    use super::{HEADER_LEN, V2_FIXED, V3_FIXED};

    /// What a record is made of.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Spec {
        pub major: u16,
        pub frn: u128,
        pub parent: u128,
        pub usn: i64,
        pub timestamp: i64,
        pub reason: u32,
        pub source_info: u32,
        pub attributes: u32,
        pub name: Vec<u16>,
    }

    fn pad(out: &mut Vec<u8>) {
        while !out.len().is_multiple_of(8) {
            out.push(0);
        }
    }

    /// One record.
    pub fn record(spec: &Spec) -> Vec<u8> {
        let name: Vec<u8> = spec
            .name
            .iter()
            .flat_map(|unit| unit.to_le_bytes())
            .collect();
        let fixed = if spec.major == 2 { V2_FIXED } else { V3_FIXED };
        let length = (fixed + name.len()).next_multiple_of(8);
        let mut out = Vec::with_capacity(length);
        out.extend_from_slice(&u32::try_from(length).unwrap_or(0).to_le_bytes());
        out.extend_from_slice(&spec.major.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        if spec.major == 2 {
            out.extend_from_slice(&u64::try_from(spec.frn).unwrap_or(0).to_le_bytes());
            out.extend_from_slice(&u64::try_from(spec.parent).unwrap_or(0).to_le_bytes());
            out.extend_from_slice(&spec.usn.to_le_bytes());
            out.extend_from_slice(&spec.timestamp.to_le_bytes());
            out.extend_from_slice(&spec.reason.to_le_bytes());
            out.extend_from_slice(&spec.source_info.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&spec.attributes.to_le_bytes());
            out.extend_from_slice(&u16::try_from(name.len()).unwrap_or(0).to_le_bytes());
            out.extend_from_slice(&u16::try_from(V2_FIXED).unwrap_or(0).to_le_bytes());
        } else {
            out.extend_from_slice(&spec.frn.to_le_bytes());
            out.extend_from_slice(&spec.parent.to_le_bytes());
            out.extend_from_slice(&spec.usn.to_le_bytes());
            out.extend_from_slice(&spec.timestamp.to_le_bytes());
            out.extend_from_slice(&spec.reason.to_le_bytes());
            out.extend_from_slice(&spec.source_info.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&spec.attributes.to_le_bytes());
            out.extend_from_slice(&u16::try_from(name.len()).unwrap_or(0).to_le_bytes());
            out.extend_from_slice(&u16::try_from(V3_FIXED).unwrap_or(0).to_le_bytes());
        }
        out.extend_from_slice(&name);
        pad(&mut out);
        out
    }

    /// A version 4 record (60 bytes: the header, ids, USN, reason and no
    /// extents), for the "skipped" cases.
    pub fn record_v4() -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&64u32.to_le_bytes());
        out.extend_from_slice(&4u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.resize(64, 0);
        out
    }

    /// A buffer: the next USN, then the records.
    pub fn buffer(next_usn: i64, parts: &[Vec<u8>]) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_LEN);
        out.extend_from_slice(&next_usn.to_le_bytes());
        for part in parts {
            out.extend_from_slice(part);
        }
        out
    }
}

#[cfg(test)]
mod tests;
