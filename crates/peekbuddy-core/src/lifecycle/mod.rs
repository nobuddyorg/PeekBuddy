//! The watch lifecycle state machine (#48).
//!
//! A watch is never dead without that being logged. [`Machine`] is one watch
//! root: typed [`Signal`]s go in (an I/O completion, a plug-and-play or power message, a
//! pause, the end of a reconcile), and out come at most one [`Transition`]
//! (which becomes exactly one `WatchStateChanged` row) and some
//! [`Action`]s for the Windows side to carry out (close the handle, reopen,
//! reconcile, verify, watch the parent directory). It is pure and total: any
//! signal in any order is accepted, and one that makes no sense in the current
//! state is ignored without a transition.
//!
//! # States
//!
//! `Arming` -> `Live`; a gap makes it `Overflowed` until the reconcile starts
//! (`Reconciling`) and ends (`Live`); `Suspended` by the user (handle stays
//! armed, see ADR 0016), for a volume query-remove (handle closed) or for a
//! lock (handle closed); `Lost` with a [`Reason`]; `Restored` after a lost
//! root came back, until its reconcile has run.
//!
//! # Visible states
//!
//! Every state maps to a [`WatchStateCode`] the
//! viewer shows ([`State::ipc_code`]); no state that can last is invisible.
//!
//! # Rows
//!
//! [`Transition::to_event`] writes the row: kind `WatchStateChanged(to)`,
//! `raw_mask` = the old state's wire byte in bits 16..24 and the [`Reason`]
//! code in the low 16 bits, `size` = events missed while paused for a resume.

use crate::event::{
    Event, Flags, GapReason, Kind, NameKind, NameRef, RootIdx, Source, VolIdx, WatchState,
};
use crate::ipc::WatchStateCode;

/// Why a watch is suspended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// The user paused it.
    User,
    /// The volume is about to be removed.
    QueryRemove,
    /// The volume is locked, dismounted or being checked.
    Locked,
}

/// The state of one watch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Being armed.
    Arming,
    /// Events are flowing.
    Live,
    /// A gap happened; a reconcile is wanted.
    Overflowed,
    /// A reconcile is running.
    Reconciling,
    /// Suspended.
    Suspended(Why),
    /// The watch is lost.
    Lost(Reason),
    /// A lost root came back; its reconcile is wanted.
    Restored,
}

impl State {
    /// The state as the event model names it.
    #[must_use]
    pub const fn watch_state(self) -> WatchState {
        match self {
            Self::Arming => WatchState::Arming,
            Self::Live => WatchState::Live,
            Self::Overflowed => WatchState::Overflowed,
            Self::Reconciling => WatchState::Reconciling,
            Self::Suspended(Why::User) => WatchState::SuspendedByUser,
            Self::Suspended(Why::QueryRemove) => WatchState::SuspendedQueryRemove,
            Self::Suspended(Why::Locked) => WatchState::SuspendedLocked,
            Self::Lost(_) => WatchState::Lost,
            Self::Restored => WatchState::Restored,
        }
    }

    /// What the viewer shows.
    #[must_use]
    pub const fn ipc_code(self) -> WatchStateCode {
        match self {
            Self::Arming | Self::Live => WatchStateCode::Live,
            Self::Overflowed | Self::Reconciling | Self::Restored => WatchStateCode::Gap,
            Self::Suspended(_) => WatchStateCode::Paused,
            Self::Lost(_) => WatchStateCode::Lost,
        }
    }

    /// Whether the watch has an open handle in this state.
    #[must_use]
    pub const fn handle_open(self) -> bool {
        matches!(
            self,
            Self::Arming
                | Self::Live
                | Self::Overflowed
                | Self::Reconciling
                | Self::Restored
                | Self::Suspended(Why::User)
        )
    }
}

/// Why a transition happened. The codes are on disk: append only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum Reason {
    /// The watch was armed.
    Armed = 1,
    /// Arming failed.
    ArmFailed = 2,
    /// A buffer overflow.
    Overflow = 3,
    /// The ring to the writer was full.
    RingFull = 4,
    /// A buffer failed to parse.
    ParseError = 5,
    /// A reconcile started.
    ReconcileStarted = 6,
    /// A reconcile finished.
    ReconcileDone = 7,
    /// A reconcile was cancelled.
    ReconcileCancelled = 8,
    /// The user paused the watch.
    UserPause = 9,
    /// The user resumed the watch.
    UserResume = 10,
    /// The volume asked whether it may be removed.
    QueryRemove = 11,
    /// The removal was refused or cancelled.
    QueryRemoveFailed = 12,
    /// The device is being or was removed.
    DeviceRemoved = 13,
    /// The volume was locked.
    VolumeLocked = 14,
    /// The volume was dismounted.
    VolumeDismounted = 15,
    /// The volume came back (mounted, unlocked, arrived).
    VolumeBack = 16,
    /// The watched root was deleted.
    RootDeleted = 17,
    /// The watched root was renamed or moved.
    RootMoved = 18,
    /// The watched root exists again.
    RootRestored = 19,
    /// A probe found the device not ready.
    DeviceNotReady = 20,
    /// A probe found the device or volume gone.
    DeviceGone = 21,
    /// A probe found everything in order after an unexpected error.
    VerifiedOk = 22,
}

