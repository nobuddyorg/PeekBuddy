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
    /// The ring to the writer was full and records were dropped (#45).
    RingFull,
}

impl GapReason {
    /// Every reason, in wire order.
    pub const ALL: [Self; 7] = [
        Self::BufferOverflow,
        Self::NotifyEnumDir,
        Self::JournalEntryDeleted,
        Self::JournalIdChanged,
        Self::ParserError,
        Self::EngineNotRunning,
        Self::RingFull,
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
            6 => Some(Self::RingFull),
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
    /// Suspended because the volume is locked, dismounted or being checked
    /// (`BitLocker`, `chkdsk`, `FSCTL_LOCK_VOLUME`).
    SuspendedLocked,
}

impl WatchState {
    /// Every state, in wire order.
    pub const ALL: [Self; 9] = [
        Self::Arming,
        Self::Live,
        Self::Overflowed,
        Self::Reconciling,
        Self::SuspendedByUser,
        Self::SuspendedQueryRemove,
        Self::Lost,
        Self::Restored,
        Self::SuspendedLocked,
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
            8 => Some(Self::SuspendedLocked),
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
    /// The reconciler found a file under a new name or place (same file id).
    /// The row carries the new path.
    ReconciledRenamed,
    /// A reconcile of a root started (#46). `raw_mask` is 1 + the wire tag of
    /// the [`GapReason`] that asked for it, 0 when none did.
    ReconcileStarted,
    /// A reconcile finished: `size` is the entries scanned, `file_id` the
    /// rows it wrote, `src_ts` its duration in 100 ns ticks. A cancelled run
    /// has `raw_mask` 1.
    ReconcileFinished,
    /// The first scan of a root wrote its baseline: `size` is the files and
    /// `file_id` the directories found. No row per file is written.
    BaselineEstablished,
    /// A directory could not be listed (access denied); one row per
    /// directory per run.
    AccessDenied,
    /// The previous run of the engine ended abnormally (a crash marker was
    /// found at start). The row's name is the crash log.
    EngineAbnormalExit,
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
            Self::ReconciledRenamed => (20, 0),
            Self::ReconcileStarted => (21, 0),
            Self::ReconcileFinished => (22, 0),
            Self::BaselineEstablished => (23, 0),
            Self::AccessDenied => (24, 0),
            Self::EngineAbnormalExit => (25, 0),
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
            20 => Self::ReconciledRenamed,
            21 => Self::ReconcileStarted,
            22 => Self::ReconcileFinished,
            23 => Self::BaselineEstablished,
            24 => Self::AccessDenied,
            25 => Self::EngineAbnormalExit,
            _ => return None,
        };
        // Kinds without a payload must carry 0 so every value has one encoding.
        if arg == 0 { Some(plain) } else { None }
    }
}

impl GapReason {
    /// The stable name used in exports.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::BufferOverflow => "buffer_overflow",
            Self::NotifyEnumDir => "notify_enum_dir",
            Self::JournalEntryDeleted => "journal_entry_deleted",
            Self::JournalIdChanged => "journal_id_changed",
            Self::ParserError => "parser_error",
            Self::EngineNotRunning => "engine_not_running",
            Self::RingFull => "ring_full",
        }
    }
}

impl WatchState {
    /// The stable name used in exports.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Arming => "arming",
            Self::Live => "live",
            Self::Overflowed => "overflowed",
            Self::Reconciling => "reconciling",
            Self::SuspendedByUser => "suspended_by_user",
            Self::SuspendedQueryRemove => "suspended_query_remove",
            Self::Lost => "lost",
            Self::Restored => "restored",
            Self::SuspendedLocked => "suspended_locked",
        }
    }
}

