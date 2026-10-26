//! The reconcile worker (#46): one background thread that walks a root when
//! something asks for it.
//!
//! Triggers are an overflow, a ring-full gap, a restored watch, an engine
//! start and a USN journal gap; never a timer. While a root is being walked,
//! further triggers for it are coalesced into one follow-up run. The thread
//! runs in `THREAD_MODE_BACKGROUND_BEGIN`, checks a stop flag once per
//! directory and sleeps on a condition variable when idle (no wakeups).
//!
//! The rows go to the store through [`StoreRows`]; the snapshot is committed
//! only after those rows are durable.

use std::collections::VecDeque;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use peekbuddy_core::event::{Event, GapReason, RootIdx, VolIdx};
use peekbuddy_core::reconcile::walk::{Config, Summary, run};
use peekbuddy_core::reconcile::{RowOut, Snapshot};
use peekbuddy_store::batch::Batch;
use peekbuddy_store::pipeline::{Producer, Push};
use peekbuddy_win::reconcile::{BackgroundMode, DirLister};

/// How long a walk waits for its rows to reach the disk.
const FLUSH_DEADLINE: Duration = Duration::from_secs(30);

/// One root the worker can walk.
#[derive(Debug, Clone)]
pub struct RootSpec {
    /// The root's index.
    pub root: RootIdx,
    /// Its volume.
    pub vol: VolIdx,
    /// The `\\?\` path of the directory.
    pub path: Vec<u16>,
}

/// Rows into the writer queue, durable on [`RowOut::flush`].
pub struct StoreRows {
    producer: Producer,
    batch: Option<Batch>,
    lost_before: u64,
}

impl std::fmt::Debug for StoreRows {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoreRows").finish_non_exhaustive()
    }
}

impl StoreRows {
    /// Rows through `producer`.
    #[must_use]
    pub fn new(producer: Producer) -> Self {
        let lost_before = producer.lost_batches();
        Self {
            producer,
            batch: None,
            lost_before,
        }
    }

    fn send(&mut self) {
        if let Some(batch) = self.batch.take() {
            let (Push::Queued(spare) | Push::Overflow(spare) | Push::Closed(spare)) =
                self.producer.push(batch);
            self.batch = Some(spare);
        }
    }
}

impl RowOut for StoreRows {
    fn row(&mut self, event: &Event, name: &[u16]) {
        let mut batch = self
            .batch
            .take()
            .unwrap_or_else(|| self.producer.new_batch());
        if batch.push(*event, name).is_err() {
            self.batch = Some(batch);
            self.send();
            batch = self
                .batch
                .take()
                .unwrap_or_else(|| self.producer.new_batch());
            let _ = batch.push(*event, name);
        }
        self.batch = Some(batch);
    }

    fn flush(&mut self) -> bool {
        if self.batch.as_ref().is_some_and(|b| !b.is_empty()) {
            self.send();
        }
        match self.producer.flush(FLUSH_DEADLINE) {
            Ok(flushed) => {
                // Batches lost since the last walk may have held rows of this
                // one: the snapshot must not move past rows that are gone.
                let intact = flushed.lost_batches == self.lost_before;
                self.lost_before = flushed.lost_batches;
                intact
            }
            Err(_) => false,
        }
    }
}

/// What the worker did, for diagnostics and tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stats {
    /// Walks finished (including cancelled ones).
    pub runs: u64,
    /// Triggers folded into a walk that was already queued or running.
    pub coalesced: u64,
    /// The summary of the last walk.
    pub last: Option<Summary>,
}

struct State {
    queue: VecDeque<(RootIdx, Option<GapReason>)>,
    running: Option<RootIdx>,
    /// A trigger arrived for the running root: walk it once more.
    again: Option<(RootIdx, Option<GapReason>)>,
    stop: bool,
    stats: Stats,
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    idle: Condvar,
    cancel: AtomicBool,
}

fn lock(state: &Mutex<State>) -> std::sync::MutexGuard<'_, State> {
    state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The worker thread.
#[derive(Debug)]
pub struct Reconciler {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shared").finish_non_exhaustive()
    }
}

