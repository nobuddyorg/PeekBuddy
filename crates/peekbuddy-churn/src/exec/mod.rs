//! Runs plans against a file system and produces the ledger.
//!
//! The loop is OS-independent: it drives a [`Target`] (the file system) and
//! a [`Clock`] (time and sleep), so it builds and tests on Linux with fakes.
//! The Windows target and clock are in `exec::windows`.
//!
//! Each thread plans its whole stream before the clock starts, runs its
//! setup (the initial tree), waits for the other threads and then runs its
//! operations, paced by a token bucket. Timings go into a vector sized up
//! front; ledger records are built after the run, so the loop does no
//! ledger work and no I/O besides the operations.

#[cfg(windows)]
pub mod windows;

use std::collections::BTreeMap;
use std::sync::Barrier;

use crate::ledger::Record;
use crate::ops::{Op, OpKind};
use crate::plan::{Planner, Shape};
use crate::profile::{Profile, Rate, Verdict, achieved_per_s, verdict};

/// FILETIME ticks (100 ns) per second.
const TICKS_PER_S: u64 = 10_000_000;

/// Shortest sleep worth taking (1 ms). Smaller debts run early instead, so
/// pacing never sleeps per operation at high rates.
pub const MIN_SLEEP: u64 = 10_000;

/// How far a thread may fall behind before the bucket drops the backlog
/// (10 ms), so a stall is not followed by an unbounded burst.
pub const CAPACITY: u64 = 100_000;

/// The file system the operations run against.
pub trait Target {
    /// Performs `op` and returns the Win32 error code, 0 on success.
    fn apply(&mut self, op: &Op) -> u32;
}

/// Time source and sleep, one per thread.
pub trait Clock {
    /// Now, as UTC FILETIME ticks.
    fn now(&mut self) -> u64;
    /// Blocks for `ticks` of 100 ns.
    fn sleep(&mut self, ticks: u64);
}

/// A token bucket: `per_s` raw events per second with [`CAPACITY`] of
/// backlog; `None` runs unpaced.
#[derive(Debug, Clone)]
pub struct Pacer {
    per_s: Option<u64>,
    start: u64,
    issued: u64,
}

impl Pacer {
    /// A bucket that starts at `start`.
    #[must_use]
    pub fn new(per_s: Option<u64>, start: u64) -> Self {
        Self {
            per_s,
            start,
            issued: 0,
        }
    }

    /// Ticks to wait at `now` before the next operation; 0 to go on.
    pub fn delay(&mut self, now: u64) -> u64 {
        let Some(per_s) = self.per_s.filter(|&rate| rate > 0) else {
            return 0;
        };
        let offset = u128::from(self.issued) * u128::from(TICKS_PER_S) / u128::from(per_s);
        let due = self
            .start
            .saturating_add(u64::try_from(offset).unwrap_or(u64::MAX));
        // Behind schedule: drop the backlog beyond CAPACITY.
        let behind = now.saturating_sub(due);
        self.start = self.start.saturating_add(behind.saturating_sub(CAPACITY));
        due.saturating_sub(now)
    }

    /// Counts `events` raw events as issued.
    pub fn issued(&mut self, events: u64) {
        self.issued = self.issued.saturating_add(events);
    }
}

/// One planned operation and the raw events it should produce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    /// The operation.
    pub op: Op,
    /// `op.expected().len()`, counted at plan time.
    pub events: u64,
}

impl Planned {
    fn new(op: Op) -> Self {
        let events = u64::try_from(op.expected().len()).unwrap_or(u64::MAX);
        Self { op, events }
    }
}

/// One thread's plan: the initial tree and then the paced stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadPlan {
    /// Thread index (`t<n>`).
    pub thread: u32,
    /// Builds the initial tree; unpaced and not part of the rate.
    pub setup: Vec<Planned>,
    /// The paced operations.
    pub ops: Vec<Planned>,
}

