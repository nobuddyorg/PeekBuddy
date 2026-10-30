use super::*;

const NOW: JournalData = JournalData {
    journal_id: 7,
    first_usn: 0,
    next_usn: 5_000,
    lowest_valid_usn: 1_000,
};

#[test]
fn nothing_saved_starts_fresh_at_the_end() {
    assert_eq!(plan_resume(None, &NOW), Resume::Fresh { from: 5_000 });
}

#[test]
fn an_intact_journal_continues_without_a_gap() {
    let saved = Checkpoint {
        journal_id: 7,
        next_usn: 3_000,
    };
    assert_eq!(
        plan_resume(Some(saved), &NOW),
        Resume::Continue { from: 3_000 }
    );
    let at_end = Checkpoint {
        next_usn: 5_000,
        ..saved
    };
    assert_eq!(
        plan_resume(Some(at_end), &NOW),
        Resume::Continue { from: 5_000 }
    );
}

#[test]
fn a_changed_journal_id_is_exactly_one_gap() {
    let saved = Checkpoint {
        journal_id: 6,
        next_usn: 3_000,
    };
    assert_eq!(
        plan_resume(Some(saved), &NOW),
        Resume::Gap {
            reason: GapReason::JournalIdChanged,
            from: 5_000
        }
    );
}

#[test]
fn a_saved_usn_below_the_lowest_valid_one_is_a_gap() {
    let saved = Checkpoint {
        journal_id: 7,
        next_usn: 999,
    };
    assert_eq!(
        plan_resume(Some(saved), &NOW),
        Resume::Gap {
            reason: GapReason::JournalEntryDeleted,
            from: 5_000
        }
    );
}

#[test]
fn a_saved_usn_in_the_future_of_the_journal_is_a_gap() {
    let saved = Checkpoint {
        journal_id: 7,
        next_usn: 9_000,
    };
    assert!(matches!(plan_resume(Some(saved), &NOW), Resume::Gap { .. }));
}

#[test]
fn read_errors_map_to_gaps() {
    assert_eq!(
        gap_for_error(ERROR_JOURNAL_ENTRY_DELETED),
        Some(GapReason::JournalEntryDeleted)
    );
    assert_eq!(
        gap_for_error(ERROR_JOURNAL_NOT_ACTIVE),
        Some(GapReason::JournalIdChanged)
    );
    assert_eq!(
        gap_for_error(ERROR_JOURNAL_DELETE_IN_PROGRESS),
        Some(GapReason::JournalIdChanged)
    );
    assert_eq!(gap_for_error(5), None);
    assert!(needs_synchronous_reads(1784));
    assert!(!needs_synchronous_reads(995));
}
