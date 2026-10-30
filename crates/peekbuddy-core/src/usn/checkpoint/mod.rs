//! Resuming a journal read, and the gaps that must be explicit (#47).
//!
//! The checkpoint is `(volume GUID, UsnJournalID, next USN)`, committed with
//! the batch it covers (#52). On start the journal is queried and the saved
//! checkpoint is compared with what is there; anything but an intact journal
//! is a gap row followed by a reconcile (#46), never a silent restart.

use crate::event::GapReason;

/// `ERROR_JOURNAL_DELETE_IN_PROGRESS`.
pub const ERROR_JOURNAL_DELETE_IN_PROGRESS: u32 = 1178;
/// `ERROR_JOURNAL_NOT_ACTIVE`.
pub const ERROR_JOURNAL_NOT_ACTIVE: u32 = 1179;
/// `ERROR_JOURNAL_ENTRY_DELETED`.
pub const ERROR_JOURNAL_ENTRY_DELETED: u32 = 1181;
/// `ERROR_INVALID_USER_BUFFER`: overlapped reads refused (`ReFS`, Dev Drive).
pub const ERROR_INVALID_USER_BUFFER: u32 = 1784;

/// Where a read stopped, per volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Checkpoint {
    /// `UsnJournalID` the USN belongs to.
    pub journal_id: u64,
    /// The next USN to read.
    pub next_usn: i64,
}

/// The fields of `USN_JOURNAL_DATA_V2` that resuming needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JournalData {
    /// `UsnJournalID`.
    pub journal_id: u64,
    /// `FirstUsn`.
    pub first_usn: i64,
    /// `NextUsn`: the USN the next change will get.
    pub next_usn: i64,
    /// `LowestValidUsn`.
    pub lowest_valid_usn: i64,
}

/// What to do at start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resume {
    /// The journal is intact: read from the saved USN, no gap.
    Continue {
        /// The USN to read from.
        from: i64,
    },
    /// Nothing saved (first run): start at the end, there is no history to
    /// be missing. A reconcile establishes the baseline (#46).
    Fresh {
        /// The USN to read from (the journal's `NextUsn`).
        from: i64,
    },
    /// Records may have been lost: write a gap row, reconcile, and read from
    /// the journal's `NextUsn`.
    Gap {
        /// Why.
        reason: GapReason,
        /// The USN to read from after the gap.
        from: i64,
    },
}

/// Compares the saved checkpoint with the journal as it is now.
#[must_use]
pub fn plan_resume(saved: Option<Checkpoint>, now: &JournalData) -> Resume {
    let Some(saved) = saved else {
        return Resume::Fresh { from: now.next_usn };
    };
    if saved.journal_id != now.journal_id {
        return Resume::Gap {
            reason: GapReason::JournalIdChanged,
            from: now.next_usn,
        };
    }
    if saved.next_usn < now.lowest_valid_usn || saved.next_usn < now.first_usn {
        return Resume::Gap {
            reason: GapReason::JournalEntryDeleted,
            from: now.next_usn,
        };
    }
    if saved.next_usn > now.next_usn {
        // The journal was reset under the same id; the saved USN is in the
        // future of it.
        return Resume::Gap {
            reason: GapReason::JournalIdChanged,
            from: now.next_usn,
        };
    }
    Resume::Continue {
        from: saved.next_usn,
    }
}

/// The gap a failed read stands for; `None` for errors that are not gaps.
///
/// There is no separate gap reason for a journal that is not active or being
/// deleted: both mean the journal is about to be recreated with a new id.
#[must_use]
pub const fn gap_for_error(code: u32) -> Option<GapReason> {
    match code {
        ERROR_JOURNAL_ENTRY_DELETED => Some(GapReason::JournalEntryDeleted),
        ERROR_JOURNAL_NOT_ACTIVE | ERROR_JOURNAL_DELETE_IN_PROGRESS => {
            Some(GapReason::JournalIdChanged)
        }
        _ => None,
    }
}

/// Whether a failed overlapped read means the volume must be read
/// synchronously on a dedicated thread.
#[must_use]
pub const fn needs_synchronous_reads(code: u32) -> bool {
    code == ERROR_INVALID_USER_BUFFER
}

#[cfg(test)]
mod tests;
