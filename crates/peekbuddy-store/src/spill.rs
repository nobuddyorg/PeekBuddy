//! The spill file: where batches go when the writer cannot keep up (#52).
//!
//! When the ring passes its high-water mark all new batches are appended here
//! instead, and the writer replays them in order once the ring is empty, so id
//! order is preserved. Each record is `u32 LE length | u32 LE CRC-32 | payload`;
//! the payload is the batch in the event wire format. A torn tail (a crash in
//! the middle of an append) ends the replay at the last whole record: the
//! batch being appended is lost, which the reconcile pass after a restart
//! reports as a gap, never silently.
//!
//! The file starts with a header, `PBSP` and a `u32` generation. The writer
//! commits the generation and the offset it has replayed up to in the same
//! transaction as the replayed rows (`meta.spill_gen`, `meta.spill_offset`),
//! so after any restart the replay resumes exactly after the last record that
//! reached the database: no record is committed twice and none is skipped.

use std::fs::{File, OpenOptions};
use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::path::{Path, PathBuf};

use peekbuddy_core::event::wire::{self, MAX_ENCODED_LEN};
use peekbuddy_core::event::{Event, MAX_NAME_UNITS, NameArena};

use crate::batch::Batch;

/// File name inside the spill directory.
pub const FILE_NAME: &str = "spill.bin";

/// Magic bytes at the start of the file.
const MAGIC: &[u8; 4] = b"PBSP";
/// Magic and generation.
const HEADER_LEN: u64 = 8;

/// Largest payload of one record; a bigger length is corruption.
const MAX_PAYLOAD: usize = 64 * 1024 * 1024;

const fn crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut index = 0;
    while index < 256 {
        let mut crc = index as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
            bit += 1;
        }
        table[index] = crc;
        index += 1;
    }
    table
}

static CRC_TABLE: [u32; 256] = crc_table();

/// CRC-32 (IEEE 802.3) of `bytes`.
#[must_use]
pub fn crc32(bytes: &[u8]) -> u32 {
    !bytes.iter().fold(!0u32, |crc, byte| {
        let index = usize::from((crc ^ u32::from(*byte)).to_le_bytes()[0]);
        CRC_TABLE.get(index).copied().unwrap_or(0) ^ (crc >> 8)
    })
}

/// Encodes `batch` as a payload appended to `out`.
///
/// # Errors
///
/// A message when an event or name cannot be encoded.
pub fn encode_batch(batch: &Batch, scratch: &mut Vec<u8>, out: &mut Vec<u8>) -> Result<(), String> {
    out.extend_from_slice(&u32::try_from(batch.len()).unwrap_or(u32::MAX).to_le_bytes());
    scratch.resize(MAX_ENCODED_LEN, 0);
    for (event, name) in batch.iter() {
        let used = wire::encode(event, name, scratch).map_err(|error| format!("{error:?}"))?;
        out.extend_from_slice(scratch.get(..used).unwrap_or_default());
    }
    Ok(())
}

/// Decodes a payload into `into` (cleared first).
///
/// # Errors
///
/// A message for a truncated or invalid payload.
pub fn decode_batch(payload: &[u8], into: &mut Batch) -> Result<(), String> {
    into.clear();
    let count = payload
        .first_chunk::<4>()
        .map(|bytes| u32::from_le_bytes(*bytes))
        .ok_or("payload too short")? as usize;
    let mut at = 4;
    let mut arena = NameArena::with_capacity(MAX_NAME_UNITS);
    for _ in 0..count {
        arena.reset();
        let rest = payload.get(at..).ok_or("truncated record")?;
        let (event, used): (Event, usize) =
            wire::decode(rest, &mut arena).map_err(|error| format!("{error:?}"))?;
        let name = arena.get(event.name).unwrap_or_default().to_vec();
        into.push(event, &name)
            .map_err(|full| format!("batch full: {full:?}"))?;
        at += used;
    }
    Ok(())
}

/// The spill file of one data directory.
#[derive(Debug)]
pub struct Spill {
    path: PathBuf,
    generation: u32,
    append: Option<File>,
    read: Option<File>,
    read_pos: u64,
    end: u64,
    scratch: Vec<u8>,
    payload: Vec<u8>,
}