impl Reason {
    /// The code stored in the row.
    #[must_use]
    pub const fn code(self) -> u16 {
        self as u16
    }
}

/// How a probe of the volume ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Mounted and ready.
    Ok,
    /// Present but not ready (`ERROR_NOT_READY`, locked).
    NotReady,
    /// Gone (`ERROR_DEVICE_NOT_CONNECTED`, not mounted).
    Gone,
}

/// An input to a watch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// The watch was armed.
    Armed,
    /// Arming failed.
    ArmFailed,
    /// A gap in the stream.
    Gap(GapReason),
    /// An unexpected completion status: the volume must be probed.
    IoError,
    /// The answer of a probe.
    Verified(Verdict),
    /// The watched root was deleted (`ERROR_ACCESS_DENIED`, 745).
    RootDeleted,
    /// The watched root was renamed or moved away.
    RootMoved,
    /// The watched root exists again (seen by the parent watch).
    RootRecreated,
    /// A reconcile of the root started.
    ReconcileStarted,
    /// A reconcile finished.
    ReconcileDone,
    /// A reconcile was cancelled.
    ReconcileCancelled,
    /// The user paused the watch.
    UserPause,
    /// The user resumed the watch; `missed` events were discarded while it
    /// was paused (`lower_bound`: at least that many).
    UserResume {
        /// Events discarded.
        missed: u64,
        /// An overflow happened while paused.
        lower_bound: bool,
    },
    /// `CM_NOTIFY_ACTION_DEVICEQUERYREMOVE`.
    QueryRemove,
    /// `CM_NOTIFY_ACTION_DEVICEQUERYREMOVEFAILED`.
    QueryRemoveFailed,
    /// `CM_NOTIFY_ACTION_DEVICEREMOVEPENDING`.
    RemovePending,
    /// `CM_NOTIFY_ACTION_DEVICEREMOVECOMPLETE`.
    RemoveComplete,
    /// The volume was locked (`GUID_IO_VOLUME_LOCK`, `BitLocker`, `chkdsk`).
    VolumeLock,
    /// The volume was dismounted.
    Dismount,
    /// A lock or dismount failed (`GUID_IO_VOLUME_LOCK_FAILED`,
    /// `GUID_IO_VOLUME_DISMOUNT_FAILED`).
    LockFailed,
    /// The volume was mounted or unlocked, or arrived.
    VolumeBack,
    /// The system is about to sleep.
    PowerSuspend,
    /// The system woke up.
    PowerResume,
}

/// Something for the Windows side to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Cancel the pending read and close the handle.
    CloseHandle,
    /// Close if needed and open and arm the root again.
    Reopen,
    /// Walk the root (#46).
    Reconcile(Option<GapReason>),
    /// Probe the volume and report [`Signal::Verified`].
    Verify,
    /// Watch the parent directory for the root coming back.
    WatchParent,
    /// Stop watching the parent directory.
    UnwatchParent,
    /// Flush the ring and commit now (the machine is going to sleep).
    FlushNow,
}

/// A state change; it is one row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transition {
    /// The state left.
    pub from: State,
    /// The state entered.
    pub to: State,
    /// Why.
    pub reason: Reason,
    /// Events missed while paused (a resume).
    pub missed: u64,
    /// The missed count is a lower bound.
    pub lower_bound: bool,
}

impl Transition {
    /// The row for this change.
    #[must_use]
    pub fn to_event(&self, root: RootIdx, vol: VolIdx, ts: u64, qpc: u64) -> Event {
        let from_byte = self.from.watch_state_byte();
        Event {
            ts,
            qpc,
            src_ts: 0,
            source: Source::System,
            kind: Kind::WatchStateChanged(self.to.watch_state()),
            // Disjoint bits (the reason code is a byte): `+` equals `|` and has no
            // equivalent `^` mutant.
            raw_mask: (u32::from(from_byte) << 16) + u32::from(self.reason.code()),
            source_info: 0,
            is_dir: false,
            size: self.missed,
            attrs: 0,
            reparse_tag: 0,
            file_id: 0,
            parent_file_id: 0,
            usn: 0,
            vol,
            root,
            name: NameRef::EMPTY,
            name_kind: NameKind::Unknown,
            flags: if self.lower_bound {
                Flags::GAP_FOLLOWS
            } else {
                Flags::NONE
            },
        }
    }
}

