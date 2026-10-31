use proptest::prelude::*;
use proptest_state_machine::{ReferenceStateMachine, StateMachineTest, prop_state_machine};

use super::*;

fn feed(machine: &mut Machine, signals: &[Signal]) -> Vec<Outcome> {
    signals.iter().map(|&s| machine.signal(s)).collect()
}

fn states(machine: &mut Machine, signals: &[Signal]) -> Vec<WatchState> {
    feed(machine, signals)
        .into_iter()
        .filter_map(|o| o.transition.map(|t| t.to.watch_state()))
        .collect()
}

fn actions(out: &Outcome) -> Vec<Action> {
    out.actions().collect()
}

fn live() -> Machine {
    let mut m = Machine::new();
    let _ = m.signal(Signal::Armed);
    m
}

#[test]
fn arming_then_live() {
    let mut m = Machine::new();
    assert_eq!(m.state(), State::Arming);
    let out = m.signal(Signal::Armed);
    assert_eq!(out.transition.unwrap().reason, Reason::Armed);
    assert_eq!(m.state(), State::Live);
    // Armed again is not a transition.
    assert_eq!(m.signal(Signal::Armed), Outcome::default());
}

#[test]
fn a_gap_is_overflowed_then_reconciling_then_live() {
    let mut m = live();
    let out = m.signal(Signal::Gap(GapReason::BufferOverflow));
    assert_eq!(m.state(), State::Overflowed);
    assert_eq!(
        actions(&out),
        [Action::Reconcile(Some(GapReason::BufferOverflow))]
    );
    // A second gap while waiting is the same wait, not a second row.
    assert_eq!(
        m.signal(Signal::Gap(GapReason::RingFull)),
        Outcome::default()
    );
    assert_eq!(
        states(&mut m, &[Signal::ReconcileStarted, Signal::ReconcileDone]),
        [WatchState::Reconciling, WatchState::Live]
    );
    // A gap during the reconcile starts over.
    let mut m = live();
    let _ = feed(
        &mut m,
        &[
            Signal::Gap(GapReason::BufferOverflow),
            Signal::ReconcileStarted,
        ],
    );
    let out = m.signal(Signal::Gap(GapReason::NotifyEnumDir));
    assert_eq!(m.state(), State::Overflowed);
    assert_eq!(actions(&out).len(), 1);
    // A cancelled reconcile asks again.
    let mut m = live();
    let _ = feed(
        &mut m,
        &[Signal::Gap(GapReason::RingFull), Signal::ReconcileStarted],
    );
    let out = m.signal(Signal::ReconcileCancelled);
    assert_eq!(m.state(), State::Overflowed);
    assert_eq!(actions(&out), [Action::Reconcile(None)]);
}

#[test]
fn a_deleted_root_is_lost_and_comes_back_restored_with_a_reconcile() {
    let mut m = live();
    let out = m.signal(Signal::RootDeleted);
    assert_eq!(m.state(), State::Lost(Reason::RootDeleted));
    assert_eq!(actions(&out), [Action::CloseHandle, Action::WatchParent]);

    let out = m.signal(Signal::RootRecreated);
    assert_eq!(m.state(), State::Restored);
    assert_eq!(
        actions(&out),
        [
            Action::UnwatchParent,
            Action::Reopen,
            Action::Reconcile(None)
        ]
    );
    assert_eq!(
        states(&mut m, &[Signal::ReconcileStarted, Signal::ReconcileDone]),
        [WatchState::Reconciling, WatchState::Live]
    );
}

#[test]
fn a_root_that_cannot_be_reopened_is_lost_again_and_watched_again() {
    let mut m = live();
    let _ = feed(&mut m, &[Signal::RootMoved, Signal::RootRecreated]);
    assert_eq!(m.state(), State::Restored);
    let out = m.signal(Signal::ArmFailed);
    assert_eq!(m.state(), State::Lost(Reason::ArmFailed));
    assert!(actions(&out).contains(&Action::WatchParent));
    // And it can come back again.
    let _ = m.signal(Signal::RootRecreated);
    assert_eq!(m.state(), State::Restored);
}

