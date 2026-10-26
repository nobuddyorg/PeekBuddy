//! The `ReadDirectoryChangesExW` capture backend (#45).
//!
//! One thread serves every root. It blocks in `GetQueuedCompletionStatusEx`
//! with no timeout, so an idle engine has no wakeups from it; control
//! messages (add or remove a root, stop) arrive as posted completions.
//!
//! # A completion
//!
//! 1. The root is re-armed first, with a spare buffer of the same size. The
//!    completed buffer is parsed afterwards and becomes the new spare, so `N`
//!    roots need `N + 1` buffers per size class.
//! 2. The buffer goes to [`Decoder`], which parses it (#44), maps records to
//!    events (#38), drops the data directory's own churn and offers each event
//!    to the [`EventSink`] (filters, the ring to the writer).
//! 3. An overflow (`TRUE` with 0 bytes, or `ERROR_NOTIFY_ENUM_DIR`), a full
//!    ring or a malformed buffer owes a gap row and asks the
//!    [`LifecycleSink`] for a reconcile of the root. The root stays armed.
//!
//! Any other failure (the root was deleted, the volume went away) ends the
//! watch with a `Lost` state through the [`LifecycleSink`]: a watch never
//! dies silently.
//!
//! # Info classes
//!
//! A root starts with the full class, falls back to the extended one and then
//! to plain `ReadDirectoryChangesW` when the file system refuses a class. The
//! chosen class is in [`RootInfo`]. [`RootSpec::reject`] makes the fallback
//! testable without a FAT volume.
//!
//! # Stopping
//!
//! `CancelIoEx`, then wait for the completion: records that arrive on the way
//! are still delivered, and the handle is closed only after the kernel has
//! given the buffer back.

use std::io;
use std::os::windows::io::AsRawHandle;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::Instant;

use peekbuddy_core::capture::{Counters, Decoder, EventSink, Outcome, Stamp, relative_prefix};
use peekbuddy_core::event::{GapReason, RootIdx, VolIdx, WatchState};
use peekbuddy_core::notify::Class;
use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::Storage::FileSystem::{
    ReadDirectoryChangesExW, ReadDirectoryChangesW, ReadDirectoryNotifyExtendedInformation,
    ReadDirectoryNotifyFullInformation,
};
use windows_sys::Win32::System::SystemInformation::GetSystemTimePreciseAsFileTime;

use crate::io::dir::DirHandle;
use crate::io::error::WinError;
use crate::io::iocp::{Batch, Completion, Iocp};
use crate::io::open::OpenOptions;
use crate::io::path::{PathClass, classify, to_long_path};
use crate::io::slot::IoSlot;
use crate::io::volume::final_path;

/// `FILE_NOTIFY_CHANGE_FILE_NAME | DIR_NAME | ATTRIBUTES | SIZE | LAST_WRITE |
/// CREATION | SECURITY`.
pub const NOTIFY_FILTER: u32 = 0x1 | 0x2 | 0x4 | 0x8 | 0x10 | 0x40 | 0x100;
/// `FILE_NOTIFY_CHANGE_LAST_ACCESS`: noise, so opt-in per watch.
pub const NOTIFY_LAST_ACCESS: u32 = 0x20;
/// The most a network path accepts: more fails with `ERROR_INVALID_PARAMETER`.
pub const REMOTE_BUFFER_BYTES: usize = 64 * 1024;

/// The key of posted control messages; roots use their index.
const CONTROL_KEY: usize = usize::MAX;
/// Completions taken per dequeue.
const BATCH: usize = 64;

/// One root to watch.
#[derive(Debug, Clone)]
pub struct RootSpec {
    /// The root's index in the settings.
    pub root: RootIdx,
    /// The volume's index in the engine's table.
    pub vol: VolIdx,
    /// The absolute path of the directory.
    pub path: Vec<u16>,
    /// Whether subdirectories are watched too.
    pub recursive: bool,
    /// Whether `LAST_ACCESS` changes are reported.
    pub last_access: bool,
    /// The notification buffer, in bytes (a multiple of 4). Capped at
    /// [`REMOTE_BUFFER_BYTES`] on a network path. The kernel fixes the size
    /// at the first call, so another size means another watch.
    pub buffer_bytes: usize,
    /// Canonical (`GetFinalPathNameByHandleW`) paths whose events are dropped
    /// before the ring: the data and export directories. Only those under
    /// this root matter.
    pub exclude: Vec<Vec<u16>>,
    /// Information classes to treat as refused: a test hook for the fallback.
    pub reject: Vec<Class>,
}