impl State {
    /// The wire byte of the state's [`WatchState`].
    fn watch_state_byte(self) -> u8 {
        Kind::WatchStateChanged(self.watch_state()).to_wire().1
    }
}

/// What a signal did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Outcome {
    /// The state change, if there was one.
    pub transition: Option<Transition>,
    actions: [Option<Action>; 4],
}

impl Outcome {
    /// The actions, in the order to carry them out.
    pub fn actions(&self) -> impl Iterator<Item = Action> + '_ {
        self.actions.iter().flatten().copied()
    }

    fn push(&mut self, action: Action) {
        if let Some(slot) = self.actions.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some(action);
        }
    }
}

/// One watch root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Machine {
    state: State,
    /// The user wants the watch paused, whatever else happens to the volume.
    user_paused: bool,
    /// A gap happened while the watch could not reconcile yet.
    reconcile_wanted: bool,
}

impl Default for Machine {
    fn default() -> Self {
        Self::new()
    }
}

impl Machine {
    /// A watch that is being armed.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: State::Arming,
            user_paused: false,
            // A watch that is paused before its first reconcile owes one.
            reconcile_wanted: true,
        }
    }

    /// The current state.
    #[must_use]
    pub const fn state(&self) -> State {
        self.state
    }

    /// Whether the user wants the watch paused.
    #[must_use]
    pub const fn user_paused(&self) -> bool {
        self.user_paused
    }

    fn go(&mut self, to: State, reason: Reason, out: &mut Outcome) {
        self.go_with(to, reason, 0, false, out);
    }

    fn go_with(&mut self, to: State, reason: Reason, missed: u64, lower: bool, out: &mut Outcome) {
        if to == self.state {
            return;
        }
        out.transition = Some(Transition {
            from: self.state,
            to,
            reason,
            missed,
            lower_bound: lower,
        });
        self.state = to;
    }

    /// Where a watch goes once the handle is usable again (after an
    /// interruption), with the actions that get it there.
    fn resume_after_interruption(&mut self, reason: Reason, out: &mut Outcome) {
        out.push(Action::Reopen);
        if self.user_paused {
            // Back to the user's pause; the reconcile waits for the resume.
            self.reconcile_wanted = true;
            self.go(State::Suspended(Why::User), reason, out);
        } else {
            self.reconcile_wanted = false;
            out.push(Action::Reconcile(None));
            self.go(State::Restored, reason, out);
        }
    }

    fn lose(&mut self, reason: Reason, out: &mut Outcome) {
        if self.state.handle_open() {
            out.push(Action::CloseHandle);
        }
        self.go(State::Lost(reason), reason, out);
    }

    /// Feeds one signal.
    #[must_use]
    #[expect(
        clippy::too_many_lines,
        reason = "the transition table reads best as one match"
    )]
    pub fn signal(&mut self, signal: Signal) -> Outcome {
        use State::{Arming, Live, Lost, Overflowed, Reconciling, Restored, Suspended};

        let mut out = Outcome::default();
        let state = self.state;
        let running = matches!(state, Arming | Live | Overflowed | Reconciling | Restored);
        match signal {
            Signal::Armed => {
                if state == Arming {
                    // The engine start: the first thing a watch does is
                    // compare the tree with what was known (#46). The time
                    // the engine was not running is a gap, and it is written
                    // as one (F2); a first run has no snapshot, so the walk
                    // writes nothing then.
                    self.reconcile_wanted = false;
                    out.push(Action::Reconcile(Some(GapReason::EngineNotRunning)));
                    self.go(Live, Reason::Armed, &mut out);
                }
            }
            Signal::ArmFailed => {
                if matches!(state, Arming | Restored | Suspended(Why::User)) {
                    self.lose(Reason::ArmFailed, &mut out);
                    out.push(Action::WatchParent);
                }
            }
            Signal::Gap(gap) => {
                let reason = match gap {
                    GapReason::RingFull => Reason::RingFull,
                    GapReason::ParserError => Reason::ParseError,
                    _ => Reason::Overflow,
                };
                match state {
                    Live | Reconciling | Restored => {
                        self.go(Overflowed, reason, &mut out);
                        out.push(Action::Reconcile(Some(gap)));
                    }
                    Suspended(Why::User) => self.reconcile_wanted = true,
                    // Already waiting for a reconcile, or the handle is
                    // closed and the reopen reconciles anyway.
                    _ => {}
                }
            }
            Signal::IoError => {
                if running {
                    out.push(Action::Verify);
                }
            }
            Signal::Verified(Verdict::Ok) => match state {
                Arming | Live | Overflowed | Reconciling | Restored => {
                    out.push(Action::Reopen);
                    out.push(Action::Reconcile(None));
                    self.go(Overflowed, Reason::VerifiedOk, &mut out);
                }
                Lost(
                    Reason::DeviceNotReady
                    | Reason::DeviceGone
                    | Reason::DeviceRemoved
                    | Reason::VolumeDismounted,
                ) => self.resume_after_interruption(Reason::VolumeBack, &mut out),
                _ => {}
            },
            Signal::Verified(verdict) => {
                if running {
                    let reason = if verdict == Verdict::NotReady {
                        Reason::DeviceNotReady
                    } else {
                        Reason::DeviceGone
                    };
                    self.lose(reason, &mut out);
                }
            }
            Signal::RootDeleted | Signal::RootMoved => {
                if running || state == Suspended(Why::User) {
                    let reason = if signal == Signal::RootDeleted {
                        Reason::RootDeleted
                    } else {
                        Reason::RootMoved
                    };
                    self.lose(reason, &mut out);
                    out.push(Action::WatchParent);
                }
            }
            Signal::RootRecreated => {
                if matches!(
                    state,
                    Lost(Reason::RootDeleted | Reason::RootMoved | Reason::ArmFailed)
                ) {
                    out.push(Action::UnwatchParent);
                    self.resume_after_interruption(Reason::RootRestored, &mut out);
                }
            }
            Signal::ReconcileStarted => {
                if matches!(state, Overflowed | Restored) {
                    self.go(Reconciling, Reason::ReconcileStarted, &mut out);
                }
            }
            Signal::ReconcileDone => {
                if matches!(state, Overflowed | Reconciling | Restored) {
                    self.go(Live, Reason::ReconcileDone, &mut out);
                }
            }
            Signal::ReconcileCancelled => {
                if state == Reconciling {
                    out.push(Action::Reconcile(None));
                    self.go(Overflowed, Reason::ReconcileCancelled, &mut out);
                }
            }
            Signal::UserPause => {
                self.user_paused = true;
                if running {
                    self.go(Suspended(Why::User), Reason::UserPause, &mut out);
                }
            }
            Signal::UserResume {
                missed,
                lower_bound,
            } => {
                self.user_paused = false;
                if state == Suspended(Why::User) {
                    if self.reconcile_wanted {
                        self.reconcile_wanted = false;
                        out.push(Action::Reconcile(None));
                        self.go_with(Overflowed, Reason::UserResume, missed, true, &mut out);
                    } else {
                        self.go_with(Live, Reason::UserResume, missed, lower_bound, &mut out);
                    }
                }
            }
            Signal::QueryRemove | Signal::VolumeLock | Signal::Dismount => {
                let (why, reason) = match signal {
                    Signal::QueryRemove => (Why::QueryRemove, Reason::QueryRemove),
                    Signal::VolumeLock => (Why::Locked, Reason::VolumeLocked),
                    _ => (Why::Locked, Reason::VolumeDismounted),
                };
                if (running || state == Suspended(Why::User)) && state != Suspended(why) {
                    out.push(Action::CloseHandle);
                    self.go(Suspended(why), reason, &mut out);
                } else if matches!(state, Suspended(Why::Locked | Why::QueryRemove))
                    && state != Suspended(why)
                {
                    // Already closed; the later, stronger reason wins.
                    self.go(Suspended(why), reason, &mut out);
                }
            }
            Signal::QueryRemoveFailed | Signal::LockFailed | Signal::VolumeBack => match state {
                Suspended(Why::QueryRemove) if signal == Signal::QueryRemoveFailed => {
                    self.resume_after_interruption(Reason::QueryRemoveFailed, &mut out);
                }
                Suspended(Why::Locked)
                    if matches!(signal, Signal::LockFailed | Signal::VolumeBack) =>
                {
                    self.resume_after_interruption(Reason::VolumeBack, &mut out);
                }
                Suspended(Why::QueryRemove)
                | Lost(Reason::DeviceRemoved | Reason::DeviceGone | Reason::VolumeDismounted)
                    if signal == Signal::VolumeBack =>
                {
                    self.resume_after_interruption(Reason::VolumeBack, &mut out);
                }
                _ => {}
            },
            Signal::RemovePending | Signal::RemoveComplete => {
                if running || matches!(state, Suspended(_)) {
                    self.lose(Reason::DeviceRemoved, &mut out);
                }
            }
            Signal::PowerSuspend => out.push(Action::FlushNow),
            Signal::PowerResume => {
                if running || matches!(state, Lost(Reason::DeviceNotReady | Reason::DeviceGone)) {
                    out.push(Action::Verify);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests;