impl Kind {
    /// The stable name used in exports: `created`, `renamed_to`,
    /// `overflow:ring_full`, `watch_state:live` and so on. Names are part of
    /// the export format; they are never renamed.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Modified => "modified",
            Self::Deleted => "deleted",
            Self::RenamedFrom => "renamed_from",
            Self::RenamedTo => "renamed_to",
            Self::AttributesChanged => "attributes_changed",
            Self::SecurityChanged => "security_changed",
            Self::StreamChanged => "stream_changed",
            Self::LinkChanged => "link_changed",
            Self::Overflow(GapReason::BufferOverflow) => "overflow:buffer_overflow",
            Self::Overflow(GapReason::NotifyEnumDir) => "overflow:notify_enum_dir",
            Self::Overflow(GapReason::JournalEntryDeleted) => "overflow:journal_entry_deleted",
            Self::Overflow(GapReason::JournalIdChanged) => "overflow:journal_id_changed",
            Self::Overflow(GapReason::ParserError) => "overflow:parser_error",
            Self::Overflow(GapReason::EngineNotRunning) => "overflow:engine_not_running",
            Self::Overflow(GapReason::RingFull) => "overflow:ring_full",
            Self::WatchStateChanged(WatchState::Arming) => "watch_state:arming",
            Self::WatchStateChanged(WatchState::Live) => "watch_state:live",
            Self::WatchStateChanged(WatchState::Overflowed) => "watch_state:overflowed",
            Self::WatchStateChanged(WatchState::Reconciling) => "watch_state:reconciling",
            Self::WatchStateChanged(WatchState::SuspendedByUser) => "watch_state:suspended_by_user",
            Self::WatchStateChanged(WatchState::SuspendedQueryRemove) => {
                "watch_state:suspended_query_remove"
            }
            Self::WatchStateChanged(WatchState::Lost) => "watch_state:lost",
            Self::WatchStateChanged(WatchState::Restored) => "watch_state:restored",
            Self::WatchStateChanged(WatchState::SuspendedLocked) => "watch_state:suspended_locked",
            Self::ReconciledCreated => "reconciled_created",
            Self::ReconciledModified => "reconciled_modified",
            Self::ReconciledDeleted => "reconciled_deleted",
            Self::ReconciledRenamed => "reconciled_renamed",
            Self::ReconcileStarted => "reconcile_started",
            Self::ReconcileFinished => "reconcile_finished",
            Self::BaselineEstablished => "baseline_established",
            Self::AccessDenied => "access_denied",
            Self::EngineAbnormalExit => "engine_abnormal_exit",
            Self::Other => "other",
            Self::EngineStarted => "engine_started",
            Self::EngineStopped => "engine_stopped",
            Self::SegmentRotated => "segment_rotated",
            Self::RetentionPurged => "retention_purged",
            Self::SettingsChanged => "settings_changed",
        }
    }

    /// The kind with this export name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        (0..=u8::MAX)
            .take(32)
            .flat_map(|tag| (0..=8).map(move |arg| (tag, arg)))
            .filter_map(|(tag, arg)| Self::from_wire(tag, arg))
            .find(|kind| kind.name() == name)
    }

    /// Whether the row is a marker of a gap in the stream.
    #[must_use]
    pub const fn is_gap(self) -> bool {
        matches!(self, Self::Overflow(_))
    }

    /// Whether the row comes from the reconciler.
    #[must_use]
    pub const fn is_reconcile(self) -> bool {
        matches!(
            self,
            Self::ReconciledCreated
                | Self::ReconciledModified
                | Self::ReconciledDeleted
                | Self::ReconciledRenamed
                | Self::ReconcileStarted
                | Self::ReconcileFinished
                | Self::BaselineEstablished
        )
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
            Kind::ReconciledRenamed,
            Kind::ReconcileStarted,
            Kind::ReconcileFinished,
            Kind::BaselineEstablished,
            Kind::AccessDenied,
            Kind::EngineAbnormalExit,
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
    #[test]
    fn names_are_unique_and_round_trip() {
        let mut names: Vec<&str> = all_kinds().iter().map(|k| k.name()).collect();
        for kind in all_kinds() {
            assert_eq!(Kind::from_name(kind.name()), Some(kind));
        }
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), all_kinds().len());
        assert_eq!(Kind::from_name("nope"), None);
    }

    #[test]
    fn gap_and_reconcile_markers() {
        assert!(Kind::Overflow(GapReason::RingFull).is_gap());
        assert!(!Kind::Created.is_gap());
        assert!(Kind::ReconciledRenamed.is_reconcile());
        assert!(!Kind::Modified.is_reconcile());
    }
}