impl Spill {
    /// Opens the spill directory `dir` (created if needed).
    ///
    /// `last_gen` and `consumed` are the position the database says it has
    /// replayed up to (generation and byte offset). A file of that generation
    /// resumes after `consumed`; a newer one is replayed from its start.
    ///
    /// # Errors
    ///
    /// The I/O error from creating the directory or reading the file.
    pub fn open(dir: &Path, last_gen: u32, consumed: u64) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(FILE_NAME);
        let mut spill = Self {
            path,
            generation: last_gen.saturating_add(1),
            append: None,
            read: None,
            read_pos: 0,
            end: 0,
            scratch: Vec::new(),
            payload: Vec::new(),
        };
        let Ok(mut file) = File::open(&spill.path) else {
            return Ok(spill);
        };
        let length = file.metadata()?.len();
        let mut header = [0u8; 8];
        if length < HEADER_LEN
            || file.read_exact(&mut header).is_err()
            || !header.starts_with(MAGIC)
        {
            // Not a spill file we wrote (a crash before the header was
            // complete): nothing in it can be trusted or needed.
            drop(file);
            let _ = std::fs::remove_file(&spill.path);
            return Ok(spill);
        }
        let [_, _, _, _, g0, g1, g2, g3] = header;
        let found = u32::from_le_bytes([g0, g1, g2, g3]);
        spill.generation = found;
        spill.end = length;
        spill.read_pos = if found == last_gen {
            consumed.clamp(HEADER_LEN, length)
        } else {
            HEADER_LEN
        };
        Ok(spill)
    }

    /// The generation of the file being written or replayed.
    #[must_use]
    pub const fn generation(&self) -> u32 {
        self.generation
    }

    /// Size of the file in bytes (0 when there is none).
    #[must_use]
    pub const fn end(&self) -> u64 {
        self.end
    }

    /// Whether records wait to be replayed.
    #[must_use]
    pub const fn has_pending(&self) -> bool {
        self.end > self.read_pos
    }

    /// Where the next record is read from (bytes from the start of the file).
    #[must_use]
    pub const fn position(&self) -> u64 {
        self.read_pos
    }

    /// Bytes written and not yet replayed.
    #[must_use]
    pub const fn pending_bytes(&self) -> u64 {
        self.end.saturating_sub(self.read_pos)
    }

    /// Appends `batch` as one record.
    ///
    /// # Errors
    ///
    /// The I/O error (a full disk): the caller then reports a gap.
    pub fn append(&mut self, batch: &Batch) -> std::io::Result<()> {
        if self.append.is_none() {
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)?;
            if self.end == 0 {
                let mut header = Vec::with_capacity(8);
                header.extend_from_slice(MAGIC);
                header.extend_from_slice(&self.generation.to_le_bytes());
                file.write_all(&header)?;
                self.end = HEADER_LEN;
                self.read_pos = HEADER_LEN;
            }
            self.append = Some(file);
        }
        self.payload.clear();
        encode_batch(batch, &mut self.scratch, &mut self.payload).map_err(std::io::Error::other)?;
        let mut record = Vec::with_capacity(8 + self.payload.len());
        record.extend_from_slice(
            &u32::try_from(self.payload.len())
                .unwrap_or(u32::MAX)
                .to_le_bytes(),
        );
        record.extend_from_slice(&crc32(&self.payload).to_le_bytes());
        record.extend_from_slice(&self.payload);
        if let Some(file) = &mut self.append {
            file.write_all(&record)?;
            file.sync_data()?;
        }
        self.end += record.len() as u64;
        Ok(())
    }

    /// Reads the next record into `into` and returns the offset just after it.
    /// `Ok(None)` when everything written has been replayed, or when the rest
    /// is a torn or corrupt tail (which is then discarded).
    ///
    /// # Errors
    ///
    /// The I/O error from reading.
    pub fn next(&mut self, into: &mut Batch) -> std::io::Result<Option<u64>> {
        if !self.has_pending() {
            return Ok(None);
        }
        if self.read.is_none() {
            self.read = Some(File::open(&self.path)?);
        }
        let Some(file) = &mut self.read else {
            return Ok(None);
        };
        file.seek(SeekFrom::Start(self.read_pos))?;
        let mut header = [0u8; 8];
        if file.read_exact(&mut header).is_err() {
            return Ok(self.discard_tail());
        }
        let [l0, l1, l2, l3, c0, c1, c2, c3] = header;
        let length = u32::from_le_bytes([l0, l1, l2, l3]) as usize;
        let crc = u32::from_le_bytes([c0, c1, c2, c3]);
        if length > MAX_PAYLOAD || (self.read_pos + 8 + length as u64) > self.end {
            return Ok(self.discard_tail());
        }
        self.payload.resize(length, 0);
        if file.read_exact(&mut self.payload).is_err() || crc32(&self.payload) != crc {
            return Ok(self.discard_tail());
        }
        if decode_batch(&self.payload, into).is_err() {
            return Ok(self.discard_tail());
        }
        self.read_pos += 8 + length as u64;
        Ok(Some(self.read_pos))
    }

    fn discard_tail(&mut self) -> Option<u64> {
        self.read_pos = self.end;
        None
    }

    /// After a full replay: removes the file so the next spill starts empty.
    ///
    /// # Errors
    ///
    /// The I/O error from deleting the file.
    pub fn finish(&mut self) -> std::io::Result<()> {
        self.append = None;
        self.read = None;
        self.read_pos = 0;
        self.end = 0;
        self.generation = self.generation.saturating_add(1);
        match std::fs::remove_file(&self.path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
#[expect(clippy::indexing_slicing, reason = "tests index fixed buffers")]
mod tests {
    use std::io::Write as _;

    use super::{Spill, crc32, decode_batch, encode_batch};
    use crate::batch::Batch;
    use crate::batch::tests::{event, units};

    fn dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("pb-spill-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn batch(first: u64, count: u64) -> Batch {
        let mut batch = Batch::with_capacity(64, 4096);
        for ts in first..first + count {
            batch
                .push(event(ts, 0), &units(&format!("dir\\file{ts}.txt")))
                .unwrap();
        }
        batch
    }

    fn collect(spill: &mut Spill) -> Vec<(u64, Vec<u16>)> {
        let mut out = Vec::new();
        let mut into = Batch::with_capacity(64, 4096);
        while spill.next(&mut into).unwrap().is_some() {
            out.extend(into.iter().map(|(e, n)| (e.ts, n.to_vec())));
        }
        out
    }

    #[test]
    fn the_crc_matches_the_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn a_batch_survives_encoding() {
        let original = batch(1, 5);
        let (mut scratch, mut payload) = (Vec::new(), Vec::new());
        encode_batch(&original, &mut scratch, &mut payload).unwrap();
        let mut copy = Batch::with_capacity(64, 4096);
        decode_batch(&payload, &mut copy).unwrap();
        let a: Vec<_> = original
            .iter()
            .map(|(e, n)| (e.ts, e.file_id, n.to_vec()))
            .collect();
        let b: Vec<_> = copy
            .iter()
            .map(|(e, n)| (e.ts, e.file_id, n.to_vec()))
            .collect();
        assert_eq!(a, b);
        assert!(decode_batch(&payload[..payload.len() - 3], &mut copy).is_err());
        assert!(decode_batch(&[1], &mut copy).is_err());
    }

    #[test]
    fn records_replay_in_order_and_the_file_is_removed_when_finished() {
        let dir = dir("order");
        let mut spill = Spill::open(&dir, 0, 0).unwrap();
        assert!(!spill.has_pending());
        for first in [1, 11, 21] {
            spill.append(&batch(first, 10)).unwrap();
        }
        assert!(spill.has_pending());
        let ts: Vec<u64> = collect(&mut spill).into_iter().map(|(ts, _)| ts).collect();
        assert_eq!(ts, (1..=30).collect::<Vec<_>>());
        assert!(!spill.has_pending());
        spill.finish().unwrap();
        assert!(!dir.join(super::FILE_NAME).exists());
        spill.append(&batch(100, 1)).unwrap();
        assert_eq!(collect(&mut spill).len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_file_from_a_previous_run_is_replayed_after_a_restart() {
        let dir = dir("restart");
        let mut spill = Spill::open(&dir, 0, 0).unwrap();
        spill.append(&batch(1, 3)).unwrap();
        drop(spill);
        let mut again = Spill::open(&dir, 0, 0).unwrap();
        assert!(again.has_pending());
        assert_eq!(collect(&mut again).len(), 3);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_torn_tail_ends_the_replay_at_the_last_whole_record() {
        let dir = dir("torn");
        let mut spill = Spill::open(&dir, 0, 0).unwrap();
        spill.append(&batch(1, 4)).unwrap();
        spill.append(&batch(5, 4)).unwrap();
        drop(spill);
        let path = dir.join(super::FILE_NAME);
        let whole = std::fs::read(&path).unwrap();
        // A crash in the middle of the second record.
        std::fs::write(&path, &whole[..whole.len() - 10]).unwrap();
        let mut again = Spill::open(&dir, 0, 0).unwrap();
        let ts: Vec<u64> = collect(&mut again).into_iter().map(|(ts, _)| ts).collect();
        assert_eq!(ts, [1, 2, 3, 4]);
        assert!(!again.has_pending());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn replay_resumes_after_the_position_the_database_recorded() {
        let dir = dir("resume");
        let mut spill = Spill::open(&dir, 0, 0).unwrap();
        for first in [1, 11, 21] {
            spill.append(&batch(first, 10)).unwrap();
        }
        let generation = spill.generation();
        let mut into = Batch::with_capacity(64, 4096);
        let after_first = spill.next(&mut into).unwrap().unwrap();
        drop(spill);
        // Same generation: skip what the database already has.
        let mut again = Spill::open(&dir, generation, after_first).unwrap();
        let ts: Vec<u64> = collect(&mut again).into_iter().map(|(ts, _)| ts).collect();
        assert_eq!(ts, (11..=30).collect::<Vec<_>>());
        // A database that knows an older generation replays the whole file.
        let mut fresh = Spill::open(&dir, generation - 1, after_first).unwrap();
        assert_eq!(collect(&mut fresh).len(), 30);
        // A database that already consumed the whole file replays nothing.
        let len = std::fs::metadata(dir.join(super::FILE_NAME)).unwrap().len();
        let mut done = Spill::open(&dir, generation, len).unwrap();
        assert!(!done.has_pending());
        assert!(collect(&mut done).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_finished_file_makes_the_next_one_a_new_generation() {
        let dir = dir("generation");
        let mut spill = Spill::open(&dir, 4, 0).unwrap();
        assert_eq!(spill.generation(), 5);
        spill.append(&batch(1, 1)).unwrap();
        collect(&mut spill);
        spill.finish().unwrap();
        assert_eq!(spill.generation(), 6);
        spill.append(&batch(2, 1)).unwrap();
        drop(spill);
        let again = Spill::open(&dir, 5, 99_999).unwrap();
        assert_eq!(again.generation(), 6);
        // The new generation is newer than the database knows: from the start.
        assert!(again.has_pending());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_header_that_is_not_ours_is_discarded() {
        let dir = dir("header");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(super::FILE_NAME), b"garbage-not-a-spill").unwrap();
        let spill = Spill::open(&dir, 0, 0).unwrap();
        assert!(!spill.has_pending());
        assert!(!dir.join(super::FILE_NAME).exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_flipped_byte_is_caught_by_the_checksum() {
        let dir = dir("crc");
        let mut spill = Spill::open(&dir, 0, 0).unwrap();
        spill.append(&batch(1, 4)).unwrap();
        drop(spill);
        let path = dir.join(super::FILE_NAME);
        let mut bytes = std::fs::read(&path).unwrap();
        let middle = bytes.len() / 2;
        bytes[middle] ^= 0xFF;
        std::fs::File::create(&path)
            .unwrap()
            .write_all(&bytes)
            .unwrap();
        let mut again = Spill::open(&dir, 0, 0).unwrap();
        assert!(collect(&mut again).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
