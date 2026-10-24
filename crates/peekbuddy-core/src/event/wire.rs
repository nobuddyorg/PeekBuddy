//! Versioned binary wire codec for events (little-endian).
//!
//! Used by the IPC live tail (#61) and the writer spill queue (#52). The
//! first byte is the format version; names travel as u16 units. Encoding
//! writes into a caller buffer and decoding stores the name in a caller
//! [`NameArena`], so neither allocates.
//!
//! Layout (version 1): version, source, kind tag, kind payload, name kind,
//! `is_dir` (one byte each); flags, vol, root (u16 each); ts, qpc, `src_ts`
//! (u64 each); `raw_mask`, `source_info` (u32 each); size (u64); attrs,
//! `reparse_tag` (u32 each); `file_id`, `parent_file_id` (u128 each); usn
//! (i64); name length (u16); the name units.

use super::{Event, Flags, Kind, MAX_NAME_UNITS, NameArena, NameKind, RootIdx, Source, VolIdx};

/// The current format version.
pub const VERSION: u8 = 1;

/// Bytes before the name units.
pub const FIXED_LEN: usize = 6 + 6 + 24 + 8 + 8 + 8 + 32 + 8 + 2;

/// The largest encoded event.
pub const MAX_ENCODED_LEN: usize = FIXED_LEN + 2 * MAX_NAME_UNITS;

/// Why an event could not be encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodeError {
    /// The output buffer is too small.
    BufferTooSmall,
    /// The name has more than [`MAX_NAME_UNITS`] units.
    NameTooLong,
}

/// Why bytes are not a valid event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// The input ended early.
    Truncated,
    /// The version byte is not [`VERSION`].
    UnknownVersion(u8),
    /// A tag or flag field holds an undefined value.
    InvalidField,
    /// The name is longer than allowed.
    NameTooLong,
    /// The name arena has no room for the name.
    ArenaFull,
}

struct Writer<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

impl Writer<'_> {
    fn put(&mut self, bytes: &[u8]) -> Result<(), EncodeError> {
        let end = self
            .pos
            .checked_add(bytes.len())
            .ok_or(EncodeError::BufferTooSmall)?;
        self.buf
            .get_mut(self.pos..end)
            .ok_or(EncodeError::BufferTooSmall)?
            .copy_from_slice(bytes);
        self.pos = end;
        Ok(())
    }
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let end = self.pos.checked_add(n).ok_or(DecodeError::Truncated)?;
        let s = self.buf.get(self.pos..end).ok_or(DecodeError::Truncated)?;
        self.pos = end;
        Ok(s)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        self.take(N)?.try_into().map_err(|_| DecodeError::Truncated)
    }

    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(u8::from_le_bytes(self.array()?))
    }
    fn u16(&mut self) -> Result<u16, DecodeError> {
        Ok(u16::from_le_bytes(self.array()?))
    }
    fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(self.array()?))
    }
    fn u64(&mut self) -> Result<u64, DecodeError> {
        Ok(u64::from_le_bytes(self.array()?))
    }
    fn i64(&mut self) -> Result<i64, DecodeError> {
        Ok(i64::from_le_bytes(self.array()?))
    }
    fn u128(&mut self) -> Result<u128, DecodeError> {
        Ok(u128::from_le_bytes(self.array()?))
    }
}

/// The encoded length of an event with a name of `name_len` units.
#[must_use]
pub const fn encoded_len(name_len: usize) -> usize {
    FIXED_LEN + 2 * name_len
}

/// Encode `ev` and its `name` into `out`; returns the bytes written.
///
/// The name reference inside `ev` is not written: the name travels inline.
///
/// # Errors
///
/// [`EncodeError::NameTooLong`] or [`EncodeError::BufferTooSmall`].
pub fn encode(ev: &Event, name: &[u16], out: &mut [u8]) -> Result<usize, EncodeError> {
    if name.len() > MAX_NAME_UNITS {
        return Err(EncodeError::NameTooLong);
    }
    let name_len = u16::try_from(name.len()).map_err(|_| EncodeError::NameTooLong)?;
    let (kind, arg) = ev.kind.to_wire();
    let mut w = Writer { buf: out, pos: 0 };
    w.put(&[
        VERSION,
        ev.source.tag(),
        kind,
        arg,
        ev.name_kind.tag(),
        u8::from(ev.is_dir),
    ])?;
    w.put(&ev.flags.bits().to_le_bytes())?;
    w.put(&ev.vol.0.to_le_bytes())?;
    w.put(&ev.root.0.to_le_bytes())?;
    w.put(&ev.ts.to_le_bytes())?;
    w.put(&ev.qpc.to_le_bytes())?;
    w.put(&ev.src_ts.to_le_bytes())?;
    w.put(&ev.raw_mask.to_le_bytes())?;
    w.put(&ev.source_info.to_le_bytes())?;
    w.put(&ev.size.to_le_bytes())?;
    w.put(&ev.attrs.to_le_bytes())?;
    w.put(&ev.reparse_tag.to_le_bytes())?;
    w.put(&ev.file_id.to_le_bytes())?;
    w.put(&ev.parent_file_id.to_le_bytes())?;
    w.put(&ev.usn.to_le_bytes())?;
    w.put(&name_len.to_le_bytes())?;
    for unit in name {
        w.put(&unit.to_le_bytes())?;
    }
    Ok(w.pos)
}