/// What [`Watcher::add_root`] set up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RootInfo {
    /// The information class the file system accepted.
    pub class: Class,
    /// The buffer size in use.
    pub buffer_bytes: usize,
}

/// Why a state was reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Detail {
    /// The information class, once armed.
    pub class: Option<Class>,
    /// The error that ended the watch.
    pub error: Option<WinError>,
}

/// Where lifecycle changes go; the state machine of #48 implements it.
pub trait LifecycleSink: Send + 'static {
    /// A watch changed state (`Arming`, `Live`, `Lost`).
    fn state(&mut self, root: RootIdx, state: WatchState, detail: Detail);
    /// The root needs a reconcile walk (#46) because events were lost.
    fn reconcile(&mut self, root: RootIdx, reason: GapReason);
}

/// What the backend counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Metrics {
    /// The decoders' counters, summed over all roots ever watched.
    pub counters: Counters,
    /// Reconciles requested.
    pub reconciles: u64,
    /// The slowest re-arm, from dequeue to the next call, in microseconds.
    pub rearm_max_us: u64,
}

#[derive(Default)]
struct Totals {
    batches: AtomicU64,
    records: AtomicU64,
    excluded: AtomicU64,
    dropped_full: AtomicU64,
    overflows: AtomicU64,
    parse_errors: AtomicU64,
    gap_rows: AtomicU64,
    gaps_owed_total: AtomicU64,
    reconciles: AtomicU64,
    rearm_max_ns: AtomicU64,
}

impl Totals {
    fn add(&self, d: Counters) {
        let add = |counter: &AtomicU64, n: u64| {
            if n > 0 {
                counter.fetch_add(n, Ordering::Relaxed);
            }
        };
        add(&self.batches, d.batches);
        add(&self.records, d.records);
        add(&self.excluded, d.excluded);
        add(&self.dropped_full, d.dropped_full);
        add(&self.overflows, d.overflows);
        add(&self.parse_errors, d.parse_errors);
        add(&self.gap_rows, d.gap_rows);
        add(&self.gaps_owed_total, d.gaps_owed_total);
    }

    fn load(&self) -> Metrics {
        let get = |counter: &AtomicU64| counter.load(Ordering::Relaxed);
        Metrics {
            counters: Counters {
                batches: get(&self.batches),
                records: get(&self.records),
                excluded: get(&self.excluded),
                dropped_full: get(&self.dropped_full),
                overflows: get(&self.overflows),
                parse_errors: get(&self.parse_errors),
                gap_rows: get(&self.gap_rows),
                gaps_owed_total: get(&self.gaps_owed_total),
            },
            reconciles: get(&self.reconciles),
            rearm_max_us: get(&self.rearm_max_ns) / 1_000,
        }
    }
}

enum Control {
    Add(RootSpec, Sender<io::Result<RootInfo>>),
    Remove(RootIdx),
    Stop,
}

struct Shared {
    iocp: Iocp,
    queue: Mutex<Vec<Control>>,
    totals: Totals,
}

impl Shared {
    fn send(&self, control: Control) -> io::Result<()> {
        self.queue
            .lock()
            .map_err(|_| io::Error::other("the control queue is poisoned"))?
            .push(control);
        self.iocp.post(CONTROL_KEY, 0, 0)
    }
}

/// The backend: a thread and its roots. Dropping it stops and joins.
#[derive(Debug)]
pub struct Watcher {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shared").finish_non_exhaustive()
    }
}

impl Watcher {
    /// Starts the thread; roots are added with [`Watcher::add_root`].
    ///
    /// # Errors
    ///
    /// The OS error when the port or the thread cannot be created.
    pub fn start(
        sink: impl EventSink + Send + 'static,
        life: impl LifecycleSink,
    ) -> io::Result<Self> {
        let shared = Arc::new(Shared {
            iocp: Iocp::new(1)?,
            queue: Mutex::new(Vec::new()),
            totals: Totals::default(),
        });
        let inner = Arc::clone(&shared);
        // The worker is built on its own thread: its buffers hold raw
        // pointers, so they never cross threads.
        let thread = std::thread::Builder::new()
            .name(String::from("peekbuddy-rdcw"))
            .spawn(move || {
                Worker {
                    shared: inner,
                    sink,
                    life,
                    roots: Vec::new(),
                    spares: Vec::new(),
                    stopping: false,
                }
                .run();
            })?;
        Ok(Self {
            shared,
            thread: Some(thread),
        })
    }

