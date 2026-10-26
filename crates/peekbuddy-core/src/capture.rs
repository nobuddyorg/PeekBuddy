//! From a completed `ReadDirectoryChanges*` buffer to events (#45).
//!
//! The Windows backend owns the I/O; everything it does with a finished
//! buffer lives here, so it builds and tests on Linux: parse (#44), map to
//! the event model (#38), drop the data directory's own churn, and hand each
//! event to an [`EventSink`] (filters, the ring to the writer).
//!
//! Nothing here allocates after [`Decoder::new`]: names are copied into one
//! scratch buffer and the sink borrows them.
//!
//! # Loss is never silent
//!
//! A sink that cannot take an event answers `false`. The decoder then counts
//! the record, stops pushing for the rest of the batch and owes one
//! [`GapReason::RingFull`] row. Owed rows are pushed first at the start of
//! the next batch (and by [`Decoder::flush_owed`]), so a gap row always
//! precedes newer events. Overflows owe a row the same way, so the number of
//! gap rows stored equals [`Counters::gaps_owed_total`] once the ring has
//! drained.

use crate::event::map::rdcw_kind;
use crate::event::{
    Event, Flags, GapReason, Kind, MAX_NAME_UNITS, NameKind, NameRef, RootIdx, Source, VolIdx,
};
use crate::name::fold_upper_into;
use crate::notify::{self, Class, ParseError};

/// One reading of the clocks: the wall time and a monotonic count taken
/// when a completion was dequeued. RDCW records carry no event time, so every
/// record of a completion shares one stamp and keeps its order in the batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stamp {
    /// `GetSystemTimePreciseAsFileTime`, 100 ns ticks.
    pub ts: u64,
    /// Monotonic time for ordering and latency inside the process.
    pub qpc: u64,
}

/// Where decoded events go.
pub trait EventSink {
    /// Offers one event with its name (UTF-16 units, valid for the call
    /// only). `false` means the sink is full and did not take it.
    ///
    /// [`Event::name`] has the length of `name` and offset 0: the sink places
    /// the units in its own arena and sets the reference.
    fn push(&mut self, event: &Event, name: &[u16]) -> bool;
}

/// What the decoder counted, all monotonic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Counters {
    /// Buffers decoded.
    pub batches: u64,
    /// Records the sink took.
    pub records: u64,
    /// Records dropped because they are under an excluded directory.
    pub excluded: u64,
    /// Records the sink refused (ring full), plus those skipped after it.
    pub dropped_full: u64,
    /// Overflows seen (`TRUE` with 0 bytes, or `ERROR_NOTIFY_ENUM_DIR`).
    pub overflows: u64,
    /// Buffers that failed to parse.
    pub parse_errors: u64,
    /// Gap rows the sink took.
    pub gap_rows: u64,
    /// Gap rows owed in total (taken or still waiting).
    pub gaps_owed_total: u64,
}

impl Counters {
    /// The growth since `earlier` (counters only grow, so this saturates at 0).
    #[must_use]
    pub const fn since(self, earlier: Self) -> Self {
        Self {
            batches: self.batches.saturating_sub(earlier.batches),
            records: self.records.saturating_sub(earlier.records),
            excluded: self.excluded.saturating_sub(earlier.excluded),
            dropped_full: self.dropped_full.saturating_sub(earlier.dropped_full),
            overflows: self.overflows.saturating_sub(earlier.overflows),
            parse_errors: self.parse_errors.saturating_sub(earlier.parse_errors),
            gap_rows: self.gap_rows.saturating_sub(earlier.gap_rows),
            gaps_owed_total: self.gaps_owed_total.saturating_sub(earlier.gaps_owed_total),
        }
    }
}

/// What a decoded buffer asks of the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Outcome {
    /// A reconcile of the root is needed for this reason (#46).
    pub reconcile: Option<GapReason>,
}

/// Decodes the completions of one root.
#[derive(Debug)]
pub struct Decoder {
    root: RootIdx,
    vol: VolIdx,
    class: Class,
    /// Upper-cased prefixes relative to the root; an empty one excludes the
    /// whole root.
    excluded: Vec<Vec<u16>>,
    name: Vec<u16>,
    folded: Vec<u16>,
    owed: [u32; GapReason::ALL.len()],
    counters: Counters,
}