#[test]
fn an_unexpected_error_is_verified_before_anything_else_happens() {
    let mut m = live();
    let out = m.signal(Signal::IoError);
    assert_eq!(out.transition, None);
    assert_eq!(actions(&out), [Action::Verify]);

    let out = m.signal(Signal::Verified(Verdict::Ok));
    assert_eq!(m.state(), State::Overflowed);
    assert_eq!(actions(&out), [Action::Reopen, Action::Reconcile(None)]);

    for (verdict, reason) in [
        (Verdict::NotReady, Reason::DeviceNotReady),
        (Verdict::Gone, Reason::DeviceGone),
    ] {
        let mut m = live();
        let _ = m.signal(Signal::IoError);
        let out = m.signal(Signal::Verified(verdict));
        assert_eq!(m.state(), State::Lost(reason));
        assert_eq!(actions(&out), [Action::CloseHandle]);
        // A later probe that finds the device again restores the watch.
        let out = m.signal(Signal::Verified(Verdict::Ok));
        assert_eq!(m.state(), State::Restored);
        assert_eq!(actions(&out), [Action::Reopen, Action::Reconcile(None)]);
    }
}

#[test]
fn query_remove_closes_the_handle_and_a_refusal_reopens_it() {
    let mut m = live();
    let out = m.signal(Signal::QueryRemove);
    assert_eq!(m.state(), State::Suspended(Why::QueryRemove));
    assert_eq!(actions(&out), [Action::CloseHandle]);
    let out = m.signal(Signal::QueryRemoveFailed);
    assert_eq!(m.state(), State::Restored);
    assert_eq!(actions(&out), [Action::Reopen, Action::Reconcile(None)]);

    let mut m = live();
    let _ = m.signal(Signal::QueryRemove);
    let out = m.signal(Signal::RemoveComplete);
    assert_eq!(m.state(), State::Lost(Reason::DeviceRemoved));
    assert_eq!(out.actions().count(), 0, "the handle is already closed");
}

#[test]
fn out_of_order_pnp_messages_do_not_confuse_it() {
    // REMOVECOMPLETE without QUERYREMOVE.
    let mut m = live();
    let out = m.signal(Signal::RemoveComplete);
    assert_eq!(m.state(), State::Lost(Reason::DeviceRemoved));
    assert_eq!(actions(&out), [Action::CloseHandle]);
    // QUERYREMOVEFAILED without QUERYREMOVE: ignored.
    let mut m = live();
    assert_eq!(m.signal(Signal::QueryRemoveFailed), Outcome::default());
    assert_eq!(m.state(), State::Live);
    // Repeated mount messages.
    assert_eq!(m.signal(Signal::VolumeBack), Outcome::default());
    assert_eq!(m.signal(Signal::VolumeBack), Outcome::default());
    // Repeated QUERYREMOVE is one row.
    let _ = m.signal(Signal::QueryRemove);
    assert_eq!(m.signal(Signal::QueryRemove), Outcome::default());
    // A removed volume that arrives again is restored.
    let _ = m.signal(Signal::RemoveComplete);
    let out = m.signal(Signal::VolumeBack);
    assert_eq!(m.state(), State::Restored);
    assert_eq!(actions(&out), [Action::Reopen, Action::Reconcile(None)]);
}

#[test]
fn a_lock_suspends_and_unlock_or_failure_restores() {
    for first in [Signal::VolumeLock, Signal::Dismount] {
        for back in [Signal::LockFailed, Signal::VolumeBack] {
            let mut m = live();
            let out = m.signal(first);
            assert_eq!(m.state(), State::Suspended(Why::Locked));
            assert_eq!(actions(&out), [Action::CloseHandle]);
            let _ = m.signal(back);
            assert_eq!(m.state(), State::Restored);
        }
    }
}

#[test]
fn a_user_pause_keeps_the_handle_and_survives_a_volume_interruption() {
    let mut m = live();
    let out = m.signal(Signal::UserPause);
    assert_eq!(m.state(), State::Suspended(Why::User));
    assert_eq!(out.actions().count(), 0, "the watch stays armed (ADR 0016)");

    // A volume interruption while paused: close, then back to the pause.
    let out = m.signal(Signal::QueryRemove);
    assert_eq!(actions(&out), [Action::CloseHandle]);
    let out = m.signal(Signal::QueryRemoveFailed);
    assert_eq!(m.state(), State::Suspended(Why::User));
    assert_eq!(
        actions(&out),
        [Action::Reopen, Action::Reconcile(None)]
            .iter()
            .take(1)
            .copied()
            .collect::<Vec<_>>()
    );

    // Resume: the interruption left a reconcile wanted, so it goes through
    // Overflowed, with the missed count as a lower bound.
    let out = m.signal(Signal::UserResume {
        missed: 7,
        lower_bound: false,
    });
    assert_eq!(m.state(), State::Overflowed);
    let t = out.transition.unwrap();
    assert_eq!((t.missed, t.lower_bound), (7, true));
    assert_eq!(actions(&out), [Action::Reconcile(None)]);
}

