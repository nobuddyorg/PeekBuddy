//! Tests of the OS-independent loop with a fake file system and clock.
#![expect(
    clippy::indexing_slicing,
    reason = "tests index values of known size; a panic is a test failure"
)]

use std::sync::atomic::{AtomicU64, Ordering};

use super::{
    CAPACITY, Clock, Config, MIN_SLEEP, Pacer, Planned, Target, ThreadPlan, ThreadRun, Timing,
    assemble, filetime, plan_thread, run, run_thread,
};
use crate::ops::{Op, OpKind};
use crate::plan::{Planner, Shape};
use crate::profile::{Names, Profile, Rate, Verdict, profile};

/// A clock that only moves when told: each `now` costs `step` ticks, each
/// sleep advances by exactly the requested time.
struct FakeClock {
    now: u64,
    step: u64,
    slept: Vec<u64>,
}

impl FakeClock {
    fn new(step: u64) -> Self {
        Self {
            now: 1_000_000,
            step,
            slept: Vec::new(),
        }
    }
}

impl Clock for FakeClock {
    fn now(&mut self) -> u64 {
        self.now += self.step;
        self.now
    }

    fn sleep(&mut self, ticks: u64) {
        self.slept.push(ticks);
        self.now += ticks;
    }
}

/// Records every operation; fails the classes in `fail` with error 5.
#[derive(Default)]
struct FakeTarget {
    applied: Vec<Op>,
    fail: Vec<OpKind>,
}

impl Target for FakeTarget {
    fn apply(&mut self, op: &Op) -> u32 {
        self.applied.push(op.clone());
        if self.fail.contains(&op.kind) { 5 } else { 0 }
    }
}

const SMALL: Shape = Shape {
    depth: 1,
    width: 2,
    files: 1,
};

#[test]
fn the_pacer_spaces_events_by_the_rate() {
    // 1,000 events/s: one event every 10,000 ticks.
    let mut pacer = Pacer::new(Some(1_000), 0);
    assert_eq!(pacer.delay(0), 0);
    pacer.issued(1);
    assert_eq!(pacer.delay(0), 10_000);
    assert_eq!(pacer.delay(4_000), 6_000);
    assert_eq!(pacer.delay(10_000), 0);
    pacer.issued(2);
    assert_eq!(pacer.delay(10_000), 20_000);
}

#[test]
fn the_pacer_drops_backlog_beyond_its_capacity() {
    let mut pacer = Pacer::new(Some(1_000), 0);
    pacer.issued(1);
    // 1 s late: only CAPACITY of it (10 events) may be caught up.
    assert_eq!(pacer.delay(10_000_000), 0);
    pacer.issued(1);
    for _ in 0..CAPACITY / 10_000 {
        assert_eq!(pacer.delay(10_000_000), 0);
        pacer.issued(1);
    }
    assert_eq!(pacer.delay(10_000_000), 10_000);
    // Exactly at capacity nothing is dropped.
    let mut edge = Pacer::new(Some(1_000), 0);
    edge.issued(1);
    assert_eq!(edge.delay(10_000 + CAPACITY), 0);
    edge.issued(CAPACITY / 10_000 + 1);
    assert_eq!(edge.delay(10_000 + CAPACITY), 10_000);
}

#[test]
fn an_unpaced_pacer_never_waits() {
    let mut pacer = Pacer::new(None, 0);
    pacer.issued(1_000_000);
    assert_eq!(pacer.delay(0), 0);
    let mut zero = Pacer::new(Some(0), 0);
    zero.issued(10);
    assert_eq!(zero.delay(0), 0);
}

#[test]
fn plans_reach_their_event_count() {
    let profile = profile("edge-mix").unwrap();
    let plan = plan_thread(1, 2, profile, SMALL, 300);
    let events: u64 = plan.ops.iter().map(|p| p.events).sum();
    assert!(events >= 300);
    let last = plan.ops.last().unwrap().events;
    assert!(events - last < 300, "plans one operation too many");
    for planned in plan.setup.iter().chain(&plan.ops) {
        assert_eq!(planned.events, planned.op.expected().len() as u64);
    }
    // The same stream the planner produces on its own.
    let mut planner = Planner::new(1, 2, profile);
    let setup = planner.setup(SMALL);
    assert_eq!(
        plan.setup.iter().map(|p| &p.op).collect::<Vec<_>>(),
        setup.iter().collect::<Vec<_>>()
    );
    for planned in &plan.ops {
        assert_eq!(planned.op, planner.next_op());
    }
    assert_eq!(plan.thread, 2);
    assert_eq!(plan_thread(1, 0, profile, SMALL, 0).ops, []);
}

