//! The source-agnostic event model (#38).
//!
//! Every layer above capture speaks in terms of [`Event`]: a fixed-size,
//! `Copy` record with no heap pointers, so the capture path never allocates
//! (F10). The raw OS value stays in [`Event::raw_mask`] (raw is truth, F3);
//! [`Kind`] is a derived classification. Names live in a per-batch
//! [`NameArena`] and are referenced by [`NameRef`] (name semantics are #39).

pub mod map;
pub mod wire;

mod arena;
mod kind;

pub use arena::{MAX_NAME_UNITS, NameArena, NameRef};
pub use kind::{GapReason, Kind, WatchState};

/// Where an event came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Source {
    /// `ReadDirectoryChangesExW`.
    Rdcw,
    /// The USN change journal.
    Usn,
    /// The reconciliation scanner.
    Reconcile,
    /// PeekBuddy itself (engine lifecycle, gaps, watch state).
    System,
}

impl Source {
    /// Every source, in wire-tag order.
    pub const ALL: [Self; 4] = [Self::Rdcw, Self::Usn, Self::Reconcile, Self::System];

    /// The stable wire tag.
    #[must_use]
    pub const fn tag(self) -> u8 {
        self as u8
    }

    /// The source for a wire tag, if it is one.
    #[must_use]
    pub const fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            0 => Some(Self::Rdcw),
            1 => Some(Self::Usn),
            2 => Some(Self::Reconcile),
            3 => Some(Self::System),
            _ => None,
        }
    }
}

/// How to read the name an event refers to (refined by #39).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum NameKind {
    /// Path relative to the watch root (RDCW).
    Relative,
    /// A single file name without a directory part (USN).
    Leaf,
    /// An 8.3 short name.
    Short,
    /// The name could not be resolved; the name reference is empty.
    Unresolved,
}

impl NameKind {
    /// Every name kind, in wire-tag order.
    pub const ALL: [Self; 4] = [Self::Relative, Self::Leaf, Self::Short, Self::Unresolved];

    /// The stable wire tag.
    #[must_use]
    pub const fn tag(self) -> u8 {
        self as u8
    }

    /// The name kind for a wire tag, if it is one.
    #[must_use]
    pub const fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            0 => Some(Self::Relative),
            1 => Some(Self::Leaf),
            2 => Some(Self::Short),
            3 => Some(Self::Unresolved),
            _ => None,
        }
    }
}

/// Index of a volume in the engine's volume table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VolIdx(pub u16);

/// Index of a watch root in the settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RootIdx(pub u16);

/// Per-event flag bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Flags(u16);

impl Flags {
    /// No flags set.
    pub const NONE: Self = Self(0);
    /// Cloud-provider hydration or dehydration churn.
    pub const CLOUD_CHURN: Self = Self(1);
    /// Caused by PeekBuddy's own data directory.
    pub const SELF_CHURN: Self = Self(1 << 1);
    /// A temporary file.
    pub const TEMP: Self = Self(1 << 2);
    /// The 8.3 short-name duplicate of another event.
    pub const SHORT_DUP: Self = Self(1 << 3);
    /// The name could not be resolved.
    pub const NAME_UNRESOLVED: Self = Self(1 << 4);
    /// A gap row follows this event.
    pub const GAP_FOLLOWS: Self = Self(1 << 5);

    const ALL_BITS: u16 = 0b11_1111;

    /// The raw bits.
    #[must_use]
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// Flags from raw bits; `None` when an undefined bit is set.
    #[must_use]
    pub const fn from_bits(bits: u16) -> Option<Self> {
        if bits & !Self::ALL_BITS == 0 {
            Some(Self(bits))
        } else {
            None
        }
    }