#[test]
fn a_resume_without_a_gap_is_live_with_the_missed_count() {
    let mut m = live();
    let _ = m.signal(Signal::UserPause);
    let out = m.signal(Signal::UserResume {
        missed: 12,
        lower_bound: false,
    });
    assert_eq!(m.state(), State::Live);
    let t = out.transition.unwrap();
    assert_eq!(
        (t.missed, t.lower_bound, t.reason),
        (12, false, Reason::UserResume)
    );
    let event = t.to_event(RootIdx(2), VolIdx(1), 10, 20);
    assert_eq!(event.kind, Kind::WatchStateChanged(WatchState::Live));
    assert_eq!(event.size, 12);
    assert_eq!(
        event.raw_mask >> 16,
        u32::from(
            Kind::WatchStateChanged(WatchState::SuspendedByUser)
                .to_wire()
                .1
        )
    );
    assert_eq!(
        event.raw_mask & 0xFFFF,
        u32::from(Reason::UserResume.code())
    );
    assert_eq!(event.source, Source::System);
}

#[test]
fn a_gap_while_paused_means_a_reconcile_on_resume() {
    let mut m = live();
    let _ = m.signal(Signal::UserPause);
    assert_eq!(
        m.signal(Signal::Gap(GapReason::BufferOverflow)),
        Outcome::default()
    );
    let out = m.signal(Signal::UserResume {
        missed: 3,
        lower_bound: true,
    });
    assert_eq!(m.state(), State::Overflowed);
    assert_eq!(actions(&out), [Action::Reconcile(None)]);
}

#[test]
fn power_messages_flush_and_verify_without_a_row() {
    let mut m = live();
    let out = m.signal(Signal::PowerSuspend);
    assert_eq!(
        (out.transition, actions(&out)),
        (None, vec![Action::FlushNow])
    );
    let out = m.signal(Signal::PowerResume);
    assert_eq!(
        (out.transition, actions(&out)),
        (None, vec![Action::Verify])
    );
}

#[test]
fn every_state_is_visible_over_ipc() {
    let all = [
        State::Arming,
        State::Live,
        State::Overflowed,
        State::Reconciling,
        State::Suspended(Why::User),
        State::Suspended(Why::QueryRemove),
        State::Suspended(Why::Locked),
        State::Lost(Reason::RootDeleted),
        State::Restored,
    ];
    let codes: Vec<WatchStateCode> = all.iter().map(|s| s.ipc_code()).collect();
    assert!(codes.contains(&WatchStateCode::Gap));
    assert!(codes.contains(&WatchStateCode::Lost));
    assert!(codes.contains(&WatchStateCode::Paused));
    // Distinct states map to distinct event-model states, except the three
    // suspensions which have their own.
    let mut seen: Vec<WatchState> = all.iter().map(|s| s.watch_state()).collect();
    seen.sort_by_key(|s| Kind::WatchStateChanged(*s).to_wire());
    seen.dedup();
    assert_eq!(seen.len(), all.len());
}

#[test]
fn reason_codes_are_stable() {
    let all = [
        (Reason::Armed, 1),
        (Reason::ArmFailed, 2),
        (Reason::Overflow, 3),
        (Reason::RingFull, 4),
        (Reason::ParseError, 5),
        (Reason::ReconcileStarted, 6),
        (Reason::ReconcileDone, 7),
        (Reason::ReconcileCancelled, 8),
        (Reason::UserPause, 9),
        (Reason::UserResume, 10),
        (Reason::QueryRemove, 11),
        (Reason::QueryRemoveFailed, 12),
        (Reason::DeviceRemoved, 13),
        (Reason::VolumeLocked, 14),
        (Reason::VolumeDismounted, 15),
        (Reason::VolumeBack, 16),
        (Reason::RootDeleted, 17),
        (Reason::RootMoved, 18),
        (Reason::RootRestored, 19),
        (Reason::DeviceNotReady, 20),
        (Reason::DeviceGone, 21),
        (Reason::VerifiedOk, 22),
    ];
    for (reason, code) in all {
        assert_eq!(reason.code(), code);
    }
}

// Random signal sequences, in any order.