/// The separator that ends a path component in a notify name.
const SEPARATOR: u16 = b'\\' as u16;

impl Decoder {
    /// A decoder for `root` on `vol`, reading `class` buffers. `excluded` are
    /// directories relative to the root (see [`relative_prefix`]).
    #[must_use]
    pub fn new(root: RootIdx, vol: VolIdx, class: Class, excluded: &[Vec<u16>]) -> Self {
        let excluded = excluded
            .iter()
            .map(|prefix| {
                let mut folded = vec![0; prefix.len()];
                let _ = fold_upper_into(prefix, &mut folded);
                folded
            })
            .collect();
        Self {
            root,
            vol,
            class,
            excluded,
            name: vec![0; MAX_NAME_UNITS],
            folded: vec![0; MAX_NAME_UNITS],
            owed: [0; GapReason::ALL.len()],
            counters: Counters::default(),
        }
    }

    /// The information class of the buffers (it changes only before the first
    /// buffer, when the backend falls back).
    pub const fn set_class(&mut self, class: Class) {
        self.class = class;
    }

    /// The counters so far.
    #[must_use]
    pub const fn counters(&self) -> Counters {
        self.counters
    }

    /// How many gap rows are still owed.
    #[must_use]
    pub fn owed(&self) -> u64 {
        self.owed.iter().map(|&n| u64::from(n)).sum()
    }

    /// Records an overflow of this root: one gap row is owed (and pushed at
    /// once if the sink has room) and the root needs a reconcile.
    pub fn overflow(
        &mut self,
        reason: GapReason,
        stamp: Stamp,
        sink: &mut impl EventSink,
    ) -> Outcome {
        self.counters.overflows += 1;
        self.owe(reason);
        self.flush_owed(stamp, sink);
        Outcome {
            reconcile: Some(reason),
        }
    }

    /// Decodes `buf[..bytes]`. `bytes == 0` is an overflow.
    pub fn decode(
        &mut self,
        buf: &[u8],
        bytes: usize,
        stamp: Stamp,
        sink: &mut impl EventSink,
    ) -> Outcome {
        if bytes == 0 {
            return self.overflow(GapReason::BufferOverflow, stamp, sink);
        }
        self.counters.batches += 1;
        let mut full = !self.flush_owed(stamp, sink);
        let mut outcome = Outcome::default();
        for item in notify::parse(buf, bytes, self.class) {
            match item {
                Ok(record) => {
                    if full {
                        self.counters.dropped_full += 1;
                        continue;
                    }
                    if !self.emit(&record, stamp, sink) {
                        full = true;
                    }
                }
                Err(error) => {
                    self.parse_failed(error, stamp, sink);
                    outcome.reconcile = Some(GapReason::ParserError);
                    break;
                }
            }
        }
        if full {
            self.owe(GapReason::RingFull);
            self.flush_owed(stamp, sink);
            outcome.reconcile = Some(GapReason::RingFull);
        }
        outcome
    }

    /// Pushes the gap rows that are owed; `false` if one could not go out.
    pub fn flush_owed(&mut self, stamp: Stamp, sink: &mut impl EventSink) -> bool {
        for reason in GapReason::ALL {
            while self.owed.get(reason as usize).is_some_and(|&n| n > 0) {
                if !sink.push(&self.gap_event(reason, stamp), &[]) {
                    return false;
                }
                if let Some(n) = self.owed.get_mut(reason as usize) {
                    *n -= 1;
                }
                self.counters.gap_rows += 1;
            }
        }
        true
    }

    fn parse_failed(&mut self, _error: ParseError, stamp: Stamp, sink: &mut impl EventSink) {
        self.counters.parse_errors += 1;
        self.owe(GapReason::ParserError);
        self.flush_owed(stamp, sink);
    }

    fn owe(&mut self, reason: GapReason) {
        if let Some(n) = self.owed.get_mut(reason as usize) {
            *n += 1;
            self.counters.gaps_owed_total += 1;
        }
    }

