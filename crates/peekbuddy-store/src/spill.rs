//! The spill file: where batches go when the writer cannot keep up (#52).
//!
//! When the ring passes its high-water mark all new batches are appended here
//! instead, and the writer replays them in order once the ring is empty, so id
//! order is preserved. Each record is `u32 LE length | u32 LE CRC-32 | payload`;
//! the payload is the batch in the event wire format. A torn tail (a crash in
//! the middle of an append) is found when the file is opened: every record
//! from the replay position on is checked (length, CRC, decode), the file is
//! cut after the last whole one and the cut is reported ([`Spill::torn_bytes`]).
//! The batch being appended is lost, which the reconcile pass after a restart
//! reports as a gap, never silently. Cutting at open matters: records that
//! are appended after a torn tail would otherwise sit behind garbage and be
//! discarded with it.
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

#[expect(clippy::cast_possible_truncation, reason = "the index is below 256")]
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

/// The offset just after the last whole record that starts at or after
/// `from`, in a file of `end` bytes: `end` when every record is whole.
fn whole_records(file: &mut File, from: u64, end: u64) -> std::io::Result<u64> {
    let mut at = from;
    let mut payload = Vec::new();
    let mut batch = Batch::standard();
    while at < end {
        file.seek(SeekFrom::Start(at))?;
        let mut header = [0u8; 8];
        if at + 8 > end || file.read_exact(&mut header).is_err() {
            break;
        }
        let [l0, l1, l2, l3, c0, c1, c2, c3] = header;
        let length = u32::from_le_bytes([l0, l1, l2, l3]) as usize;
        let crc = u32::from_le_bytes([c0, c1, c2, c3]);
        if length > MAX_PAYLOAD || at + 8 + length as u64 > end {
            break;
        }
        payload.resize(length, 0);
        if file.read_exact(&mut payload).is_err()
            || crc32(&payload) != crc
            || decode_batch(&payload, &mut batch).is_err()
        {
            break;
        }
        at += 8 + length as u64;
    }
    Ok(at)
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
    torn: u64,
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
            torn: 0,
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
            spill.torn = length;
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
        let whole = whole_records(&mut file, spill.read_pos, length)?;
        drop(file);
        if whole < length {
            OpenOptions::new()
                .write(true)
                .open(&spill.path)?
                .set_len(whole)?;
            spill.end = whole;
            spill.torn = length - whole;
        }
        Ok(spill)
    }

    /// Bytes cut from the end of the file because a record was not whole (a
    /// crash in the middle of an append), or the whole file when its header
    /// was not complete. 0 for a file that was whole.
    #[must_use]
    pub const fn torn_bytes(&self) -> u64 {
        self.torn
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
        self.append_group(std::slice::from_ref(batch))
    }

    /// Appends `batches` as one record each with a single sync: the sync is
    /// the cost of a spill, and the replay waits for the lock meanwhile.
    ///
    /// # Errors
    ///
    /// The I/O error (a full disk): none of the group is then in the file.
    pub fn append_group(&mut self, batches: &[Batch]) -> std::io::Result<()> {
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
        let mut record: Vec<u8> = Vec::new();
        for batch in batches {
            self.payload.clear();
            encode_batch(batch, &mut self.scratch, &mut self.payload)
                .map_err(std::io::Error::other)?;
            record.extend_from_slice(
                &u32::try_from(self.payload.len())
                    .unwrap_or(u32::MAX)
                    .to_le_bytes(),
            );
            record.extend_from_slice(&crc32(&self.payload).to_le_bytes());
            record.extend_from_slice(&self.payload);
        }
        if let Some(file) = &mut self.append {
            let written = file.write_all(&record).and_then(|()| file.sync_data());
            if let Err(error) = written {
                // A full disk can leave half a record: cut it, or the records
                // appended once there is room again would sit behind it and
                // be discarded with it at the replay.
                // (An append-only handle may not truncate: open another.)
                let end = self.end;
                self.append = None;
                let _ = OpenOptions::new()
                    .write(true)
                    .open(&self.path)
                    .and_then(|file| file.set_len(end));
                return Err(error);
            }
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

    use super::{Spill, crc32, decode_batch, encode_batch, whole_records};
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

    /// Bytes one record of `batch` takes in the file.
    fn record_len(batch: &Batch) -> usize {
        let mut payload = Vec::new();
        encode_batch(batch, &mut Vec::new(), &mut payload).unwrap();
        8 + payload.len()
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
    fn a_torn_tail_is_cut_at_open_and_reported() {
        let dir = dir("torn-cut");
        let mut spill = Spill::open(&dir, 0, 0).unwrap();
        spill.append(&batch(1, 4)).unwrap();
        spill.append(&batch(5, 4)).unwrap();
        drop(spill);
        let path = dir.join(super::FILE_NAME);
        let whole = std::fs::read(&path).unwrap();
        std::fs::write(&path, &whole[..whole.len() - 10]).unwrap();
        let again = Spill::open(&dir, 0, 0).unwrap();
        // The second record is gone from the file, not just skipped.
        let second = whole.len() - record_len(&batch(5, 4));
        assert_eq!(again.torn_bytes(), (whole.len() - 10 - second) as u64);
        assert_eq!(again.end(), second as u64);
        assert_eq!(std::fs::metadata(&path).unwrap().len(), second as u64);
        // A file that was whole has nothing torn.
        let clean = Spill::open(&dir, 0, 0).unwrap();
        assert_eq!(clean.torn_bytes(), 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn batches_appended_after_a_torn_tail_are_not_lost_with_it() {
        let dir = dir("torn-append");
        let mut spill = Spill::open(&dir, 0, 0).unwrap();
        spill.append(&batch(1, 3)).unwrap();
        spill.append(&batch(4, 3)).unwrap();
        drop(spill);
        let path = dir.join(super::FILE_NAME);
        let whole = std::fs::read(&path).unwrap();
        std::fs::write(&path, &whole[..whole.len() - 5]).unwrap();
        // The next run spills before it has replayed the old file.
        let mut again = Spill::open(&dir, 0, 0).unwrap();
        again.append(&batch(100, 2)).unwrap();
        let ts: Vec<u64> = collect(&mut again).into_iter().map(|(ts, _)| ts).collect();
        assert_eq!(ts, [1, 2, 3, 100, 101]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_record_with_a_wrong_checksum_ends_the_file_there() {
        let dir = dir("torn-crc");
        let mut spill = Spill::open(&dir, 0, 0).unwrap();
        spill.append(&batch(1, 2)).unwrap();
        spill.append(&batch(3, 2)).unwrap();
        spill.append(&batch(5, 2)).unwrap();
        drop(spill);
        let path = dir.join(super::FILE_NAME);
        let mut bytes = std::fs::read(&path).unwrap();
        // Flip a payload byte of the middle record.
        let first = record_len(&batch(1, 2));
        bytes[8 + first + 12] ^= 0xFF;
        std::fs::write(&path, &bytes).unwrap();
        let mut again = Spill::open(&dir, 0, 0).unwrap();
        assert!(again.torn_bytes() > 0);
        let ts: Vec<u64> = collect(&mut again).into_iter().map(|(ts, _)| ts).collect();
        assert_eq!(ts, [1, 2]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_partial_record_header_is_a_torn_tail() {
        let dir = dir("torn-header");
        let mut spill = Spill::open(&dir, 0, 0).unwrap();
        spill.append(&batch(1, 2)).unwrap();
        drop(spill);
        let path = dir.join(super::FILE_NAME);
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.extend_from_slice(&[9, 0, 0]);
        std::fs::write(&path, &bytes).unwrap();
        let mut again = Spill::open(&dir, 0, 0).unwrap();
        assert_eq!(again.torn_bytes(), 3);
        assert_eq!(collect(&mut again).len(), 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_file_without_a_complete_header_is_dropped_and_reported() {
        let dir = dir("torn-magic");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(super::FILE_NAME);
        std::fs::write(&path, b"PBS").unwrap();
        let spill = Spill::open(&dir, 0, 0).unwrap();
        assert_eq!(spill.torn_bytes(), 3);
        assert!(!spill.has_pending());
        assert!(!path.exists());
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
        assert_eq!(collect(&mut done), []);
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
        assert_eq!(collect(&mut again), []);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_record_of_ordinary_size_replays() {
        // Far more than a few kilobytes: the payload limit is 64 MiB.
        let dir = dir("large");
        let mut spill = Spill::open(&dir, 0, 0).unwrap();
        spill.append(&batch(1, 60)).unwrap();
        assert!(spill.pending_bytes() > 4_096);
        assert_eq!(collect(&mut spill).len(), 60);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_record_that_announces_more_than_the_limit_is_a_torn_tail() {
        let dir = dir("announce");
        std::fs::create_dir_all(&dir).unwrap();
        let mut file = b"PBSP".to_vec();
        file.extend_from_slice(&1_u32.to_le_bytes());
        // 64 MiB and one byte, a checksum, and a few bytes that are not it.
        file.extend_from_slice(&67_108_865_u32.to_le_bytes());
        file.extend_from_slice(&0_u32.to_le_bytes());
        file.extend_from_slice(&[7; 16]);
        std::fs::write(dir.join(super::FILE_NAME), &file).unwrap();
        let mut spill = Spill::open(&dir, 0, 0).unwrap();
        // Found at open: the 24 bytes after the header are cut, not replayed.
        assert_eq!(spill.torn_bytes(), 24);
        assert!(!spill.has_pending());
        let mut into = Batch::with_capacity(64, 4096);
        assert_eq!(spill.next(&mut into).unwrap(), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_file_with_only_its_header_is_a_valid_empty_spill() {
        let dir = dir("header-only");
        std::fs::create_dir_all(&dir).unwrap();
        let mut file = b"PBSP".to_vec();
        file.extend_from_slice(&5_u32.to_le_bytes());
        std::fs::write(dir.join(super::FILE_NAME), &file).unwrap();
        let spill = Spill::open(&dir, 9, 0).unwrap();
        assert_eq!(spill.generation(), 5);
        assert!(!spill.has_pending());
        assert_eq!((spill.torn_bytes(), spill.end()), (0, 8));
        assert!(dir.join(super::FILE_NAME).exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A spill file with `records` records of `batch(n, 2)`; returns its path
    /// and the length of each record.
    fn written(dir: &std::path::Path, records: u64) -> (std::path::PathBuf, usize) {
        let mut spill = Spill::open(dir, 0, 0).unwrap();
        for index in 0..records {
            spill.append(&batch(index * 2 + 1, 2)).unwrap();
        }
        drop(spill);
        (dir.join(super::FILE_NAME), record_len(&batch(1, 2)))
    }

    #[test]
    fn a_file_of_whole_records_is_taken_whole() {
        let dir = dir("whole");
        let (path, record) = written(&dir, 3);
        let size = std::fs::metadata(&path).unwrap().len();
        let mut again = Spill::open(&dir, 0, 0).unwrap();
        assert_eq!((again.torn_bytes(), again.end()), (0, size));
        assert_eq!(size, (8 + 3 * record) as u64);
        assert_eq!(collect(&mut again).len(), 6);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_tail_of_one_header_without_a_record_is_cut() {
        let dir = dir("zero-header");
        let (path, record) = written(&dir, 2);
        let mut bytes = std::fs::read(&path).unwrap();
        // A header of its own size: zero length, zero checksum.
        bytes.extend_from_slice(&[0; 8]);
        std::fs::write(&path, &bytes).unwrap();
        let again = Spill::open(&dir, 0, 0).unwrap();
        assert_eq!(again.torn_bytes(), 8);
        assert_eq!(again.end(), (8 + 2 * record) as u64);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_record_is_cut_for_a_bad_checksum_alone_or_a_bad_payload_alone() {
        use std::io::Write as _;

        // The checksum field is wrong, the payload is whole.
        let dir_crc = dir("crc-only");
        let (path, record) = written(&dir_crc, 2);
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[8 + record + 4] ^= 0xFF;
        std::fs::write(&path, &bytes).unwrap();
        let again = Spill::open(&dir_crc, 0, 0).unwrap();
        assert_eq!(again.torn_bytes(), record as u64);
        std::fs::remove_dir_all(&dir_crc).unwrap();

        // The checksum is right for a payload that is not a batch.
        let dir_payload = dir("payload-only");
        let (path, record) = written(&dir_payload, 1);
        let junk = [0xFF_u8; 24];
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        file.write_all(&u32::try_from(junk.len()).unwrap().to_le_bytes())
            .unwrap();
        file.write_all(&crc32(&junk).to_le_bytes()).unwrap();
        file.write_all(&junk).unwrap();
        drop(file);
        let again = Spill::open(&dir_payload, 0, 0).unwrap();
        assert_eq!(again.torn_bytes(), 8 + junk.len() as u64);
        assert_eq!(again.end(), (8 + record) as u64);
        std::fs::remove_dir_all(&dir_payload).unwrap();
    }

    #[test]
    fn a_torn_record_met_while_replaying_ends_the_replay_there() {
        let dir = dir("torn-while-replaying");
        let (path, record) = written(&dir, 3);
        let mut spill = Spill::open(&dir, 0, 0).unwrap();
        // The file is damaged after it was opened.
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[8 + record + 12] ^= 0xFF;
        std::fs::write(&path, &bytes).unwrap();
        let mut into = Batch::with_capacity(64, 4096);
        assert!(spill.next(&mut into).unwrap().is_some());
        assert_eq!(spill.next(&mut into).unwrap(), None);
        assert!(!spill.has_pending());
        assert_eq!(spill.pending_bytes(), 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pending_bytes_counts_what_is_written_and_not_yet_replayed() {
        let dir = dir("pending");
        let mut spill = Spill::open(&dir, 0, 0).unwrap();
        assert_eq!(spill.pending_bytes(), 0);
        spill.append(&batch(1, 4)).unwrap();
        let length = std::fs::metadata(dir.join(super::FILE_NAME)).unwrap().len();
        assert_eq!(spill.pending_bytes(), length - 8);
        assert_eq!(collect(&mut spill).len(), 4);
        assert_eq!(spill.pending_bytes(), 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn finishing_without_a_file_is_not_an_error() {
        let dir = dir("no-file");
        let mut spill = Spill::open(&dir, 0, 0).unwrap();
        spill.finish().unwrap();
        spill.finish().unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_record_of_more_than_a_megabyte_replays() {
        // Past 64 KiB and past 1 MiB: the payload limit is 64 MiB, and a
        // limit computed with the wrong operator would be far below it.
        let dir = dir("megabyte");
        let mut spill = Spill::open(&dir, 0, 0).unwrap();
        let mut big = Batch::with_capacity(14_000, 14_000 * 32);
        for ts in 1..=14_000u64 {
            big.push(event(ts, 0), &units(r"d\f")).unwrap();
        }
        spill.append(&big).unwrap();
        assert!(
            spill.pending_bytes() > 1_200_000,
            "{}",
            spill.pending_bytes()
        );
        // Read into a batch as large as the one written.
        let mut into = Batch::with_capacity(14_000, 14_000 * 32);
        assert!(spill.next(&mut into).unwrap().is_some());
        assert_eq!(into.iter().count(), 14_000);
        assert!(spill.next(&mut into).unwrap().is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn finishing_reports_a_file_that_cannot_be_removed() {
        let dir = dir("unremovable");
        let mut spill = Spill::open(&dir, 0, 0).unwrap();
        // A directory where the file belongs: removing it as a file fails with
        // something other than "not found".
        std::fs::create_dir_all(dir.join(super::FILE_NAME)).unwrap();
        assert!(spill.finish().is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn whole_records_stops_at_the_last_complete_record() {
        let dir = dir("whole-bound");
        let mut spill = Spill::open(&dir, 0, 0).unwrap();
        let (first, second) = (batch(1, 3), batch(10, 4));
        spill.append(&first).unwrap();
        spill.append(&second).unwrap();
        drop(spill);
        // The file starts with an 8-byte header of its own.
        let start = 8;
        let first_end = start + record_len(&first) as u64;
        let end = first_end + record_len(&second) as u64;
        let mut file = std::fs::File::open(dir.join(super::FILE_NAME)).unwrap();
        assert_eq!(file.metadata().unwrap().len(), end);

        assert_eq!(whole_records(&mut file, start, end).unwrap(), end);
        assert_eq!(whole_records(&mut file, first_end, end).unwrap(), end);
        // A cut inside the second header or payload, with the bytes still
        // in the file: the bound alone decides.
        for cut in [first_end + 5, first_end + 8, end - 1] {
            assert_eq!(
                whole_records(&mut file, start, cut).unwrap(),
                first_end,
                "{cut}"
            );
        }
        assert_eq!(
            whole_records(&mut file, start, first_end - 1).unwrap(),
            start
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