/// Plans thread `thread` until its operations add up to `events` raw
/// events.
#[must_use]
pub fn plan_thread(
    seed: u64,
    thread: u32,
    profile: &'static Profile,
    shape: Shape,
    events: u64,
) -> ThreadPlan {
    let mut planner = Planner::new(seed, thread, profile);
    let setup = planner.setup(shape).into_iter().map(Planned::new).collect();
    let mut ops = Vec::new();
    let mut total = 0u64;
    while total < events {
        let next = Planned::new(planner.next_op());
        total = total.saturating_add(next.events);
        ops.push(next);
    }
    ThreadPlan { thread, setup, ops }
}

/// When one operation ran and how it ended.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Timing {
    /// Start, UTC FILETIME ticks.
    pub t_start: u64,
    /// End, UTC FILETIME ticks.
    pub t_end: u64,
    /// Win32 error code, 0 on success.
    pub result: u32,
}

/// What one thread did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThreadRun {
    /// One timing per setup operation.
    pub setup: Vec<Timing>,
    /// One timing per paced operation.
    pub ops: Vec<Timing>,
}

fn timed<T: Target, C: Clock>(target: &mut T, clock: &mut C, op: &Op) -> Timing {
    let t_start = clock.now();
    let result = target.apply(op);
    let t_end = clock.now();
    Timing {
        t_start,
        t_end,
        result,
    }
}

/// Runs one thread's plan. `ready` is called between setup and the paced
/// operations (the start barrier in [`run`]).
pub fn run_thread<T: Target, C: Clock>(
    plan: &ThreadPlan,
    per_s: Option<u64>,
    target: &mut T,
    clock: &mut C,
    ready: impl FnOnce(),
) -> ThreadRun {
    let mut run = ThreadRun {
        setup: Vec::with_capacity(plan.setup.len()),
        ops: Vec::with_capacity(plan.ops.len()),
    };
    for planned in &plan.setup {
        run.setup.push(timed(target, clock, &planned.op));
    }
    ready();
    let mut pacer = Pacer::new(per_s, clock.now());
    for planned in &plan.ops {
        let wait = pacer.delay(clock.now());
        if wait >= MIN_SLEEP {
            clock.sleep(wait);
        }
        run.ops.push(timed(target, clock, &planned.op));
        pacer.issued(planned.events);
    }
    run
}

/// How a whole run is configured.
#[derive(Debug, Clone, Copy)]
pub struct Config {
    /// Workload.
    pub profile: &'static Profile,
    /// Seed of every thread's stream.
    pub seed: u64,
    /// Generator threads (at least 1).
    pub threads: u32,
    /// Initial tree per thread.
    pub shape: Shape,
}

impl Config {
    fn threads(&self) -> u32 {
        self.threads.max(1)
    }

    /// Raw events each thread plans for.
    #[must_use]
    pub fn events_per_thread(&self) -> u64 {
        let total = match self.profile.rate {
            Rate::Steady {
                events_per_s,
                seconds,
            } => events_per_s.saturating_mul(seconds),
            Rate::Burst { events, .. } => events,
        };
        total.div_ceil(u64::from(self.threads()))
    }

    /// Pacing rate of each thread; `None` for a burst.
    #[must_use]
    pub fn per_thread_rate(&self) -> Option<u64> {
        match self.profile.rate {
            Rate::Steady { events_per_s, .. } => {
                Some(events_per_s.div_ceil(u64::from(self.threads())))
            }
            Rate::Burst { .. } => None,
        }
    }
}

/// Successful and failed operations of one class.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClassCount {
    /// Operations that succeeded.
    pub ok: u64,
    /// Operations that failed.
    pub failed: u64,
}

/// The outcome of a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// Ledger records, `seq` in the order operations finished.
    pub records: Vec<Record>,
    /// Raw events of successful paced operations.
    pub events: u64,
    /// From the first paced start to the last paced end, in ticks.
    pub elapsed: u64,
    /// Raw events per second over `elapsed`.
    pub achieved_per_s: u64,
    /// Whether the run reached the profile's target.
    pub verdict: Verdict,
    /// Per class, setup and paced operations together.
    pub classes: BTreeMap<OpKind, ClassCount>,
}

