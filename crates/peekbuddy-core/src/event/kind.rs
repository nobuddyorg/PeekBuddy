//! The kind taxonomy.

/// Why a gap in the event stream exists (F2: every gap is a visible row).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum GapReason {
    /// RDCW buffer overflow (TRUE with 0 bytes).
    BufferOverflow,
    /// `ERROR_NOTIFY_ENUM_DIR`.
    NotifyEnumDir,
    /// USN journal entries were deleted before they were read.
    JournalEntryDeleted,
    /// The USN journal id changed.
    JournalIdChanged,
    /// A buffer failed to parse.
    ParserError,
    /// The engine was not running (startup gap).
    EngineNotRunning,
}

impl GapReason {
    /// Every reason, in wire order.
    pub const ALL: [Self; 6] = [
        Self::BufferOverflow,
        Self::NotifyEnumDir,
        Self::JournalEntryDeleted,
        Self::JournalIdChanged,
        Self::ParserError,
        Self::EngineNotRunning,
    ];

    const fn tag(self) -> u8 {
        self as u8
    }

    const fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            0 => Some(Self::BufferOverflow),
            1 => Some(Self::NotifyEnumDir),
            2 => Some(Self::JournalEntryDeleted),
            3 => Some(Self::JournalIdChanged),
            4 => Some(Self::ParserError),
            5 => Some(Self::EngineNotRunning),
            _ => None,
        }
    }
}

/// The watch lifecycle state (F19).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum WatchState {
    /// The watch is being armed.
    Arming,
    /// Events are flowing.
    Live,
    /// The buffer overflowed.
    Overflowed,
    /// A reconciliation scan is running.
    Reconciling,
    /// Paused by the user.
    SuspendedByUser,
    /// Suspended because the volume is being removed (query-remove).
    SuspendedQueryRemove,
    /// The watch was lost.
    Lost,
    /// The watch was restored.
    Restored,
}

impl WatchState {
    /// Every state, in wire order.
    pub const ALL: [Self; 8] = [
        Self::Arming,
        Self::Live,
        Self::Overflowed,
        Self::Reconciling,
        Self::SuspendedByUser,
        Self::SuspendedQueryRemove,
        Self::Lost,
        Self::Restored,
    ];

    const fn tag(self) -> u8 {
        self as u8
    }

    const fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            0 => Some(Self::Arming),
            1 => Some(Self::Live),
            2 => Some(Self::Overflowed),
            3 => Some(Self::Reconciling),
            4 => Some(Self::SuspendedByUser),
            5 => Some(Self::SuspendedQueryRemove),
            6 => Some(Self::Lost),
            7 => Some(Self::Restored),
            _ => None,
        }
    }
}

/// What happened, derived from the raw record (which stays in the event).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A file or directory appeared.
    Created,
    /// Data changed.
    Modified,
    /// A file or directory went away.
    Deleted,
    /// The old name of a rename.
    RenamedFrom,
    /// The new name of a rename.
    RenamedTo,
    /// Attributes, times, extended attributes and similar metadata changed.
    AttributesChanged,
    /// The security descriptor changed.
    SecurityChanged,
    /// A named stream changed.
    StreamChanged,
    /// A hard link or reparse point changed.
    LinkChanged,
    /// A gap in the stream.
    Overflow(GapReason),
    /// A watch changed state.
    WatchStateChanged(WatchState),
    /// The reconciler found a file that appeared during a gap.
    ReconciledCreated,
    /// The reconciler found a file that changed during a gap.
    ReconciledModified,
    /// The reconciler found a file that vanished during a gap.
    ReconciledDeleted,
    /// Not classifiable; the raw value is kept.
    Other,
    /// The engine started.
    EngineStarted,
    /// The engine stopped.
    EngineStopped,
    /// The store rotated to a new segment.
    SegmentRotated,
    /// Retention deleted old data (#54).
    RetentionPurged,
    /// Settings changed.
    SettingsChanged,
}