#[test]
fn a_thread_runs_setup_then_paced_operations() {
    let plan = plan_thread(1, 0, profile("latency-10k").unwrap(), SMALL, 200);
    let mut target = FakeTarget::default();
    // 1 tick per clock read: the target is never the bottleneck.
    let mut clock = FakeClock::new(1);
    let mut ready_at = None;
    let run = run_thread(&plan, Some(1_000), &mut target, &mut clock, || {
        ready_at = Some(0);
    });
    assert!(ready_at.is_some());
    assert_eq!(run.setup.len(), plan.setup.len());
    assert_eq!(run.ops.len(), plan.ops.len());
    let all: Vec<&Op> = plan.setup.iter().chain(&plan.ops).map(|p| &p.op).collect();
    assert_eq!(target.applied.iter().collect::<Vec<_>>(), all);
    // Setup is unpaced; sleeps are never shorter than MIN_SLEEP.
    assert!(clock.slept.iter().all(|&ticks| ticks >= MIN_SLEEP));
    let first = run.ops.first().unwrap().t_start;
    let last = run.ops.last().unwrap().t_end;
    let events: u64 = plan.ops.iter().map(|p| p.events).sum();
    // ~events / 1,000 per s, within a millisecond of slack per thread.
    let ideal = (events - plan.ops.last().unwrap().events) * 10_000;
    assert!(
        last - first + MIN_SLEEP >= ideal,
        "{} vs {ideal}",
        last - first
    );
    assert!(
        last - first <= ideal + 2 * MIN_SLEEP,
        "{} vs {ideal}",
        last - first
    );
    for timing in run.setup.iter().chain(&run.ops) {
        assert!(timing.t_end > timing.t_start);
        assert_eq!(timing.result, 0);
    }
}

#[test]
fn the_ready_hook_runs_after_setup_and_before_the_first_paced_op() {
    struct Counting(u64);
    impl Target for Counting {
        fn apply(&mut self, _: &Op) -> u32 {
            self.0 += 1;
            0
        }
    }
    let plan = plan_thread(3, 0, profile("edge-mix").unwrap(), SMALL, 20);
    let applied = AtomicU64::new(0);
    let mut target = Counting(0);
    let mut clock = FakeClock::new(1);
    let target_ref = &mut target;
    let run = run_thread(&plan, None, target_ref, &mut clock, || {
        applied.store(1, Ordering::Relaxed);
    });
    assert_eq!(applied.load(Ordering::Relaxed), 1);
    assert_eq!(target.0, (plan.setup.len() + plan.ops.len()) as u64);
    assert_eq!(clock.slept, [0u64; 0], "an unpaced run never sleeps");
    assert_eq!(run.ops.len(), plan.ops.len());
}

fn op(kind: OpKind, path: &str) -> Op {
    Op {
        kind,
        paths: vec![path.encode_utf16().collect()],
        removed_below: Vec::new(),
    }
}

fn timing(t_start: u64, t_end: u64, result: u32) -> Timing {
    Timing {
        t_start,
        t_end,
        result,
    }
}

const STEADY: Profile = Profile {
    name: "test",
    mix: &[(OpKind::CreateFile, 1), (OpKind::Delete, 1)],
    rate: Rate::Steady {
        events_per_s: 2,
        seconds: 1,
    },
    names: Names::Ascii,
};

#[test]
fn assemble_orders_by_end_time_and_counts_paced_events() {
    let a = ThreadPlan {
        thread: 0,
        setup: vec![Planned::new(op(OpKind::CreateDir, "t0"))],
        ops: vec![
            Planned::new(op(OpKind::CreateFile, "t0\\f1")),
            Planned::new(op(OpKind::Delete, "t0\\f1")),
        ],
    };
    let b = ThreadPlan {
        thread: 1,
        setup: Vec::new(),
        ops: vec![Planned::new(op(OpKind::CreateFile, "t1\\f1"))],
    };
    let run_a = ThreadRun {
        setup: vec![timing(0, 5, 0)],
        ops: vec![timing(10, 30, 0), timing(30, 50, 2)],
    };
    // Same end time as a's first op: thread order breaks the tie.
    let run_b = ThreadRun {
        setup: Vec::new(),
        ops: vec![timing(20, 30, 0)],
    };
    let out = assemble(&STEADY, &[(a, run_a), (b, run_b)]);
    let order: Vec<(u64, u32, u64)> = out
        .records
        .iter()
        .map(|r| (r.seq, r.thread, r.t_end))
        .collect();
    assert_eq!(order, [(0, 0, 5), (1, 0, 30), (2, 1, 30), (3, 0, 50)]);
    // The failed delete expects nothing and adds no events.
    assert_eq!(out.records[3].result, 2);
    assert_eq!(out.records[3].expected, []);
    assert_eq!(out.events, 2);
    // Paced span: 10 to 50 ticks, setup excluded.
    assert_eq!(out.elapsed, 40);
    assert_eq!(out.achieved_per_s, 2 * 10_000_000 / 40);
    assert_eq!(out.verdict, Verdict::Exercised);
    assert_eq!(out.classes[&OpKind::CreateDir].ok, 1);
    assert_eq!(out.classes[&OpKind::CreateFile].ok, 2);
    assert_eq!(out.classes[&OpKind::Delete].failed, 1);
    assert_eq!(out.failures(), 1);
    assert_eq!(out.missing_classes(&STEADY), [OpKind::Delete]);
}