    fn gap_event(&self, reason: GapReason, stamp: Stamp) -> Event {
        Event {
            ts: stamp.ts,
            qpc: stamp.qpc,
            src_ts: 0,
            source: Source::Rdcw,
            kind: Kind::Overflow(reason),
            raw_mask: 0,
            source_info: 0,
            is_dir: false,
            size: 0,
            attrs: 0,
            reparse_tag: 0,
            file_id: 0,
            parent_file_id: 0,
            usn: 0,
            vol: self.vol,
            root: self.root,
            name: NameRef::EMPTY,
            name_kind: NameKind::Unknown,
            flags: Flags::NONE,
        }
    }

    /// Maps and pushes one record; `false` if the sink was full.
    fn emit(
        &mut self,
        record: &notify::Record<'_>,
        stamp: Stamp,
        sink: &mut impl EventSink,
    ) -> bool {
        let copied = record.name.copy_into(&mut self.name);
        let len = copied.unwrap_or(0);
        let flags = if copied.is_some() {
            Flags::NONE
        } else {
            Flags::NAME_UNRESOLVED
        };
        let name = self.name.get(..len).unwrap_or_default();
        if is_excluded(&self.excluded, &mut self.folded, name) {
            self.counters.excluded += 1;
            return true;
        }
        let extended = record.extended;
        let event = Event {
            ts: stamp.ts,
            qpc: stamp.qpc,
            src_ts: 0,
            source: Source::Rdcw,
            kind: rdcw_kind(record.action.raw()),
            raw_mask: record.action.raw(),
            source_info: 0,
            is_dir: extended.is_some_and(|e| e.is_dir()),
            size: extended.map_or(0, |e| u64::try_from(e.file_size).unwrap_or(0)),
            attrs: extended.map_or(0, |e| e.attributes),
            reparse_tag: extended.map_or(0, |e| e.reparse_tag),
            file_id: extended.map_or(0, |e| u128::from(e.file_id.cast_unsigned())),
            parent_file_id: extended.map_or(0, |e| u128::from(e.parent_file_id.cast_unsigned())),
            usn: 0,
            vol: self.vol,
            root: self.root,
            name: NameRef {
                offset: 0,
                len: u16::try_from(len).unwrap_or(0),
            },
            name_kind: extended
                .and_then(|e| e.name_flags)
                .map_or(NameKind::Unknown, NameKind::from_file_name_flags),
            flags,
        };
        if sink.push(&event, name) {
            self.counters.records += 1;
            true
        } else {
            self.counters.dropped_full += 1;
            false
        }
    }
}

/// Whether `name` is one of the `excluded` directories or below one; `folded`
/// is working memory for the upper-cased name.
fn is_excluded(excluded: &[Vec<u16>], folded: &mut [u16], name: &[u16]) -> bool {
    if excluded.is_empty() {
        return false;
    }
    let Some(folded) = folded.get_mut(..name.len()) else {
        return false;
    };
    for (slot, &unit) in folded.iter_mut().zip(name) {
        *slot = crate::name::fold_upper(unit);
    }
    let folded = &*folded;
    excluded.iter().any(|prefix| {
        folded.starts_with(prefix)
            && (folded.len() == prefix.len()
                || prefix.is_empty()
                || folded.get(prefix.len()) == Some(&SEPARATOR))
    })
}

/// The path of `other` relative to `root`, if `other` is `root` or below it.
/// Both are canonical (`\\?\C:\dir`, from `GetFinalPathNameByHandleW`) and
/// compared ignoring case; the result keeps `other`'s spelling and has no
/// leading or trailing separator. Equal paths give an empty prefix.
#[must_use]
pub fn relative_prefix(root: &[u16], other: &[u16]) -> Option<Vec<u16>> {
    let root = root.strip_suffix(&[SEPARATOR]).unwrap_or(root);
    let other = other.strip_suffix(&[SEPARATOR]).unwrap_or(other);
    let head = other.get(..root.len())?;
    let same = head
        .iter()
        .zip(root)
        .all(|(&a, &b)| crate::name::fold_upper(a) == crate::name::fold_upper(b));
    if !same {
        return None;
    }
    let rest = other.get(root.len()..)?;
    if rest.is_empty() {
        return Some(Vec::new());
    }
    rest.strip_prefix(&[SEPARATOR]).map(<[u16]>::to_vec)
}

#[cfg(test)]
mod tests;
