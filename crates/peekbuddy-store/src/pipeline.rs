//! The writer thread, the ring in front of it and the spill path (#52).
//!
//! Capture pushes whole [`Batch`]es into a bounded ring and never touches the
//! disk. One thread owns the [`Writer`] and applies the group-commit
//! [`Policy`]. When the ring is full (the writer is stalled), pushes go to an
//! in-memory overflow list instead and a second thread appends them to the
//! spill file; from then on everything goes there until the writer has caught
//! up and replayed it, which keeps the id order. If even the spill cannot be
//! written (disk full) the batch is dropped and counted in
//! [`Stats::lost_batches`]; the engine reads the counter after every commit
//! and records a gap and asks for a reconcile, so a loss is always detected
//! (F2). Nothing here runs a timer: the writer waits with a
//! timeout only while events are pending, otherwise for ever.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::batch::Batch;
use crate::policy::{Pending, Policy};
use crate::spill::Spill;
use crate::writer::{Commit, Cursor, WAL_CHECKPOINT_BYTES, WriteError, Writer};

/// Commits are retried this many times before the batches are given up.
const COMMIT_ATTEMPTS: u32 = 3;

/// What the thread tells its observers after each commit.
#[derive(Debug, Clone, Copy)]
pub struct CommitInfo {
    /// What was written.
    pub commit: Commit,
    /// Batches in the commit.
    pub batches: usize,
    /// Batches waiting in the ring right after the commit.
    pub ring_depth: usize,
    /// Highest ring depth seen so far.
    pub ring_high_water: usize,
    /// Rows replayed from the spill in this commit.
    pub from_spill: bool,
}

/// A callback run before each commit attempt.
pub type BeforeCommit = Arc<dyn Fn() + Send + Sync>;

/// A callback told about each successful commit.
pub type CommitObserver = Arc<dyn Fn(&CommitInfo) + Send + Sync>;

/// Callbacks the pipeline calls from its threads.
#[derive(Clone, Default)]
pub struct Hooks {
    /// Called before each commit attempt (tests stall the writer here).
    pub before_commit: Option<BeforeCommit>,
    /// Called after each successful commit.
    pub on_commit: Option<CommitObserver>,
}

impl std::fmt::Debug for Hooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hooks").finish_non_exhaustive()
    }
}

/// How the pipeline is sized.
#[derive(Debug, Clone)]
pub struct Config {
    /// Commit policy.
    pub policy: Policy,
    /// Batches the ring holds before overflow goes to the spill file.
    pub ring_batches: usize,
    /// Directory of the spill file (`spill/` in the data dir).
    pub spill_dir: PathBuf,
    /// Callbacks.
    pub hooks: Hooks,
}

impl Config {
    /// The defaults with `spill_dir`.
    #[must_use]
    pub fn new(spill_dir: PathBuf) -> Self {
        Self {
            policy: Policy::default(),
            // About 170k events at 256 per batch: the ring covers the
            // burst deficit measured in the spike (17k events/s for 10 s).
            ring_batches: 700,
            spill_dir,
            hooks: Hooks::default(),
        }
    }
}

/// The outcome of [`Producer::push`].
#[derive(Debug)]
pub enum Push {
    /// Queued for the writer. An empty batch to refill comes back.
    Queued(Batch),
    /// The ring was full or a spill is active: the batch goes through the
    /// spill file. An empty batch to refill comes back.
    Overflow(Batch),
    /// The pipeline is closed; the batch comes back untouched.
    Closed(Batch),
}

/// Counters of the pipeline.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    /// Rows committed.
    pub rows: u64,
    /// Commits.
    pub commits: u64,
    /// Batches that went through the spill file.
    pub spilled_batches: u64,
    /// Batches dropped because they could not be written anywhere.
    pub lost_batches: u64,
    /// Highest ring depth.
    pub ring_high_water: usize,
}