fn arb_signal() -> impl Strategy<Value = Signal> {
    prop_oneof![
        Just(Signal::Armed),
        Just(Signal::ArmFailed),
        Just(Signal::Gap(GapReason::BufferOverflow)),
        Just(Signal::Gap(GapReason::RingFull)),
        Just(Signal::Gap(GapReason::ParserError)),
        Just(Signal::IoError),
        Just(Signal::Verified(Verdict::Ok)),
        Just(Signal::Verified(Verdict::NotReady)),
        Just(Signal::Verified(Verdict::Gone)),
        Just(Signal::RootDeleted),
        Just(Signal::RootMoved),
        Just(Signal::RootRecreated),
        Just(Signal::ReconcileStarted),
        Just(Signal::ReconcileDone),
        Just(Signal::ReconcileCancelled),
        Just(Signal::UserPause),
        (0..5_u64, any::<bool>()).prop_map(|(missed, lower_bound)| Signal::UserResume {
            missed,
            lower_bound
        }),
        Just(Signal::QueryRemove),
        Just(Signal::QueryRemoveFailed),
        Just(Signal::RemovePending),
        Just(Signal::RemoveComplete),
        Just(Signal::VolumeLock),
        Just(Signal::Dismount),
        Just(Signal::LockFailed),
        Just(Signal::VolumeBack),
        Just(Signal::PowerSuspend),
        Just(Signal::PowerResume),
    ]
}

struct Signals;

impl ReferenceStateMachine for Signals {
    type State = u8;
    type Transition = Signal;

    fn init_state() -> BoxedStrategy<u8> {
        Just(0).boxed()
    }

    fn transitions(_: &u8) -> BoxedStrategy<Signal> {
        arb_signal().boxed()
    }

    fn apply(state: u8, _: &Signal) -> u8 {
        state
    }
}

struct Sut {
    machine: Machine,
    rows: usize,
    transitions: usize,
}

impl StateMachineTest for Sut {
    type SystemUnderTest = Self;
    type Reference = Signals;

    fn init_test(_: &u8) -> Self {
        Self {
            machine: Machine::new(),
            rows: 0,
            transitions: 0,
        }
    }

    fn apply(mut sut: Self, _: &u8, signal: Signal) -> Self {
        let before = sut.machine;
        let out = sut.machine.signal(signal);
        let after = sut.machine;
        let acts: Vec<Action> = out.actions().collect();
        match out.transition {
            None => assert_eq!(
                before.state(),
                after.state(),
                "{signal:?} changed state silently"
            ),
            Some(t) => {
                // One transition is one row, from the state we were in to the
                // state we are in, and never to itself.
                assert_eq!(t.from, before.state());
                assert_eq!(t.to, after.state());
                assert_ne!(t.from, t.to);
                sut.rows += 1;
                sut.transitions += 1;
                // Waiting for a reconcile means one was asked for.
                if matches!(t.to, State::Overflowed | State::Restored) {
                    assert!(
                        acts.iter().any(|a| matches!(a, Action::Reconcile(_))),
                        "{signal:?}: {t:?} without a reconcile"
                    );
                }
                // A lost watch gave its handle back.
                if matches!(t.to, State::Lost(_)) && t.from.handle_open() {
                    assert!(acts.contains(&Action::CloseHandle), "{signal:?}: {t:?}");
                }
                // A suspension for the volume closed it.
                if matches!(t.to, State::Suspended(Why::QueryRemove | Why::Locked))
                    && t.from.handle_open()
                {
                    assert!(acts.contains(&Action::CloseHandle), "{signal:?}: {t:?}");
                }
                // Leaving a closed state reopens.
                if !t.from.handle_open() && t.to.handle_open() {
                    assert!(acts.contains(&Action::Reopen), "{signal:?}: {t:?}");
                }
            }
        }
        sut
    }

    fn check_invariants(sut: &Self, _: &u8) {
        let state = sut.machine.state();
        // A user pause is never undone by anything but the user.
        if sut.machine.user_paused() {
            assert!(
                matches!(state, State::Suspended(_) | State::Lost(_)),
                "paused but {state:?}"
            );
        }
        // Every state has a name the viewer can show.
        let _ = state.ipc_code();
        let _ = state.watch_state();
        assert_eq!(sut.rows, sut.transitions);
    }
}

prop_state_machine! {
    #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]

    #[test]
    fn any_signal_order_keeps_the_invariants(sequential 1..80 => Sut);
}