impl Reconciler {
    /// Starts the worker for `roots`. The snapshot and the row sink move into
    /// the thread.
    ///
    /// # Errors
    ///
    /// The OS error when the thread cannot be spawned.
    pub fn start<N, R>(
        snapshot: N,
        rows: R,
        roots: Vec<RootSpec>,
        clock: fn() -> (u64, u64),
    ) -> io::Result<Self>
    where
        N: Snapshot + Send + 'static,
        R: RowOut + Send + 'static,
    {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                queue: VecDeque::new(),
                running: None,
                again: None,
                stop: false,
                stats: Stats::default(),
            }),
            wake: Condvar::new(),
            idle: Condvar::new(),
            cancel: AtomicBool::new(false),
        });
        let inner = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name(String::from("peekbuddy-reconcile"))
            .spawn(move || work(&inner, snapshot, rows, &roots, clock))?;
        Ok(Self {
            shared,
            thread: Some(thread),
        })
    }

    /// Asks for a walk of `root`. A root that is already queued is not
    /// queued twice; one that is running gets exactly one follow-up.
    pub fn trigger(&self, root: RootIdx, reason: Option<GapReason>) {
        let mut state = lock(&self.shared.state);
        if state.stop {
            return;
        }
        if state.running == Some(root) {
            if state.again.is_some_and(|(r, _)| r == root) {
                state.stats.coalesced += 1;
            } else {
                state.again = Some((root, reason));
            }
        } else if state.queue.iter().any(|(r, _)| *r == root) {
            state.stats.coalesced += 1;
        } else {
            state.queue.push_back((root, reason));
        }
        drop(state);
        self.shared.wake.notify_one();
    }

    /// Stops the walk in progress at the next directory; the next queued
    /// trigger runs normally.
    pub fn cancel_current(&self) {
        self.shared.cancel.store(true, Ordering::Release);
    }

    /// The counters.
    #[must_use]
    pub fn stats(&self) -> Stats {
        lock(&self.shared.state).stats
    }

    /// Waits until nothing is queued or running, at most `timeout`.
    #[must_use]
    pub fn wait_idle(&self, timeout: Duration) -> bool {
        let mut state = lock(&self.shared.state);
        let deadline = std::time::Instant::now() + timeout;
        while !(state.queue.is_empty() && state.running.is_none() && state.again.is_none()) {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return false;
            }
            state = self
                .shared
                .idle
                .wait_timeout(state, left)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
        true
    }

    /// Cancels the walk in progress, drops what is queued and joins the
    /// thread.
    pub fn stop(&mut self) {
        {
            let mut state = lock(&self.shared.state);
            state.stop = true;
            state.queue.clear();
            state.again = None;
        }
        self.shared.cancel.store(true, Ordering::Release);
        self.shared.wake.notify_all();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Reconciler {
    fn drop(&mut self) {
        self.stop();
    }
}

fn work<N: Snapshot, R: RowOut>(
    shared: &Shared,
    mut snapshot: N,
    mut rows: R,
    roots: &[RootSpec],
    clock: fn() -> (u64, u64),
) {
    let _background = BackgroundMode::enter();
    loop {
        let (root, reason) = {
            let mut state = lock(&shared.state);
            loop {
                if state.stop {
                    return;
                }
                if let Some(next) = state.queue.pop_front() {
                    state.running = Some(next.0);
                    shared.cancel.store(false, Ordering::Release);
                    break next;
                }
                state = shared
                    .wake
                    .wait(state)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
        };
        let summary = roots
            .iter()
            .find(|spec| spec.root == root)
            .and_then(|spec| {
                let mut source = DirLister::new(&spec.path);
                let cfg = Config {
                    root: spec.root,
                    vol: spec.vol,
                    reason,
                    cancel: &shared.cancel,
                    clock,
                };
                run(&cfg, &mut source, &mut snapshot, &mut rows).ok()
            });
        let mut state = lock(&shared.state);
        state.running = None;
        state.stats.runs += 1;
        state.stats.last = summary;
        if let Some(again) = state.again.take().filter(|_| !state.stop) {
            state.queue.push_back(again);
        }
        drop(state);
        shared.idle.notify_all();
    }
}