#[derive(Default)]
struct State {
    queue: VecDeque<Batch>,
    overflow: VecDeque<Batch>,
    free: Vec<Batch>,
    cursors: Vec<Cursor>,
    spilling: bool,
    spiller_busy: bool,
    closed: bool,
    pushed: u64,
    done: u64,
    high_water: usize,
}

struct Shared {
    state: Mutex<State>,
    /// Wakes the writer: a batch, a cursor, a close.
    writer_wake: Condvar,
    /// Wakes the spiller: an overflow batch, a close.
    spiller_wake: Condvar,
    /// Signalled when `done` advances.
    progress: Condvar,
    spill: Mutex<Spill>,
    capacity: usize,
    /// Size of the spill file found at start: records below it were pushed by
    /// a previous run and are not counted in `pushed`.
    old_end: u64,
    rows: AtomicU64,
    commits: AtomicU64,
    spilled: AtomicU64,
    lost: AtomicU64,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Capture's handle: cheap to clone, never blocks on disk.
#[derive(Clone)]
pub struct Producer {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for Producer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Producer").finish_non_exhaustive()
    }
}

impl Producer {
    /// An empty batch, recycled when one is available.
    #[must_use]
    pub fn new_batch(&self) -> Batch {
        lock(&self.shared.state)
            .free
            .pop()
            .unwrap_or_else(Batch::standard)
    }

    /// Hands a filled batch to the writer. Never waits for the disk.
    pub fn push(&self, batch: Batch) -> Push {
        let mut state = lock(&self.shared.state);
        if state.closed {
            return Push::Closed(batch);
        }
        state.pushed += 1;
        let spare = state.free.pop().unwrap_or_else(Batch::standard);
        if !state.spilling && state.queue.len() < self.shared.capacity {
            state.queue.push_back(batch);
            let depth = state.queue.len();
            state.high_water = state.high_water.max(depth);
            drop(state);
            self.shared.writer_wake.notify_one();
            return Push::Queued(spare);
        }
        state.spilling = true;
        state.overflow.push_back(batch);
        drop(state);
        self.shared.spiller_wake.notify_one();
        Push::Overflow(spare)
    }

    /// Waits until everything pushed so far (by anyone) is committed or
    /// counted lost, at most `deadline`: the same wait as
    /// [`Pipeline::flush`], for producers that do not own the pipeline.
    ///
    /// # Errors
    ///
    /// [`FlushTimeout`] when the deadline passed first.
    pub fn flush(&self, deadline: Duration) -> Result<Flushed, FlushTimeout> {
        flush_shared(&self.shared, deadline)
    }

    /// Batches lost since the pipeline started.
    #[must_use]
    pub fn lost_batches(&self) -> u64 {
        self.shared.lost.load(Ordering::Relaxed)
    }

    /// Queues a capture cursor to be written with the next commit.
    pub fn set_cursor(&self, cursor: Cursor) {
        let mut state = lock(&self.shared.state);
        state.cursors.retain(|queued| queued.root != cursor.root);
        state.cursors.push(cursor);
        drop(state);
        self.shared.writer_wake.notify_one();
    }
}

fn flush_shared(shared: &Shared, deadline: Duration) -> Result<Flushed, FlushTimeout> {
    let until = Instant::now() + deadline;
    let mut state = lock(&shared.state);
    let target = state.pushed;
    while state.done < target {
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(FlushTimeout);
        }
        state = shared
            .progress
            .wait_timeout(state, left)
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .0;
    }
    drop(state);
    Ok(Flushed {
        rows: shared.rows.load(Ordering::Relaxed),
        lost_batches: shared.lost.load(Ordering::Relaxed),
    })
}

/// What a flush reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Flushed {
    /// Rows committed in total so far.
    pub rows: u64,
    /// Batches lost since the pipeline started.
    pub lost_batches: u64,
}

/// A flush that did not finish in time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlushTimeout;

/// The running pipeline.
pub struct Pipeline {
    shared: Arc<Shared>,
    writer: Option<JoinHandle<Writer>>,
    spiller: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for Pipeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pipeline").finish_non_exhaustive()
    }
}