#[test]
fn a_slow_run_is_not_exercised() {
    let plan = ThreadPlan {
        thread: 0,
        setup: Vec::new(),
        ops: vec![Planned::new(op(OpKind::CreateFile, "t0\\f1"))],
    };
    // One event in 1 s against a target of 2 per second.
    let run = ThreadRun {
        setup: Vec::new(),
        ops: vec![timing(0, 10_000_000, 0)],
    };
    let out = assemble(&STEADY, &[(plan, run)]);
    assert_eq!(out.achieved_per_s, 1);
    assert_eq!(out.verdict, Verdict::NotExercised);
    assert_eq!(out.missing_classes(&STEADY), [OpKind::Delete]);
}

#[test]
fn config_splits_events_and_rate_over_threads() {
    let config = |name, threads| Config {
        profile: profile(name).unwrap(),
        seed: 1,
        threads,
        shape: SMALL,
    };
    let latency = config("latency-10k", 3);
    assert_eq!(latency.events_per_thread(), 100_000);
    // 10,000 ev/s plus 2 % over three threads.
    assert_eq!(latency.per_thread_rate(), Some(3_400));
    let burst = config("burst-100k", 4);
    assert_eq!(burst.events_per_thread(), 250_000);
    assert_eq!(burst.per_thread_rate(), None);
    // Zero threads means one.
    assert_eq!(config("edge-mix", 0).events_per_thread(), 2_000);
    assert_eq!(config("edge-mix", 0).per_thread_rate(), Some(204));
    assert_eq!(config("edge-mix", 7).per_thread_rate(), Some(30));
}

const TINY: Profile = Profile {
    name: "tiny",
    mix: &[
        (OpKind::CreateFile, 3),
        (OpKind::Write, 2),
        (OpKind::Rename, 1),
    ],
    rate: Rate::Steady {
        events_per_s: 1_000,
        seconds: 1,
    },
    names: Names::Ascii,
};

#[test]
#[cfg_attr(
    miri,
    ignore = "plans thousands of operations on three threads; runs natively on every PR"
)]
fn run_uses_every_thread_and_numbers_records_once() {
    let config = Config {
        profile: &TINY,
        seed: 9,
        threads: 3,
        shape: SMALL,
    };
    let out = run(&config, |_| (FakeTarget::default(), FakeClock::new(1)));
    let seqs: Vec<u64> = out.records.iter().map(|r| r.seq).collect();
    assert_eq!(seqs, (0..out.records.len() as u64).collect::<Vec<_>>());
    for thread in 0..3 {
        let prefix = format!("t{thread}");
        let mine: Vec<_> = out.records.iter().filter(|r| r.thread == thread).collect();
        assert_ne!(mine.len(), 0);
        assert!(mine.iter().all(|r| r.paths[0].starts_with(&prefix)));
    }
    assert!(out.events >= 1_000);
    assert_eq!(out.missing_classes(&TINY), []);
    assert_eq!(out.failures(), 0);
}

#[test]
#[cfg_attr(
    miri,
    ignore = "plans thousands of operations on three threads; runs natively on every PR"
)]
fn same_seed_one_thread_same_ledger_minus_times() {
    let config = Config {
        profile: &TINY,
        seed: 4,
        threads: 1,
        shape: SMALL,
    };
    let strip = |step| {
        let mut records = run(&config, |_| (FakeTarget::default(), FakeClock::new(step))).records;
        for record in &mut records {
            record.t_start = 0;
            record.t_end = 0;
        }
        records
    };
    assert_eq!(strip(1), strip(7));
}

#[test]
fn filetime_counts_from_1601() {
    assert_eq!(filetime(std::time::UNIX_EPOCH), 116_444_736_000_000_000);
    let later = std::time::UNIX_EPOCH + std::time::Duration::from_micros(3);
    assert_eq!(filetime(later), 116_444_736_000_000_030);
}