impl Kind {
    /// The wire representation: a tag and a payload byte.
    #[must_use]
    pub const fn to_wire(self) -> (u8, u8) {
        match self {
            Self::Created => (0, 0),
            Self::Modified => (1, 0),
            Self::Deleted => (2, 0),
            Self::RenamedFrom => (3, 0),
            Self::RenamedTo => (4, 0),
            Self::AttributesChanged => (5, 0),
            Self::SecurityChanged => (6, 0),
            Self::StreamChanged => (7, 0),
            Self::LinkChanged => (8, 0),
            Self::Overflow(r) => (9, r.tag()),
            Self::WatchStateChanged(s) => (10, s.tag()),
            Self::ReconciledCreated => (11, 0),
            Self::ReconciledModified => (12, 0),
            Self::ReconciledDeleted => (13, 0),
            Self::Other => (14, 0),
            Self::EngineStarted => (15, 0),
            Self::EngineStopped => (16, 0),
            Self::SegmentRotated => (17, 0),
            Self::RetentionPurged => (18, 0),
            Self::SettingsChanged => (19, 0),
        }
    }

    /// The kind for a wire tag and payload; `None` when not valid.
    #[must_use]
    pub const fn from_wire(tag: u8, arg: u8) -> Option<Self> {
        let plain = match tag {
            0 => Self::Created,
            1 => Self::Modified,
            2 => Self::Deleted,
            3 => Self::RenamedFrom,
            4 => Self::RenamedTo,
            5 => Self::AttributesChanged,
            6 => Self::SecurityChanged,
            7 => Self::StreamChanged,
            8 => Self::LinkChanged,
            9 => {
                return match GapReason::from_tag(arg) {
                    Some(r) => Some(Self::Overflow(r)),
                    None => None,
                };
            }
            10 => {
                return match WatchState::from_tag(arg) {
                    Some(s) => Some(Self::WatchStateChanged(s)),
                    None => None,
                };
            }
            11 => Self::ReconciledCreated,
            12 => Self::ReconciledModified,
            13 => Self::ReconciledDeleted,
            14 => Self::Other,
            15 => Self::EngineStarted,
            16 => Self::EngineStopped,
            17 => Self::SegmentRotated,
            18 => Self::RetentionPurged,
            19 => Self::SettingsChanged,
            _ => return None,
        };
        // Kinds without a payload must carry 0 so every value has one encoding.
        if arg == 0 { Some(plain) } else { None }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Every kind, including every payload.
    pub(crate) fn all_kinds() -> Vec<Kind> {
        let mut v = vec![
            Kind::Created,
            Kind::Modified,
            Kind::Deleted,
            Kind::RenamedFrom,
            Kind::RenamedTo,
            Kind::AttributesChanged,
            Kind::SecurityChanged,
            Kind::StreamChanged,
            Kind::LinkChanged,
            Kind::ReconciledCreated,
            Kind::ReconciledModified,
            Kind::ReconciledDeleted,
            Kind::Other,
            Kind::EngineStarted,
            Kind::EngineStopped,
            Kind::SegmentRotated,
            Kind::RetentionPurged,
            Kind::SettingsChanged,
        ];
        v.extend(GapReason::ALL.map(Kind::Overflow));
        v.extend(WatchState::ALL.map(Kind::WatchStateChanged));
        v
    }

    #[test]
    fn wire_round_trips_for_every_kind() {
        for k in all_kinds() {
            let (t, a) = k.to_wire();
            assert_eq!(Kind::from_wire(t, a), Some(k));
        }
    }

    #[test]
    fn wire_is_canonical_and_total() {
        let valid: Vec<Kind> = (0..=u8::MAX)
            .flat_map(|t| (0..=u8::MAX).map(move |a| (t, a)))
            .filter_map(|(t, a)| Kind::from_wire(t, a))
            .collect();
        assert_eq!(valid.len(), all_kinds().len());
    }
}