fn now_ns(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

impl Pipeline {
    /// Starts the threads around `writer`. A spill file left by a previous
    /// run is replayed before new batches are accepted into the ring.
    ///
    /// # Errors
    ///
    /// The I/O error from opening the spill directory or spawning a thread.
    pub fn start(writer: Writer, config: Config) -> std::io::Result<Self> {
        let (generation, consumed) = writer.spill_position();
        let spill = Spill::open(&config.spill_dir, generation, consumed)?;
        let old_end = spill.end();
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                spilling: spill.has_pending(),
                ..State::default()
            }),
            writer_wake: Condvar::new(),
            spiller_wake: Condvar::new(),
            progress: Condvar::new(),
            spill: Mutex::new(spill),
            capacity: config.ring_batches.max(1),
            old_end,
            rows: AtomicU64::new(0),
            commits: AtomicU64::new(0),
            spilled: AtomicU64::new(0),
            lost: AtomicU64::new(0),
        });
        let (for_writer, for_spiller) = (Arc::clone(&shared), Arc::clone(&shared));
        let spiller = std::thread::Builder::new()
            .name(String::from("spill"))
            .spawn(move || spill_loop(&for_spiller))?;
        let writer = std::thread::Builder::new()
            .name(String::from("writer"))
            .spawn(move || write_loop(writer, &for_writer, &config.policy, &config.hooks))?;
        Ok(Self {
            shared,
            writer: Some(writer),
            spiller: Some(spiller),
        })
    }

    /// A producer handle for capture.
    #[must_use]
    pub fn producer(&self) -> Producer {
        Producer {
            shared: Arc::clone(&self.shared),
        }
    }

    /// Counters.
    #[must_use]
    pub fn stats(&self) -> Stats {
        Stats {
            rows: self.shared.rows.load(Ordering::Relaxed),
            commits: self.shared.commits.load(Ordering::Relaxed),
            spilled_batches: self.shared.spilled.load(Ordering::Relaxed),
            lost_batches: self.shared.lost.load(Ordering::Relaxed),
            ring_high_water: lock(&self.shared.state).high_water,
        }
    }

    /// Waits until everything pushed so far is committed (or counted lost),
    /// at most `deadline`. The engine calls it on quit and `WM_ENDSESSION`
    /// (5 s, F19) before the final checkpoint.
    ///
    /// # Errors
    ///
    /// [`FlushTimeout`] when the deadline passed first; the data that was not
    /// written stays in the ring or the spill file.
    pub fn flush(&self, deadline: Duration) -> Result<Flushed, FlushTimeout> {
        flush_shared(&self.shared, deadline)
    }

    /// Stops accepting batches, lets the writer commit what is queued, saves
    /// overflow to the spill file and returns the writer for the final
    /// checkpoint.
    #[must_use]
    pub fn close(mut self) -> Option<Writer> {
        lock(&self.shared.state).closed = true;
        self.shared.writer_wake.notify_all();
        self.shared.spiller_wake.notify_all();
        let writer = self.writer.take().and_then(|thread| thread.join().ok());
        if let Some(thread) = self.spiller.take() {
            let _ = thread.join();
        }
        writer
    }
}

