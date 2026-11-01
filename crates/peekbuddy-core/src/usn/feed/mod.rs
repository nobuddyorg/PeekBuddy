//! From a journal buffer to events (#212).
//!
//! The OS-independent half of the USN capture source: the Windows side reads
//! the buffers and answers [`Names`]; everything else is here, so it builds
//! and tests on Linux. A [`Feed`] holds the [`Scope`] of one watched root and
//! turns each accepted record into an [`Event`] for an [`EventSink`], with
//! the name relative to the root like the RDCW events carry.
//!
//! # No row is dropped for a missing name
//!
//! The unprivileged read returns no names (spike 0001). A record without one
//! asks [`Names::path_of`] for the path of its file reference number. When
//! that fails (the file is gone, or cannot be opened) the row is still
//! written, with the path of its parent directory and
//! [`Flags::NAME_UNRESOLVED`]; the same flag marks a path that has a
//! directory without a name in it.
//!
//! # Checkpoint
//!
//! Events carry the record's USN, and the writer stores the last one with the
//! batch it covers. [`Feed::resume_after`] makes a restarted feed skip every
//! record up to that USN, so a record is never written twice.
//!
//! # Loss is never silent
//!
//! A sink that is full drops the rest of the batch and the feed owes one
//! [`GapReason::RingFull`] row; a damaged buffer owes a
//! [`GapReason::ParserError`] row. Owed rows go out first at the start of the
//! next batch. [`Feed::gap`] writes the row of a resume decision or a failed
//! read at once.
//!
//! After the first batch nothing allocates per file record: the scratch
//! buffers keep their capacity.

use crate::capture::{EventSink, Stamp};
use crate::event::map::usn_kind;
use crate::event::{
    Event, Flags, GapReason, Kind, MAX_NAME_UNITS, NameKind, NameRef, RootIdx, Source, VolIdx,
};
use crate::name::{CaseMode, WideStr, eq_ignore_case_ordinal};
use crate::usn::record::{DecodeError, Record, records};
use crate::usn::scope::{Action, Scope};

const BACKSLASH: u16 = 0x5C;

/// What the Windows side can look up. Both calls may be slow (they open
/// files); `path_of` is not called for a record that carries its name.
pub trait Names {
    /// Writes the volume-relative path of the file with reference number
    /// `frn` (like `\dir\file.txt`) into `out`; `false` when it cannot be
    /// resolved (deleted, reused slot, no access).
    fn path_of(&mut self, frn: u128, out: &mut Vec<u16>) -> bool;

    /// Adds every directory below the directory `frn` to `scope` (the
    /// reconcile walker's directory walk). Called when a directory moved
    /// into the tree: its descendants emit no records.
    fn walk(&mut self, frn: u128, scope: &mut Scope);
}

/// What the feed counted, all monotonic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Counters {
    /// Buffers decoded.
    pub batches: u64,
    /// Events the sink took for records.
    pub records: u64,
    /// Records outside the watched tree.
    pub outside: u64,
    /// Records skipped because they are at or before the resume point.
    pub replayed: u64,
    /// Rows written without a resolved name.
    pub unresolved: u64,
    /// Records the sink refused, plus those skipped after it.
    pub dropped_full: u64,
    /// Buffers (or the rest of one) that failed to decode.
    pub parse_errors: u64,
    /// Gap rows the sink took.
    pub gap_rows: u64,
    /// Subtree walks asked for.
    pub walks: u64,
}

/// The feed of one watched root on one volume.
#[derive(Debug)]
pub struct Feed {
    scope: Scope,
    vol: VolIdx,
    root: RootIdx,
    /// Volume-relative path of the root, no trailing separator.
    root_path: Vec<u16>,
    after: Option<i64>,
    owed: [u64; GapReason::ALL.len()],
    counters: Counters,
    parent: Vec<u16>,
    name: Vec<u16>,
    resolved: Vec<u16>,
}

impl Feed {
    /// A feed for `root` of `scope`; `root_path` is the root's
    /// volume-relative path (`\Users\me\proj`, empty for a volume root).
    #[must_use]
    pub fn new(scope: Scope, vol: VolIdx, root: RootIdx, root_path: &[u16]) -> Self {
        let mut root_path = root_path.to_vec();
        while root_path.last() == Some(&BACKSLASH) {
            root_path.pop();
        }
        Self {
            scope,
            vol,
            root,
            root_path,
            after: None,
            owed: [0; GapReason::ALL.len()],
            counters: Counters::default(),
            parent: Vec::new(),
            name: Vec::new(),
            resolved: Vec::new(),
        }
    }

    /// Skips the records up to and including `usn` (the checkpoint of the
    /// events already stored).
    pub const fn resume_after(&mut self, usn: i64) {
        self.after = Some(usn);
    }

    /// The scope, for the caller that fills it from a directory walk.
    pub const fn scope_mut(&mut self) -> &mut Scope {
        &mut self.scope
    }

    /// The scope.
    #[must_use]
    pub const fn scope(&self) -> &Scope {
        &self.scope
    }

    /// What was counted.
    #[must_use]
    pub const fn counters(&self) -> Counters {
        self.counters
    }

    /// Gap rows that are owed and not yet written.
    #[must_use]
    pub fn owed_total(&self) -> u64 {
        self.owed.iter().sum()
    }

