//! The saved journal position of one root, kept by the store (#260).
//!
//! The USN source saves a [`Checkpoint`] after the events of every buffer were
//! handed to the sink; it goes into the writer queue as a [`Cursor`] behind
//! them ([`Producer::set_cursor`]), so the committed cursor is never ahead of
//! the committed rows. At start the source asks for the position again: the
//! newest one this run saved, or the one the database had when the pipeline
//! started ([`Producer::saved_cursor`]). The journal id goes with the USN
//! (schema v4), because a USN means nothing in another journal.

use peekbuddy_core::event::RootIdx;
use peekbuddy_core::usn::checkpoint::Checkpoint;

use crate::pipeline::Producer;
use crate::writer::Cursor;

/// The checkpoints of one root over the pipeline.
#[derive(Debug, Clone)]
pub struct RootCheckpoints {
    producer: Producer,
    root: RootIdx,
}

impl RootCheckpoints {
    /// The checkpoints of `root`, written through `producer`.
    #[must_use]
    pub const fn new(producer: Producer, root: RootIdx) -> Self {
        Self { producer, root }
    }

    /// The position to resume from; `None` when the root has none that names
    /// both a journal and a USN (a first run, or a cursor of another source).
    #[must_use]
    pub fn saved(&self) -> Option<Checkpoint> {
        self.producer
            .saved_cursor(self.root)
            .and_then(|cursor| checkpoint_of(&cursor))
    }

    /// Queues `checkpoint` behind the rows pushed so far.
    pub fn record(&self, checkpoint: Checkpoint) {
        self.producer.set_cursor(cursor_of(self.root, checkpoint));
    }
}

/// The cursor row that holds `checkpoint`: the journal id is bit-cast to the
/// signed column type.
#[must_use]
pub const fn cursor_of(root: RootIdx, checkpoint: Checkpoint) -> Cursor {
    Cursor {
        root,
        usn: Some(checkpoint.next_usn),
        journal_id: Some(checkpoint.journal_id.cast_signed()),
    }
}

/// The checkpoint a cursor row holds, when it has both halves.
#[must_use]
pub const fn checkpoint_of(cursor: &Cursor) -> Option<Checkpoint> {
    match (cursor.usn, cursor.journal_id) {
        (Some(next_usn), Some(journal_id)) => Some(Checkpoint {
            journal_id: journal_id.cast_unsigned(),
            next_usn,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use peekbuddy_core::event::RootIdx;
    use peekbuddy_core::usn::checkpoint::Checkpoint;

    use super::{checkpoint_of, cursor_of};
    use crate::writer::Cursor;

    #[test]
    fn a_journal_id_above_the_signed_range_survives_the_column() {
        let checkpoint = Checkpoint {
            journal_id: u64::MAX - 3,
            next_usn: 1 << 40,
        };
        let cursor = cursor_of(RootIdx(4), checkpoint);
        assert_eq!(cursor.root, RootIdx(4));
        assert_eq!(cursor.journal_id, Some(-4));
        assert_eq!(checkpoint_of(&cursor), Some(checkpoint));
    }

    #[test]
    fn a_cursor_without_a_journal_or_a_usn_is_no_checkpoint() {
        for (usn, journal_id) in [(Some(5), None), (None, Some(5)), (None, None)] {
            let cursor = Cursor {
                root: RootIdx(0),
                usn,
                journal_id,
            };
            assert_eq!(checkpoint_of(&cursor), None, "{cursor:?}");
        }
    }
}