/// Moves overflow batches to the spill file.
fn spill_loop(shared: &Shared) {
    loop {
        let mut state = lock(&shared.state);
        while state.overflow.is_empty() && !state.closed {
            state = shared
                .spiller_wake
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        let Some(mut batch) = state.overflow.pop_front() else {
            return;
        };
        state.spiller_busy = true;
        drop(state);

        let written = lock(&shared.spill).append(&batch);
        match written {
            Ok(()) => {
                shared.spilled.fetch_add(1, Ordering::Relaxed);
            }
            Err(_) => {
                shared.lost.fetch_add(1, Ordering::Relaxed);
            }
        }
        batch.clear();
        let mut state = lock(&shared.state);
        state.spiller_busy = false;
        if written.is_err() {
            // A lost batch counts as done, so a flush cannot hang on it.
            state.done += 1;
            shared.progress.notify_all();
        }
        if state.free.len() < shared.capacity {
            state.free.push(batch);
        }
        shared.writer_wake.notify_one();
    }
}

enum Work {
    /// Batches and cursors to commit; `from_spill` marks a replayed record and
    /// `counted` whether it was pushed (and counted) by this run.
    Group {
        batches: Vec<Batch>,
        cursors: Vec<Cursor>,
        from_spill: bool,
        counted: bool,
        /// Generation and end offset of a replayed record.
        spill_end: Option<(u32, u64)>,
    },
    CursorsOnly(Vec<Cursor>),
    /// The writer has been idle since the last commit: checkpoint once.
    Checkpoint,
    Stop,
}

/// How long after the last commit an idle writer runs its one checkpoint.
const IDLE_CHECKPOINT: Duration = Duration::from_secs(1);

enum Step {
    Work(Work),
    /// The spill state changed; look at the queue again.
    Again,
    /// Nothing to replay now (closing with records left for the next start).
    Skip,
}

/// Replays the next spill record, or finishes the file when it is empty.
fn replay_step(
    shared: &Shared,
    replay: &mut Batch,
    pending: &mut Pending,
    clock: Instant,
    closed: bool,
) -> Step {
    let mut spill = lock(&shared.spill);
    if closed && spill.has_pending() {
        // Quitting: the rest stays in the file for the next start.
        return Step::Skip;
    }
    let counted = spill.position() >= shared.old_end;
    let generation = spill.generation();
    if let Ok(Some(end)) = spill.next(replay) {
        drop(spill);
        let mut batch = Batch::standard();
        std::mem::swap(&mut batch, replay);
        pending.add(now_ns(clock), batch.len());
        let cursors = std::mem::take(&mut lock(&shared.state).cursors);
        return Step::Work(Work::Group {
            batches: vec![batch],
            cursors,
            from_spill: true,
            counted,
            spill_end: Some((generation, end)),
        });
    }
    // Everything replayed (or the rest was a torn tail): the file is done.
    let _ = spill.finish();
    drop(spill);
    let mut state = lock(&shared.state);
    if state.queue.is_empty() && state.overflow.is_empty() && !state.spiller_busy {
        state.spilling = false;
    }
    Step::Again
}

/// Waits for the next unit of work and, for a queue batch, collects the group
/// the policy says belongs to one commit.
fn next_work(
    shared: &Shared,
    policy: &Policy,
    pending: &mut Pending,
    clock: Instant,
    replay: &mut Batch,
    dirty_since: Option<Instant>,
) -> Work {
    let mut group: Vec<Batch> = Vec::new();
    let mut state = lock(&shared.state);
    loop {
        if let Some(batch) = state.queue.pop_front() {
            pending.add(now_ns(clock), batch.len());
            group.push(batch);
            // Linger from the first event, take whatever is queued.
            loop {
                if pending.due(now_ns(clock), policy) {
                    break;
                }
                if let Some(batch) = state.queue.pop_front() {
                    pending.add(now_ns(clock), batch.len());
                    group.push(batch);
                    continue;
                }
                if state.closed {
                    break;
                }
                let Some(wait) = pending.wait_ns(now_ns(clock), policy) else {
                    break;
                };
                state = shared
                    .writer_wake
                    .wait_timeout(state, Duration::from_nanos(wait))
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .0;
            }
            let cursors = std::mem::take(&mut state.cursors);
            return Work::Group {
                batches: group,
                cursors,
                from_spill: false,
                counted: true,
                spill_end: None,
            };
        }
        // Nothing queued. Older data in the spill file comes next, but only
        // once nothing newer is still on its way to the file.
        if state.spilling && state.overflow.is_empty() && !state.spiller_busy {
            let closed = state.closed;
            drop(state);
            match replay_step(shared, replay, pending, clock, closed) {
                Step::Work(work) => return work,
                Step::Again => {
                    state = lock(&shared.state);
                    continue;
                }
                Step::Skip => state = lock(&shared.state),
            }
        }
        if !state.cursors.is_empty() {
            return Work::CursorsOnly(std::mem::take(&mut state.cursors));
        }
        if state.closed && state.overflow.is_empty() && !state.spiller_busy {
            return Work::Stop;
        }
        // Idle. With nothing committed since the last checkpoint there is no
        // deadline and no timer at all: wait for a push. Right after a
        // commit one one-shot wait ends the burst with a checkpoint.
        if let Some(since) = dirty_since {
            let left = IDLE_CHECKPOINT.saturating_sub(since.elapsed());
            if left.is_zero() {
                return Work::Checkpoint;
            }
            state = shared
                .writer_wake
                .wait_timeout(state, left)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        } else {
            state = shared
                .writer_wake
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }
}

fn write_loop(mut writer: Writer, shared: &Shared, policy: &Policy, hooks: &Hooks) -> Writer {
    let clock = Instant::now();
    let mut pending = Pending::new();
    let mut replay = Batch::standard();
    let mut dirty_since: Option<Instant> = None;
    loop {
        match next_work(
            shared,
            policy,
            &mut pending,
            clock,
            &mut replay,
            dirty_since,
        ) {
            Work::Stop => return writer,
            Work::Checkpoint => {
                let _ = writer.checkpoint_passive();
                dirty_since = None;
            }
            Work::CursorsOnly(cursors) => {
                let _ = writer.write(&[], &cursors);
            }
            Work::Group {
                mut batches,
                cursors,
                from_spill,
                counted,
                spill_end,
            } => {
                pending.clear();
                let target = GroupTarget {
                    cursors: &cursors,
                    from_spill,
                    counted,
                    spill_end,
                };
                commit_group(&mut writer, shared, hooks, &mut batches, &target);
                dirty_since.get_or_insert_with(Instant::now);
                // Between batches under sustained churn: size-triggered.
                if writer.wal_bytes() >= WAL_CHECKPOINT_BYTES {
                    let _ = writer.checkpoint_passive();
                    dirty_since = None;
                }
            }
        }
    }
}

/// What a commit carries besides the batches.
#[derive(Clone, Copy)]
struct GroupTarget<'a> {
    cursors: &'a [Cursor],
    from_spill: bool,
    counted: bool,
    spill_end: Option<(u32, u64)>,
}

/// Writes one group with retries, then recycles its batches.
fn commit_group(
    writer: &mut Writer,
    shared: &Shared,
    hooks: &Hooks,
    group: &mut Vec<Batch>,
    target: &GroupTarget<'_>,
) {
    let GroupTarget {
        cursors,
        from_spill,
        counted,
        spill_end,
    } = *target;
    let mut outcome: Option<Commit> = None;
    for attempt in 1..=COMMIT_ATTEMPTS {
        if let Some(before) = &hooks.before_commit {
            before();
        }
        match writer.write_with_spill(group, cursors, spill_end) {
            Ok(commit) => {
                outcome = Some(commit);
                break;
            }
            Err(WriteError::UnknownRoot(_)) => break,
            Err(_) if attempt < COMMIT_ATTEMPTS => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => {}
        }
    }
    let count = group.len() as u64;
    let mut state = lock(&shared.state);
    match outcome {
        Some(commit) => {
            shared.rows.fetch_add(commit.rows, Ordering::Relaxed);
            shared.commits.fetch_add(1, Ordering::Relaxed);
            if let Some(observer) = &hooks.on_commit {
                observer(&CommitInfo {
                    commit,
                    batches: group.len(),
                    ring_depth: state.queue.len(),
                    ring_high_water: state.high_water,
                    from_spill,
                });
            }
        }
        None => {
            shared.lost.fetch_add(count, Ordering::Relaxed);
        }
    }
    if counted {
        state.done += count;
    }
    for mut batch in group.drain(..) {
        batch.clear();
        if state.free.len() < shared.capacity {
            state.free.push(batch);
        }
    }
    drop(state);
    shared.progress.notify_all();
}