#[test]
fn each_gap_reason_has_its_own_transition_reason() {
    for (gap, reason) in [
        (GapReason::RingFull, Reason::RingFull),
        (GapReason::ParserError, Reason::ParseError),
        (GapReason::BufferOverflow, Reason::Overflow),
        (GapReason::NotifyEnumDir, Reason::Overflow),
        (GapReason::JournalEntryDeleted, Reason::Overflow),
        (GapReason::JournalIdChanged, Reason::Overflow),
        (GapReason::EngineNotRunning, Reason::Overflow),
    ] {
        let mut m = live();
        let out = m.signal(Signal::Gap(gap));
        assert_eq!(out.transition.unwrap().reason, reason, "{gap:?}");
    }
}

#[test]
fn the_machine_remembers_that_the_user_paused() {
    let mut m = live();
    assert!(!m.user_paused());
    let _ = m.signal(Signal::UserPause);
    assert!(m.user_paused());
}

#[test]
fn volume_events_suspend_a_running_watch_with_their_own_reason() {
    for (signal, why, reason) in [
        (Signal::QueryRemove, Why::QueryRemove, Reason::QueryRemove),
        (Signal::VolumeLock, Why::Locked, Reason::VolumeLocked),
        (Signal::Dismount, Why::Locked, Reason::VolumeDismounted),
    ] {
        let mut m = live();
        let out = m.signal(signal);
        assert_eq!(m.state(), State::Suspended(why), "{signal:?}");
        assert_eq!(out.transition.unwrap().reason, reason, "{signal:?}");
        assert_eq!(actions(&out), [Action::CloseHandle], "{signal:?}");
        // The same event again changes nothing.
        assert_eq!(m.signal(signal), Outcome::default(), "{signal:?}");
    }
}

#[test]
fn a_later_volume_event_replaces_the_reason_without_closing_twice() {
    let mut m = live();
    let _ = m.signal(Signal::QueryRemove);
    let out = m.signal(Signal::VolumeLock);
    assert_eq!(m.state(), State::Suspended(Why::Locked));
    assert_eq!(out.transition.unwrap().reason, Reason::VolumeLocked);
    assert_eq!(actions(&out), Vec::<Action>::new());
    // Locked already: a dismount is the same suspension, not a new row.
    assert_eq!(m.signal(Signal::Dismount), Outcome::default());
}

#[test]
fn a_lost_watch_does_not_react_to_volume_events() {
    for signal in [Signal::QueryRemove, Signal::VolumeLock, Signal::Dismount] {
        let mut m = live();
        let _ = m.signal(Signal::RootDeleted);
        assert!(matches!(m.state(), State::Lost(_)));
        let before = m.state();
        assert_eq!(m.signal(signal), Outcome::default(), "{signal:?}");
        assert_eq!(m.state(), before, "{signal:?}");
    }
}

#[test]
fn a_refusal_only_resumes_what_it_refuses() {
    // The removal that was refused resumes; a failed lock does not.
    let mut m = live();
    let _ = m.signal(Signal::QueryRemove);
    assert_eq!(m.signal(Signal::LockFailed), Outcome::default());
    assert_eq!(m.state(), State::Suspended(Why::QueryRemove));
    let out = m.signal(Signal::QueryRemoveFailed);
    assert_eq!(out.transition.unwrap().reason, Reason::QueryRemoveFailed);

    // A refused removal does not unlock a locked volume.
    let mut m = live();
    let _ = m.signal(Signal::VolumeLock);
    assert_eq!(m.signal(Signal::QueryRemoveFailed), Outcome::default());
    assert_eq!(m.state(), State::Suspended(Why::Locked));
    let out = m.signal(Signal::LockFailed);
    assert_eq!(out.transition.unwrap().reason, Reason::VolumeBack);

    // And a refused removal does not bring back a watch whose device went.
    let mut m = live();
    let _ = m.signal(Signal::RemoveComplete);
    let lost = m.state();
    assert!(matches!(lost, State::Lost(_)));
    assert_eq!(m.signal(Signal::QueryRemoveFailed), Outcome::default());
    assert_eq!(m.state(), lost);
}

#[test]
fn a_deleted_root_and_a_moved_root_are_lost_for_their_own_reasons() {
    let mut m = live();
    let out = m.signal(Signal::RootDeleted);
    assert_eq!(out.transition.unwrap().reason, Reason::RootDeleted);
    let mut m = live();
    let out = m.signal(Signal::RootMoved);
    assert_eq!(out.transition.unwrap().reason, Reason::RootMoved);
}