impl Outcome {
    /// Classes of `profile`'s mix that never succeeded.
    #[must_use]
    pub fn missing_classes(&self, profile: &Profile) -> Vec<OpKind> {
        profile
            .mix
            .iter()
            .map(|(kind, _)| *kind)
            .filter(|kind| self.classes.get(kind).is_none_or(|count| count.ok == 0))
            .collect()
    }

    /// Failed operations.
    #[must_use]
    pub fn failures(&self) -> u64 {
        self.classes.values().map(|count| count.failed).sum()
    }
}

/// Builds the outcome from each thread's plan and run.
#[must_use]
pub fn assemble(profile: &Profile, threads: &[(ThreadPlan, ThreadRun)]) -> Outcome {
    let mut done: Vec<(Timing, u32, usize, &Op)> = Vec::new();
    let mut events = 0u64;
    let mut first = u64::MAX;
    let mut last = 0u64;
    let mut classes: BTreeMap<OpKind, ClassCount> = BTreeMap::new();
    for (plan, run) in threads {
        let setup_ops = plan.setup.iter().zip(&run.setup);
        let paced = plan.ops.iter().zip(&run.ops);
        for (index, (planned, timing)) in setup_ops.chain(paced.clone()).enumerate() {
            done.push((*timing, plan.thread, index, &planned.op));
            let count = classes.entry(planned.op.kind).or_default();
            if timing.result == 0 {
                count.ok += 1;
            } else {
                count.failed += 1;
            }
        }
        for (planned, timing) in paced {
            first = first.min(timing.t_start);
            last = last.max(timing.t_end);
            if timing.result == 0 {
                events = events.saturating_add(planned.events);
            }
        }
    }
    done.sort_by_key(|(timing, thread, index, _)| (timing.t_end, *thread, *index));
    let records = done
        .iter()
        .zip(0u64..)
        .map(|((timing, thread, _, op), seq)| {
            Record::new(
                seq,
                *thread,
                op,
                timing.t_start,
                timing.t_end,
                timing.result,
            )
        })
        .collect();
    let elapsed = last.saturating_sub(first);
    let achieved = achieved_per_s(events, elapsed);
    Outcome {
        records,
        events,
        elapsed,
        achieved_per_s: achieved,
        verdict: verdict(profile.rate, achieved),
        classes,
    }
}

/// Runs `config` with one thread per `t<n>`; `make` builds each thread's
/// target and clock.
pub fn run<T, C, F>(config: &Config, make: F) -> Outcome
where
    T: Target,
    C: Clock,
    F: Fn(u32) -> (T, C) + Sync,
{
    let threads = config.threads();
    let start = Barrier::new(usize::try_from(threads).unwrap_or(usize::MAX));
    let per_s = config.per_thread_rate();
    let events = config.events_per_thread();
    let results: Vec<(ThreadPlan, ThreadRun)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|thread| {
                let start = &start;
                let make = &make;
                scope.spawn(move || {
                    let plan =
                        plan_thread(config.seed, thread, config.profile, config.shape, events);
                    let (mut target, mut clock) = make(thread);
                    let run = run_thread(&plan, per_s, &mut target, &mut clock, || {
                        start.wait();
                    });
                    (plan, run)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|payload| std::panic::resume_unwind(payload))
            })
            .collect()
    });
    assemble(config.profile, &results)
}

/// UTC FILETIME ticks of a [`std::time::SystemTime`]; on Windows `now()`
/// reads `GetSystemTimePreciseAsFileTime`.
#[must_use]
pub fn filetime(time: std::time::SystemTime) -> u64 {
    /// 1601-01-01 to 1970-01-01 in ticks.
    const UNIX_EPOCH_TICKS: u64 = 116_444_736_000_000_000;
    let since = time
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let ticks = since.as_nanos() / 100;
    UNIX_EPOCH_TICKS.saturating_add(u64::try_from(ticks).unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests;