    /// Writes one gap row now. `false` if the sink refused it; the row is
    /// then owed and goes out with the next batch.
    pub fn gap(&mut self, reason: GapReason, stamp: Stamp, sink: &mut impl EventSink) -> bool {
        if sink.push(&self.gap_event(reason, stamp), &[]) {
            self.counters.gap_rows += 1;
            true
        } else {
            self.owe(reason);
            false
        }
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

    /// Decodes one buffer of journal output and offers its events to
    /// `sink`. Returns the USN to read from next.
    ///
    /// # Errors
    ///
    /// [`DecodeError`] when the buffer has no header; a parser-error gap row
    /// is owed then. A buffer damaged further in yields the events before
    /// the damage and the USN from its header.
    pub fn decode(
        &mut self,
        buf: &[u8],
        stamp: Stamp,
        names: &mut impl Names,
        sink: &mut impl EventSink,
    ) -> Result<i64, DecodeError> {
        self.counters.batches += 1;
        self.flush_owed(stamp, sink);
        let iter = match records(buf) {
            Ok(iter) => iter,
            Err(error) => {
                self.counters.parse_errors += 1;
                self.owe(GapReason::ParserError);
                self.flush_owed(stamp, sink);
                return Err(error);
            }
        };
        let next = iter.next_usn();
        let mut full = false;
        for item in iter {
            let Ok(rec) = item else {
                self.counters.parse_errors += 1;
                self.owe(GapReason::ParserError);
                break;
            };
            if self.after.is_some_and(|after| rec.usn <= after) {
                self.counters.replayed += 1;
                continue;
            }
            let decision = self.scope.apply_into(&rec, &mut self.parent);
            let mut unresolved = decision.path_partial;
            match decision.action {
                Action::None => {}
                Action::Walk(frn) => {
                    self.counters.walks += 1;
                    names.walk(frn, &mut self.scope);
                }
                Action::NeedName(frn) => {
                    if names.path_of(frn, &mut self.resolved) {
                        let leaf = leaf_of(&self.resolved);
                        self.scope.set_name(frn, leaf);
                    }
                }
            }
            if !decision.accept {
                self.counters.outside += 1;
                continue;
            }
            unresolved |= self.compose_name(&rec, names);
            if unresolved {
                self.counters.unresolved += 1;
            }
            if full {
                self.counters.dropped_full += 1;
                continue;
            }
            let event = self.record_event(&rec, stamp, unresolved);
            if sink.push(&event, &self.name) {
                self.counters.records += 1;
            } else {
                full = true;
                self.counters.dropped_full += 1;
                self.owe(GapReason::RingFull);
            }
        }
        sink.batch_end();
        self.flush_owed(stamp, sink);
        Ok(next)
    }

    /// Fills `self.name` with the name relative to the root: parent path,
    /// then the leaf, from the record or from the file system. `true` when
    /// the name is not complete.
    fn compose_name(&mut self, rec: &Record<'_>, names: &mut impl Names) -> bool {
        self.name.clear();
        self.name.extend_from_slice(&self.parent);
        let mut unresolved = false;
        if rec.name_withheld() {
            if names.path_of(rec.frn, &mut self.resolved)
                && let Some(relative) = relative_to(&self.resolved, &self.root_path)
            {
                self.name.clear();
                self.name.extend_from_slice(relative);
            } else {
                unresolved = true;
            }
        } else {
            if !self.name.is_empty() {
                self.name.push(BACKSLASH);
            }
            self.name.extend(rec.name());
        }
        if self.name.len() > MAX_NAME_UNITS {
            self.name.truncate(MAX_NAME_UNITS);
            unresolved = true;
        }
        unresolved
    }

    /// The event of an accepted record, its name in `self.name`.
    fn record_event(&self, rec: &Record<'_>, stamp: Stamp, unresolved: bool) -> Event {
        Event {
            ts: stamp.ts,
            qpc: stamp.qpc,
            src_ts: rec.timestamp.cast_unsigned(),
            source: Source::Usn,
            kind: usn_kind(rec.reason),
            raw_mask: rec.reason,
            source_info: rec.source_info,
            is_dir: rec.is_dir(),
            size: 0,
            attrs: rec.attributes,
            reparse_tag: 0,
            file_id: rec.frn,
            parent_file_id: rec.parent,
            usn: rec.usn,
            vol: self.vol,
            root: self.root,
            name: NameRef {
                offset: 0,
                len: u16::try_from(self.name.len()).unwrap_or(0),
            },
            name_kind: NameKind::Unknown,
            flags: if unresolved {
                Flags::NAME_UNRESOLVED
            } else {
                Flags::NONE
            },
        }
    }

    fn owe(&mut self, reason: GapReason) {
        if let Some(n) = self.owed.get_mut(reason as usize) {
            *n += 1;
        }
    }

    fn gap_event(&self, reason: GapReason, stamp: Stamp) -> Event {
        Event {
            ts: stamp.ts,
            qpc: stamp.qpc,
            src_ts: 0,
            source: Source::Usn,
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
}

/// The last component of a volume-relative path.
fn leaf_of(path: &[u16]) -> &[u16] {
    path.rsplit(|&unit| unit == BACKSLASH)
        .next()
        .unwrap_or_default()
}

/// `path` below `root_path` (both volume-relative), without the separator;
/// `None` when `path` is not below it.
fn relative_to<'a>(path: &'a [u16], root_path: &[u16]) -> Option<&'a [u16]> {
    let head = path.get(..root_path.len())?;
    if !eq_ignore_case_ordinal(
        WideStr::new(head),
        WideStr::new(root_path),
        CaseMode::Insensitive,
    ) {
        return None;
    }
    let rest = path.get(root_path.len()..)?;
    match rest.split_first() {
        Some((&BACKSLASH, below)) if !below.is_empty() => Some(below),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