    /// Arms a watch on `spec` and waits until it is live.
    ///
    /// # Errors
    ///
    /// The OS error when the directory cannot be opened or watched.
    pub fn add_root(&self, spec: RootSpec) -> io::Result<RootInfo> {
        let (reply, answer) = channel();
        self.shared.send(Control::Add(spec, reply))?;
        answer
            .recv()
            .map_err(|_| io::Error::other("the capture thread ended"))?
    }

    /// Stops watching `root`; its pending records are still delivered.
    ///
    /// # Errors
    ///
    /// The OS error when the request cannot be queued.
    pub fn remove_root(&self, root: RootIdx) -> io::Result<()> {
        self.shared.send(Control::Remove(root))
    }

    /// The counters so far.
    #[must_use]
    pub fn metrics(&self) -> Metrics {
        self.shared.totals.load()
    }

    /// Cancels every watch, waits for the completions, joins the thread and
    /// returns the final counters.
    #[must_use]
    pub fn stop(mut self) -> Metrics {
        self.shutdown();
        self.metrics()
    }

    fn shutdown(&mut self) {
        let Some(thread) = self.thread.take() else {
            return;
        };
        if self.shared.send(Control::Stop).is_ok() {
            let _ = thread.join();
        }
        // If the message could not be queued the thread keeps running: it
        // owns pending I/O, and detaching is safer than freeing under it.
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.shutdown();
    }
}

struct Root {
    id: RootIdx,
    dir: DirHandle,
    slot: IoSlot,
    class: Class,
    recursive: bool,
    filter: u32,
    decoder: Decoder,
    seen: Counters,
    stopping: bool,
}

struct Worker<S, L> {
    shared: Arc<Shared>,
    sink: S,
    life: L,
    roots: Vec<Option<Root>>,
    /// Idle buffers; one per size class is enough.
    spares: Vec<IoSlot>,
    stopping: bool,
}

impl<S: EventSink, L: LifecycleSink> Worker<S, L> {
    fn run(&mut self) {
        let mut batch = Batch::new(BATCH);
        loop {
            if self.stopping && self.roots.iter().all(Option::is_none) {
                break;
            }
            if self.shared.iocp.dequeue(&mut batch, None).is_err() {
                break;
            }
            let stamp = now();
            for completion in batch.iter() {
                if completion.key == CONTROL_KEY {
                    self.control();
                } else {
                    self.completed(completion, stamp);
                }
            }
        }
    }

    fn control(&mut self) {
        let pending = match self.shared.queue.lock() {
            Ok(mut queue) => std::mem::take(&mut *queue),
            Err(_) => return,
        };
        for control in pending {
            match control {
                Control::Add(spec, reply) => {
                    let result = if self.stopping {
                        Err(io::Error::other("the backend is stopping"))
                    } else {
                        self.add(&spec)
                    };
                    let _ = reply.send(result);
                }
                Control::Remove(id) => {
                    let index = self
                        .roots
                        .iter()
                        .position(|root| root.as_ref().is_some_and(|r| r.id == id));
                    if let Some(index) = index {
                        self.begin_stop(index);
                    }
                }
                Control::Stop => {
                    self.stopping = true;
                    for index in 0..self.roots.len() {
                        self.begin_stop(index);
                    }
                }
            }
        }
    }

    /// Cancels a root's pending read; the completion finishes it.
    fn begin_stop(&mut self, index: usize) {
        let Some(root) = self.roots.get_mut(index).and_then(Option::as_mut) else {
            return;
        };
        root.stopping = true;
        if root.dir.cancel(&root.slot).is_err() {
            // Nothing sensible is left to wait for.
            self.remove(index);
        }
    }

    fn remove(&mut self, index: usize) {
        if let Some(place) = self.roots.get_mut(index) {
            *place = None;
        }
    }