/// Decode one event from the start of `bytes`, storing its name in `arena`.
/// Returns the event (its name reference points into `arena`) and the number
/// of bytes consumed.
///
/// # Errors
///
/// A [`DecodeError`] for truncated or invalid input, or a full arena.
pub fn decode(bytes: &[u8], arena: &mut NameArena) -> Result<(Event, usize), DecodeError> {
    let mut r = Reader { buf: bytes, pos: 0 };
    let version = r.u8()?;
    if version != VERSION {
        return Err(DecodeError::UnknownVersion(version));
    }
    let source = Source::from_tag(r.u8()?).ok_or(DecodeError::InvalidField)?;
    let kind_tag = r.u8()?;
    let kind_arg = r.u8()?;
    let kind = Kind::from_wire(kind_tag, kind_arg).ok_or(DecodeError::InvalidField)?;
    let name_kind = NameKind::from_tag(r.u8()?).ok_or(DecodeError::InvalidField)?;
    let is_dir = match r.u8()? {
        0 => false,
        1 => true,
        _ => return Err(DecodeError::InvalidField),
    };
    let flags = Flags::from_bits(r.u16()?).ok_or(DecodeError::InvalidField)?;
    let vol = VolIdx(r.u16()?);
    let root = RootIdx(r.u16()?);
    let ts = r.u64()?;
    let qpc = r.u64()?;
    let src_ts = r.u64()?;
    let raw_mask = r.u32()?;
    let source_info = r.u32()?;
    let size = r.u64()?;
    let attrs = r.u32()?;
    let reparse_tag = r.u32()?;
    let file_id = r.u128()?;
    let parent_file_id = r.u128()?;
    let usn = r.i64()?;
    let name_len = usize::from(r.u16()?);
    if name_len > MAX_NAME_UNITS {
        return Err(DecodeError::NameTooLong);
    }
    let raw_name = r.take(name_len * 2)?;
    let units = raw_name
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c));
    let name = arena
        .push_iter(name_len, units)
        .ok_or(DecodeError::ArenaFull)?;
    let ev = Event {
        ts,
        qpc,
        src_ts,
        source,
        kind,
        raw_mask,
        source_info,
        is_dir,
        size,
        attrs,
        reparse_tag,
        file_id,
        parent_file_id,
        usn,
        vol,
        root,
        name,
        name_kind,
        flags,
    };
    Ok((ev, r.pos))
}

#[cfg(test)]
mod tests {
    use super::super::kind::tests::all_kinds;
    use super::super::tests::sample;
    use super::*;
    use proptest::prelude::*;

    fn arb_event() -> impl Strategy<Value = Event> {
        let kinds = all_kinds();
        let head = (
            any::<(u64, u64, u64)>(),
            0..Source::ALL.len(),
            0..kinds.len(),
            any::<(u32, u32, bool)>(),
            any::<(u64, u32, u32)>(),
        );
        let tail = (
            any::<(u128, u128, i64)>(),
            any::<(u16, u16)>(),
            0..NameKind::ALL.len(),
            0..=Flags::ALL_BITS,
        );
        (head, tail).prop_map(move |(h, t)| Event {
            ts: h.0.0,
            qpc: h.0.1,
            src_ts: h.0.2,
            source: Source::ALL.get(h.1).copied().unwrap_or(Source::System),
            kind: kinds.get(h.2).copied().unwrap_or(Kind::Other),
            raw_mask: h.3.0,
            source_info: h.3.1,
            is_dir: h.3.2,
            size: h.4.0,
            attrs: h.4.1,
            reparse_tag: h.4.2,
            file_id: t.0.0,
            parent_file_id: t.0.1,
            usn: t.0.2,
            vol: VolIdx(t.1.0),
            root: RootIdx(t.1.1),
            name: super::super::NameRef::EMPTY,
            name_kind: NameKind::ALL.get(t.2).copied().unwrap_or(NameKind::Leaf),
            flags: Flags::from_bits(t.3).unwrap_or(Flags::NONE),
        })
    }

