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

fn saved(next_usn: i64) -> Checkpoint {
    Checkpoint {
        journal_id: 7,
        next_usn,
    }
}

#[test]
fn the_lowest_valid_usn_itself_still_resumes() {
    assert_eq!(
        plan_resume(Some(saved(1_000)), &NOW),
        Resume::Continue { from: 1_000 }
    );
    assert!(matches!(
        plan_resume(Some(saved(999)), &NOW),
        Resume::Gap { .. }
    ));
}

#[test]
fn the_first_usn_of_the_journal_is_the_other_lower_bound() {
    let now = JournalData {
        journal_id: 7,
        first_usn: 2_000,
        next_usn: 5_000,
        lowest_valid_usn: 1_000,
    };
    assert_eq!(
        plan_resume(Some(saved(2_000)), &now),
        Resume::Continue { from: 2_000 }
    );
    // Valid by the lowest bound, before the first entry: a gap.
    assert_eq!(
        plan_resume(Some(saved(1_999)), &now),
        Resume::Gap {
            reason: GapReason::JournalEntryDeleted,
            from: 5_000
        }
    );
    assert_eq!(
        plan_resume(Some(saved(1_500)), &now),
        Resume::Gap {
            reason: GapReason::JournalEntryDeleted,
            from: 5_000
        }
    );
}

#[test]
fn a_saved_usn_at_the_end_of_the_journal_resumes() {
    assert_eq!(
        plan_resume(Some(saved(5_000)), &NOW),
        Resume::Continue { from: 5_000 }
    );
    assert!(matches!(
        plan_resume(Some(saved(5_001)), &NOW),
        Resume::Gap { .. }
    ));
}