    fn add(&mut self, spec: &RootSpec) -> io::Result<RootInfo> {
        let long = to_long_path(&spec.path).map_err(io::Error::other)?;
        let remote = classify(&spec.path) == PathClass::Remote;
        let bytes = if remote {
            spec.buffer_bytes.min(REMOTE_BUFFER_BYTES)
        } else {
            spec.buffer_bytes
        };
        let dir = DirHandle::open(&long, OpenOptions::default())?;
        let canonical = final_path(&dir)?;
        let excluded: Vec<Vec<u16>> = spec
            .exclude
            .iter()
            .filter_map(|other| relative_prefix(&canonical, other))
            .collect();
        let mut slot = IoSlot::new(bytes).map_err(io::Error::other)?;
        let index = self
            .roots
            .iter()
            .position(Option::is_none)
            .unwrap_or(self.roots.len());
        self.shared.iocp.associate(&dir, index)?;
        self.life
            .state(spec.root, WatchState::Arming, Detail::default());
        let filter = NOTIFY_FILTER
            | if spec.last_access {
                NOTIFY_LAST_ACCESS
            } else {
                0
            };
        let class = arm_with_fallback(&dir, &mut slot, spec.recursive, filter, &spec.reject)?;
        if !self.spares.iter().any(|spare| spare.len() == bytes) {
            self.spares
                .push(IoSlot::new(bytes).map_err(io::Error::other)?);
        }
        let root = Root {
            id: spec.root,
            dir,
            slot,
            class,
            recursive: spec.recursive,
            filter,
            decoder: Decoder::new(spec.root, spec.vol, class, &excluded),
            seen: Counters::default(),
            stopping: false,
        };
        if index == self.roots.len() {
            self.roots.push(Some(root));
        } else if let Some(place) = self.roots.get_mut(index) {
            *place = Some(root);
        }
        self.life.state(
            spec.root,
            WatchState::Live,
            Detail {
                class: Some(class),
                error: None,
            },
        );
        Ok(RootInfo {
            class,
            buffer_bytes: bytes,
        })
    }

    fn completed(&mut self, completion: Completion, stamp: Stamp) {
        let index = completion.key;
        let Some(root) = self.roots.get_mut(index).and_then(Option::as_mut) else {
            return;
        };
        if completion.overlapped != root.slot.overlapped_addr() {
            // A completion of a buffer this root no longer waits on.
            return;
        }
        let _ = root.slot.finish();
        let bytes = usize::try_from(completion.bytes).unwrap_or(0);
        let status = completion.status;
        let mut ended = None;
        let mut outcome = Outcome::default();
        if completion.is_success() && bytes > 0 {
            (outcome, ended) = deliver(
                root,
                &mut self.spares,
                &mut self.sink,
                &self.shared,
                bytes,
                stamp,
            );
        } else if completion.is_success() || status == WinError::NotifyEnumDir {
            let reason = if completion.is_success() {
                GapReason::BufferOverflow
            } else {
                GapReason::NotifyEnumDir
            };
            if !root.stopping {
                ended = rearm(root, &self.shared, stamp).err();
            }
            outcome = root.decoder.overflow(reason, stamp, &mut self.sink);
        } else if !root.stopping {
            // Access denied (the root was deleted), cleanup, a vanished
            // volume, or a cancel nobody asked for.
            ended = Some(status);
        }
        publish(root, &self.shared);
        if let Some(reason) = outcome.reconcile {
            self.shared
                .totals
                .reconciles
                .fetch_add(1, Ordering::Relaxed);
            self.life.reconcile(root.id, reason);
        }
        if let Some(error) = ended {
            let id = root.id;
            self.remove(index);
            self.life.state(
                id,
                WatchState::Lost,
                Detail {
                    class: None,
                    error: Some(error),
                },
            );
        } else if root.stopping {
            // Delivered what was in flight; let go of the handle.
            root.decoder.flush_owed(stamp, &mut self.sink);
            publish(root, &self.shared);
            self.remove(index);
        }
    }
}

/// Re-arms `root` with a spare buffer, then decodes the completed one.
fn deliver(
    root: &mut Root,
    spares: &mut Vec<IoSlot>,
    sink: &mut impl EventSink,
    shared: &Shared,
    bytes: usize,
    stamp: Stamp,
) -> (Outcome, Option<WinError>) {
    if root.stopping {
        let data = root.slot.data(bytes).unwrap_or_default();
        return (root.decoder.decode(data, bytes, stamp, sink), None);
    }
    let want = root.slot.len();
    let spare = match spares.iter().position(|spare| spare.len() == want) {
        Some(at) => Some(spares.swap_remove(at)),
        None => IoSlot::new(want).ok(),
    };
    let Some(mut done) = spare else {
        // No memory for a second buffer: decode in place, then re-arm.
        let outcome = {
            let data = root.slot.data(bytes).unwrap_or_default();
            root.decoder.decode(data, bytes, stamp, sink)
        };
        return (outcome, rearm(root, shared, stamp).err());
    };
    std::mem::swap(&mut root.slot, &mut done);
    // `done` holds the completed data; `root.slot` is the spare, now armed.
    let armed = rearm(root, shared, stamp);
    let outcome = {
        let data = done.data(bytes).unwrap_or_default();
        root.decoder.decode(data, bytes, stamp, sink)
    };
    spares.push(done);
    // A watch that cannot be re-armed is over; the records are delivered.
    (outcome, armed.err())
}