    fn set(buf: &mut [u8], i: usize, v: u8) {
        *buf.get_mut(i).unwrap() = v;
    }

    fn round_trip(ev: &Event, name: &[u16]) {
        let mut buf = vec![0_u8; encoded_len(name.len())];
        let n = encode(ev, name, &mut buf).unwrap();
        assert_eq!(n, buf.len());
        let mut arena = NameArena::with_capacity(MAX_NAME_UNITS);
        let (got, used) = decode(&buf, &mut arena).unwrap();
        assert_eq!(used, n);
        assert_eq!(arena.get(got.name), Some(name));
        assert_eq!(
            Event {
                name: ev.name,
                ..got
            },
            *ev
        );
    }

    #[test]
    fn fixed_len_matches_the_layout() {
        let mut buf = [0_u8; FIXED_LEN];
        assert_eq!(encode(&sample(), &[], &mut buf), Ok(FIXED_LEN));
    }

    #[test]
    fn sample_round_trips() {
        round_trip(&sample(), &[0x61, 0xD83D, 0xDE00]);
    }

    #[test]
    fn max_length_name_round_trips() {
        let name = vec![0xFFFF_u16; MAX_NAME_UNITS];
        round_trip(&sample(), &name);
    }

    #[test]
    fn name_one_over_the_limit_is_rejected() {
        let name = vec![0_u16; MAX_NAME_UNITS + 1];
        let mut buf = vec![0_u8; MAX_ENCODED_LEN + 2];
        assert_eq!(
            encode(&sample(), &name, &mut buf),
            Err(EncodeError::NameTooLong)
        );
    }

    #[test]
    fn small_buffer_is_rejected() {
        let mut buf = [0_u8; FIXED_LEN - 1];
        assert_eq!(
            encode(&sample(), &[], &mut buf),
            Err(EncodeError::BufferTooSmall)
        );
    }

    #[test]
    fn rejects_bad_version_fields_and_truncation() {
        let mut buf = vec![0_u8; encoded_len(2)];
        encode(&sample(), &[1, 2], &mut buf).unwrap();
        let mut arena = NameArena::with_capacity(8);

        let mut bad = buf.clone();
        set(&mut bad, 0, 9);
        assert_eq!(
            decode(&bad, &mut arena),
            Err(DecodeError::UnknownVersion(9))
        );
        for (i, v) in [(1_usize, 200_u8), (2, 200), (4, 9), (5, 2)] {
            let mut bad = buf.clone();
            set(&mut bad, i, v);
            assert_eq!(
                decode(&bad, &mut arena),
                Err(DecodeError::InvalidField),
                "byte {i}"
            );
        }
        let mut bad = buf.clone();
        set(&mut bad, 6, 0xFF);
        set(&mut bad, 7, 0xFF);
        assert_eq!(decode(&bad, &mut arena), Err(DecodeError::InvalidField));
        assert_eq!(
            decode(
                buf.split_last().map_or(&[][..], |(_, rest)| rest),
                &mut arena
            ),
            Err(DecodeError::Truncated)
        );
        assert_eq!(decode(&[], &mut arena), Err(DecodeError::Truncated));
        assert!(arena.is_empty());
    }

    #[test]
    fn full_arena_is_reported() {
        let mut buf = vec![0_u8; encoded_len(3)];
        encode(&sample(), &[1, 2, 3], &mut buf).unwrap();
        let mut arena = NameArena::with_capacity(2);
        assert_eq!(decode(&buf, &mut arena), Err(DecodeError::ArenaFull));
    }

    proptest! {
        #[test]
        fn arbitrary_events_round_trip(
            ev in arb_event(),
            name in proptest::collection::vec(any::<u16>(), 0..64),
        ) {
            round_trip(&ev, &name);
        }

        #[test]
        fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..256)) {
            let mut arena = NameArena::with_capacity(256);
            let _ = decode(&bytes, &mut arena);
        }

        #[test]
        fn mutated_valid_bytes_never_panic(
            ev in arb_event(),
            pos in 0_usize..FIXED_LEN + 8,
            val in any::<u8>(),
        ) {
            let mut buf = vec![0_u8; encoded_len(4)];
            encode(&ev, &[1, 2, 3, 4], &mut buf).unwrap();
            if let Some(b) = buf.get_mut(pos) {
                *b = val;
            }
            let mut arena = NameArena::with_capacity(256);
            let _ = decode(&buf, &mut arena);
        }
    }
}