    /// Whether every bit of `other` is set.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// The union of both sets.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// One file-system (or system) event as the capture path produces it.
///
/// `Copy`, fixed size, no heap pointers. The store assigns the event id
/// (#52); the read side uses [`StoredEvent`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Event {
    /// Capture wall time, FILETIME 100 ns ticks.
    pub ts: u64,
    /// Monotonic QPC ticks: in-process ordering and latency only.
    pub qpc: u64,
    /// The source's own timestamp (USN `TimeStamp`); 0 = none.
    pub src_ts: u64,
    /// The originating source.
    pub source: Source,
    /// The derived classification.
    pub kind: Kind,
    /// `FILE_ACTION_*` value or `USN_REASON_*` mask, unmodified.
    pub raw_mask: u32,
    /// USN `SourceInfo`; 0 for RDCW.
    pub source_info: u32,
    /// Whether the subject is a directory.
    pub is_dir: bool,
    /// File size in bytes, when known.
    pub size: u64,
    /// File attributes.
    pub attrs: u32,
    /// Reparse tag, 0 when not a reparse point.
    pub reparse_tag: u32,
    /// File id, 64-bit ids widened; `ReFS` ids are native 128-bit.
    pub file_id: u128,
    /// Parent directory file id, widened the same way.
    pub parent_file_id: u128,
    /// USN of the record; 0 when not USN.
    pub usn: i64,
    /// Volume index.
    pub vol: VolIdx,
    /// Watch root index.
    pub root: RootIdx,
    /// The name, as a reference into a [`NameArena`].
    pub name: NameRef,
    /// How to read the name.
    pub name_kind: NameKind,
    /// Flag bits.
    pub flags: Flags,
}

/// An event with the id the store assigned to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoredEvent {
    /// Store-assigned id.
    pub id: u64,
    /// The event.
    pub ev: Event,
}

const _: () = assert!(size_of::<Event>() <= 128);

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A fully populated event for tests.
    pub(crate) fn sample() -> Event {
        Event {
            ts: 133_000_000_000_000_000,
            qpc: 42,
            src_ts: 7,
            source: Source::Usn,
            kind: Kind::Modified,
            raw_mask: 0x8000_0002,
            source_info: 1,
            is_dir: false,
            size: 4096,
            attrs: 0x20,
            reparse_tag: 0,
            file_id: u128::MAX - 3,
            parent_file_id: 5,
            usn: -9,
            vol: VolIdx(1),
            root: RootIdx(2),
            name: NameRef::EMPTY,
            name_kind: NameKind::Leaf,
            flags: Flags::TEMP.union(Flags::GAP_FOLLOWS),
        }
    }

    #[test]
    fn event_is_copy_without_drop() {
        const fn is_copy<T: Copy>() {}
        is_copy::<Event>();
        assert!(!std::mem::needs_drop::<Event>());
    }

    #[test]
    fn flag_bits_are_stable() {
        let all = [
            (Flags::CLOUD_CHURN, 0x01),
            (Flags::SELF_CHURN, 0x02),
            (Flags::TEMP, 0x04),
            (Flags::SHORT_DUP, 0x08),
            (Flags::NAME_UNRESOLVED, 0x10),
            (Flags::GAP_FOLLOWS, 0x20),
        ];
        for (flag, bits) in all {
            assert_eq!(flag.bits(), bits);
        }
        let both = Flags::TEMP.union(Flags::TEMP);
        assert_eq!(both, Flags::TEMP);
        assert_eq!(Flags::TEMP.union(Flags::SELF_CHURN).bits(), 0x06);
    }

    #[test]
    fn flags_reject_undefined_bits() {
        assert_eq!(Flags::from_bits(0), Some(Flags::NONE));
        assert!(Flags::from_bits(Flags::ALL_BITS).is_some());
        assert_eq!(Flags::from_bits(1 << 6), None);
        assert!(Flags::TEMP.union(Flags::CLOUD_CHURN).contains(Flags::TEMP));
        assert!(!Flags::TEMP.contains(Flags::CLOUD_CHURN));
    }

    #[test]
    fn tags_round_trip() {
        for s in Source::ALL {
            assert_eq!(Source::from_tag(s.tag()), Some(s));
        }
        for n in NameKind::ALL {
            assert_eq!(NameKind::from_tag(n.tag()), Some(n));
        }
        assert_eq!(Source::from_tag(4), None);
        assert_eq!(NameKind::from_tag(4), None);
    }

    #[test]
    fn size_is_reported() {
        // PERFORMANCE.md records this number; keep both in step.
        assert_eq!(size_of::<Event>(), 112);
    }
}