/// Issues the next read on `root.slot`, recording the latency since `stamp`.
fn rearm(root: &mut Root, shared: &Shared, stamp: Stamp) -> Result<(), WinError> {
    let result = issue(
        &root.dir,
        &mut root.slot,
        root.recursive,
        root.filter,
        root.class,
    );
    shared
        .totals
        .rearm_max_ns
        .fetch_max(monotonic_ns().saturating_sub(stamp.qpc), Ordering::Relaxed);
    result.map_err(|error| WinError::from_io(&error).unwrap_or(WinError::Other(0)))
}

/// Adds what the decoder counted since the last call to the totals.
fn publish(root: &mut Root, shared: &Shared) {
    let now = root.decoder.counters();
    shared.totals.add(now.since(root.seen));
    root.seen = now;
}

/// Arms the first read, trying the full class, then the extended one, then
/// the plain call, skipping `reject`ed classes.
fn arm_with_fallback(
    dir: &DirHandle,
    slot: &mut IoSlot,
    recursive: bool,
    filter: u32,
    reject: &[Class],
) -> io::Result<Class> {
    let mut last = io::Error::from(io::ErrorKind::Unsupported);
    for class in [Class::Full, Class::Extended, Class::Plain] {
        if reject.contains(&class) {
            continue;
        }
        match issue(dir, slot, recursive, filter, class) {
            Ok(()) => return Ok(class),
            Err(error) => {
                // Only a refusal of the class falls through; a bad path or a
                // dead volume would fail the same way for every class.
                let refused = matches!(
                    WinError::from_io(&error),
                    Some(WinError::InvalidParameter | WinError::Other(1 | 50))
                );
                last = error;
                if !refused {
                    break;
                }
            }
        }
    }
    Err(last)
}

/// Starts one read on `slot`.
fn issue(
    dir: &DirHandle,
    slot: &mut IoSlot,
    recursive: bool,
    filter: u32,
    class: Class,
) -> io::Result<()> {
    let p = slot.begin().map_err(io::Error::other)?;
    let subtree = i32::from(recursive);
    let ok = match class {
        Class::Full => {
            // SAFETY: the pointers come from `begin` and stay valid until the
            // slot is finished; the handle is open for overlapped listing.
            unsafe {
                ReadDirectoryChangesExW(
                    dir.as_raw_handle(),
                    p.buffer.cast(),
                    p.len,
                    subtree,
                    filter,
                    std::ptr::null_mut(),
                    p.overlapped,
                    None,
                    ReadDirectoryNotifyFullInformation,
                )
            }
        }
        Class::Extended => {
            // SAFETY: as above.
            unsafe {
                ReadDirectoryChangesExW(
                    dir.as_raw_handle(),
                    p.buffer.cast(),
                    p.len,
                    subtree,
                    filter,
                    std::ptr::null_mut(),
                    p.overlapped,
                    None,
                    ReadDirectoryNotifyExtendedInformation,
                )
            }
        }
        Class::Plain => {
            // SAFETY: as above.
            unsafe {
                ReadDirectoryChangesW(
                    dir.as_raw_handle(),
                    p.buffer.cast(),
                    p.len,
                    subtree,
                    filter,
                    std::ptr::null_mut(),
                    p.overlapped,
                    None,
                )
            }
        }
    };
    if ok == 0 {
        let error = io::Error::last_os_error();
        let _ = slot.finish();
        return Err(error);
    }
    Ok(())
}

/// Monotonic nanoseconds since the first reading. `Instant` is the
/// performance counter underneath; the count is only used for ordering and
/// latency inside the process.
fn monotonic_ns() -> u64 {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    u64::try_from(EPOCH.get_or_init(Instant::now).elapsed().as_nanos()).unwrap_or(u64::MAX)
}

/// One reading of both clocks.
fn now() -> Stamp {
    let mut time = FILETIME::default();
    // SAFETY: `time` is a valid out pointer.
    unsafe { GetSystemTimePreciseAsFileTime(&raw mut time) };
    Stamp {
        ts: (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime),
        qpc: monotonic_ns(),
    }
}
