//! The capture host (#48): the watcher, the reconciler and one lifecycle
//! machine per root, joined.
//!
//! Every outside event becomes a [`Signal`] on a queue: a completion status
//! from the watcher (through [`LifecycleSink`]), the end of a reconcile, a
//! pause, a plug-and-play or power message. One worker thread takes the signals in
//! order, feeds the root's [`Machine`], writes the transition as a row and
//! carries out the actions, so no callback ever blocks (a plug-and-play or power
//! callback must return at once) and the machines need no locking against each
//! other.
//!
//! A watch that is lost because its root was deleted or moved is watched from
//! its nearest existing ancestor, so coming back is an event, not polling.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::Duration;

use peekbuddy_core::capture::EventSink;
use peekbuddy_core::event::{Event, GapReason, Kind, RootIdx, VolIdx, WatchState};
use peekbuddy_core::filter::{FilterSet, Scratch};
use peekbuddy_core::lifecycle::{Action, Machine, Signal, State};
use peekbuddy_core::name::{WideStr, fold_upper};
use peekbuddy_core::pause::{PausableSink, Pause};
use peekbuddy_core::perf::Counters;
use peekbuddy_store::batch::Batch;
use peekbuddy_store::pipeline::{Producer, Push};
use peekbuddy_win::io::error::WinError;
use peekbuddy_win::probe::{Probe, probe_root};
use peekbuddy_win::rdcw::{Detail, LifecycleSink, RootSpec as RdcwRoot, Watcher};

use crate::pause_ctl::{Clock, RowSink};
use crate::reconciler::{Observer, Reconciler};

/// Waiter watches (of a lost root) use root indexes with this bit.
const WAIT_BIT: u16 = 0x8000;
/// Guard watches (of a live root's parent) use root indexes with this bit.
const GUARD_BIT: u16 = 0x4000;
/// The notification buffer of a parent watch.
const PARENT_BUFFER: usize = 64 * 1024;
const SEPARATOR: u16 = b'\\' as u16;

/// A probe of a root, by its `\\?\` path.
pub type ProbeFn = Box<dyn Fn(&[u16]) -> Probe + Send + Sync>;

/// Lost roots being waited for: the parent-watch index to the root and the
/// path below the watched ancestor, upper-cased.
type Waiting = Arc<Mutex<HashMap<u16, (RootIdx, Vec<u16>)>>>;

/// Creates the plug-and-play registration of one root; dropping what it
/// returns removes the registration. `None` when the root cannot be watched
/// that way.
pub type NotifierFactory =
    Box<dyn Fn(RootIdx, &[u16]) -> Option<Box<dyn Send + Sync>> + Send + Sync>;

/// One watched root, as the host needs it.
#[derive(Debug, Clone)]
pub struct WatchSpec {
    /// The root's index (its position in the settings).
    pub root: RootIdx,
    /// Its volume.
    pub vol: VolIdx,
    /// The absolute path as configured.
    pub plain: Vec<u16>,
    /// The `\\?\` form.
    pub long: Vec<u16>,
    /// Whether subdirectories are watched.
    pub recursive: bool,
    /// The notification buffer in bytes.
    pub buffer_bytes: usize,
    /// Also report last-access changes.
    pub last_access: bool,
}

/// Where the lifecycle rows go, and what the host does around the writer.
pub struct Services {
    /// State rows.
    pub rows: Box<dyn RowSink>,
    /// Waits until queued rows are on disk (power suspend).
    pub flush: Box<dyn Fn() + Send + Sync>,
    /// Probes a root; replaceable so tests can inject faults.
    pub probe: ProbeFn,
    /// The clocks for rows.
    pub clock: Clock,
    /// Registers a root for volume notifications (query-remove, lock, ...).
    pub notifier: Option<NotifierFactory>,
}

/// A real probe.
pub fn real_probe() -> ProbeFn {
    Box::new(probe_root)
}

enum Job {
    Signal(RootIdx, Signal),
    /// Open the watch for the first time.
    Open(RootIdx),
    /// Something happened above a lost root: is it back?
    Recheck(RootIdx),
    Stop,
}

struct Inner {
    watches: Vec<WatchSpec>,
    machines: Mutex<Vec<Machine>>,
    queue: Mutex<VecDeque<Job>>,
    wake: Condvar,
    services: Mutex<Services>,
    watcher: Mutex<Option<Watcher>>,
    parents: Mutex<Option<Watcher>>,
    waiting: Waiting,
    reconciler: Mutex<Option<Reconciler>>,
    /// The volume notification registrations, by root.
    notifiers: Mutex<HashMap<u16, Box<dyn Send + Sync>>>,
    /// Registrations that live as long as the host.
    keep: Mutex<Vec<Box<dyn Send + Sync>>>,
    /// Canonical directories whose events are dropped before the ring.
    exclude: Vec<Vec<u16>>,
    stopping: AtomicBool,
}

/// The default notification buffer of a local root.
const DEFAULT_BUFFER_BYTES: usize = 1024 * 1024;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

/// One [`WatchSpec`] per `[[watch]]` of the settings, in order: a watch's
/// position is its root index. Volumes get indexes in order of first use.
/// A path that cannot be opened still gets a spec, so the failure is a logged
/// `Lost` row and not a missing watch.
#[must_use]
pub fn watches_from_settings(settings: &peekbuddy_core::settings::Settings) -> Vec<WatchSpec> {
    use peekbuddy_win::io::dir::DirHandle;
    use peekbuddy_win::io::open::OpenOptions;
    use peekbuddy_win::io::path::to_long_path;
    use peekbuddy_win::io::volume::volume_identity;

    let mut volumes: Vec<Vec<u16>> = Vec::new();
    settings
        .watches
        .iter()
        .enumerate()
        .map(|(index, watch)| {
            let plain = wide(&watch.path);
            let long = to_long_path(&plain).unwrap_or_else(|_| plain.clone());
            let guid = DirHandle::open(&long, OpenOptions::default())
                .ok()
                .and_then(|dir| volume_identity(&dir, &long).ok())
                .map(|identity| identity.guid_path);
            let vol = guid.map_or(0, |guid| {
                volumes.iter().position(|v| *v == guid).unwrap_or_else(|| {
                    volumes.push(guid);
                    volumes.len() - 1
                })
            });
            WatchSpec {
                root: RootIdx(u16::try_from(index).unwrap_or(u16::MAX)),
                vol: VolIdx(u16::try_from(vol).unwrap_or(0)),
                plain,
                long,
                recursive: watch.recursive,
                buffer_bytes: watch
                    .buffer_kib
                    .map_or(DEFAULT_BUFFER_BYTES, |kib| {
                        usize::try_from(kib).unwrap_or(1024).saturating_mul(1024)
                    })
                    .next_multiple_of(4),
                last_access: false,
            }
        })
        .collect()
}

/// The filter rules of the settings compiled once: the global presets, globs
/// and regexes, then each watch's own. `None` when they do not compile (the
/// settings validator refused such rules, so this is a safety net); then
/// everything is recorded.
#[must_use]
pub fn filters_from_settings(settings: &peekbuddy_core::settings::Settings) -> Option<FilterSet> {
    use peekbuddy_core::filter::{Pattern, Rule, Scope, presets};

    let all = presets::all().ok()?;
    let mut rules: Vec<Rule> = Vec::new();
    let mut add = |scope: Scope, names: &[String], globs: &[String], regexes: &[String]| {
        for name in names {
            if let Some(preset) = all.iter().find(|p| &p.name == name) {
                rules.extend(preset.rules(scope));
            }
        }
        for glob in globs {
            rules.push(Rule {
                scope,
                ..Rule::exclude(Pattern::Glob(glob.clone()))
            });
        }
        for regex in regexes {
            rules.push(Rule {
                scope,
                ..Rule::exclude(Pattern::Regex(regex.clone()))
            });
        }
    };
    let global = &settings.filters;
    add(
        Scope::Global,
        &global.presets,
        &global.exclude_globs,
        &global.exclude_regex,
    );
    for (index, watch) in settings.watches.iter().enumerate() {
        let scope = Scope::Watch(RootIdx(u16::try_from(index).unwrap_or(u16::MAX)));
        add(scope, &watch.presets, &watch.exclude_globs, &[]);
    }
    if rules.is_empty() {
        return None;
    }
    FilterSet::compile(&rules).ok()
}

/// The canonical (`GetFinalPathNameByHandleW`) forms of `dirs`, for the
/// self-exclusion of the data directory. A directory that cannot be opened is
/// left out.
#[must_use]
pub fn canonical_dirs(dirs: &[&std::path::Path]) -> Vec<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;

    use peekbuddy_win::io::dir::DirHandle;
    use peekbuddy_win::io::open::OpenOptions;
    use peekbuddy_win::io::path::to_long_path;
    use peekbuddy_win::io::volume::final_path;

    dirs.iter()
        .filter_map(|dir| {
            let wide: Vec<u16> = dir.as_os_str().encode_wide().collect();
            let long = to_long_path(&wide).ok()?;
            let handle = DirHandle::open(&long, OpenOptions::default()).ok()?;
            final_path(&handle).ok()
        })
        .collect()
}

/// A handle to post signals; cheap to clone and safe to use from any thread,
/// including plug-and-play and power callbacks.
#[derive(Clone)]
pub struct Signaller {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Signaller {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Signaller").finish_non_exhaustive()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Inner {
    fn post(&self, job: Job) {
        lock(&self.queue).push_back(job);
        self.wake.notify_one();
    }
}

impl Signaller {
    /// Posts `signal` for `root`; returns at once.
    pub fn signal(&self, root: RootIdx, signal: Signal) {
        self.inner.post(Job::Signal(root, signal));
    }

    /// Posts `signal` for every root on `vol`.
    pub fn signal_volume(&self, vol: VolIdx, signal: Signal) {
        for watch in self.inner.watches.iter().filter(|w| w.vol == vol) {
            self.inner.post(Job::Signal(watch.root, signal));
        }
    }

    /// Posts `signal` for every root.
    pub fn signal_all(&self, signal: Signal) {
        for watch in &self.inner.watches {
            self.inner.post(Job::Signal(watch.root, signal));
        }
    }

    /// The state of every root, for the tray and the viewer.
    #[must_use]
    pub fn states(&self) -> Vec<(RootIdx, State)> {
        let machines = lock(&self.inner.machines);
        self.inner
            .watches
            .iter()
            .zip(machines.iter())
            .map(|(watch, machine)| (watch.root, machine.state()))
            .collect()
    }
}

/// The completions of the watcher as signals.
struct WatcherSignals(Signaller);

impl LifecycleSink for WatcherSignals {
    fn state(&mut self, root: RootIdx, state: WatchState, detail: Detail) {
        match state {
            WatchState::Live => self.0.signal(root, Signal::Armed),
            WatchState::Lost => {
                let signal = match detail.error {
                    Some(WinError::AccessDenied | WinError::NotifyCleanup) => Signal::RootDeleted,
                    _ => Signal::IoError,
                };
                self.0.signal(root, signal);
            }
            _ => {}
        }
    }

    fn reconcile(&mut self, root: RootIdx, reason: GapReason) {
        self.0.signal(root, Signal::Gap(reason));
    }
}

/// The reconciler's progress as signals.
struct ReconcileSignals(Signaller);

impl Observer for ReconcileSignals {
    fn started(&self, root: RootIdx) {
        self.0.signal(root, Signal::ReconcileStarted);
    }

    fn finished(&self, root: RootIdx, summary: Option<&peekbuddy_core::reconcile::walk::Summary>) {
        let cancelled = summary.is_some_and(|s| s.cancelled);
        self.0.signal(
            root,
            if cancelled {
                Signal::ReconcileCancelled
            } else {
                Signal::ReconcileDone
            },
        );
    }
}

/// Pause rows as signals: the machine writes the row.
pub struct PauseSignals {
    target: Arc<OnceLock<Signaller>>,
}

impl PauseSignals {
    /// A sink that forwards once the returned cell is set by [`Host::start`].
    #[must_use]
    pub fn new() -> (Self, Arc<OnceLock<Signaller>>) {
        let target = Arc::new(OnceLock::new());
        (
            Self {
                target: Arc::clone(&target),
            },
            target,
        )
    }
}

impl RowSink for PauseSignals {
    fn row(&mut self, event: Event) {
        let Some(signaller) = self.target.get() else {
            return;
        };
        let signal = match event.kind {
            Kind::WatchStateChanged(WatchState::SuspendedByUser) => Signal::UserPause,
            Kind::WatchStateChanged(WatchState::Live) => Signal::UserResume {
                missed: event.size,
                lower_bound: event
                    .flags
                    .contains(peekbuddy_core::event::Flags::GAP_FOLLOWS),
            },
            _ => return,
        };
        signaller.signal(event.root, signal);
    }
}

/// Events into the writer queue: filters first, then a batch that goes out at
/// the end of each completion.
pub struct StoreSink {
    producer: Producer,
    batch: Option<Batch>,
    filters: Option<FilterSet>,
    scratch: Scratch,
    counters: Arc<Counters>,
}

impl StoreSink {
    /// A sink over `producer`; `filters` decide what is never recorded.
    #[must_use]
    pub fn new(producer: Producer, filters: Option<FilterSet>, counters: Arc<Counters>) -> Self {
        Self {
            producer,
            batch: None,
            filters,
            scratch: Scratch::new(),
            counters,
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

impl EventSink for StoreSink {
    fn push(&mut self, event: &Event, name: &[u16]) -> bool {
        if let Some(filters) = &self.filters
            && !filters
                .eval(event, WideStr::new(name), &mut self.scratch)
                .record
        {
            return true;
        }
        self.counters
            .events_received
            .fetch_add(1, Ordering::Relaxed);
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
        true
    }

    fn batch_end(&mut self) {
        if self.batch.as_ref().is_some_and(|batch| !batch.is_empty()) {
            self.send();
        }
    }
}

/// Reports events above a lost root, to find out when it is back.
struct ParentSink {
    waiting: Waiting,
    target: Signaller,
}

impl EventSink for ParentSink {
    fn push(&mut self, event: &Event, name: &[u16]) -> bool {
        let waiting = lock(&self.waiting);
        let Some((root, wanted)) = waiting.get(&event.root.0) else {
            return true;
        };
        let folded: Vec<u16> = name.iter().map(|&u| fold_upper(u)).collect();
        if event.root.0 & GUARD_BIT != 0 {
            // A guard watches the parent of a live root: its own name going
            // away is the root being renamed or deleted.
            if folded == *wanted {
                match event.kind {
                    Kind::RenamedFrom => self
                        .target
                        .inner
                        .post(Job::Signal(*root, Signal::RootMoved)),
                    Kind::Deleted => self
                        .target
                        .inner
                        .post(Job::Signal(*root, Signal::RootDeleted)),
                    _ => {}
                }
            }
            return true;
        }
        if !matches!(event.kind, Kind::Created | Kind::RenamedTo | Kind::Modified) {
            return true;
        }
        // The event names the root, or a directory on the way to it.
        let on_the_way = wanted.starts_with(&folded)
            && (wanted.len() == folded.len() || wanted.get(folded.len()) == Some(&SEPARATOR));
        if on_the_way {
            self.target.inner.post(Job::Recheck(*root));
        }
        true
    }
}

/// The running capture: threads, machines and signals.
pub struct Host {
    inner: Arc<Inner>,
    worker: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for Host {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Host").finish_non_exhaustive()
    }
}

/// What the host is made of.
pub struct HostParts {
    /// The roots, indexed by position.
    pub watches: Vec<WatchSpec>,
    /// Where events go (a [`StoreSink`] in the engine).
    pub sink: Box<dyn EventSink + Send>,
    /// The pause switches capture reads.
    pub pause: Arc<Pause>,
    /// The reconcile worker's snapshot and rows.
    pub reconciler: Option<Reconciler>,
    /// Services around the writer.
    pub services: Services,
    /// Canonical directories whose events are dropped before the ring.
    pub exclude: Vec<Vec<u16>>,
}

impl Host {
    /// Starts the watcher and the worker and opens every watch.
    ///
    /// The `reconciler` is created by the caller with
    /// [`Host::observer`] so it can report back.
    ///
    /// # Errors
    ///
    /// The OS error when a thread cannot be started.
    pub fn start(parts: HostParts, bind: &OnceLock<Signaller>) -> std::io::Result<Self> {
        let HostParts {
            watches,
            sink,
            pause,
            reconciler,
            services,
            exclude,
        } = parts;
        let count = watches.len();
        let inner = Arc::new(Inner {
            watches,
            machines: Mutex::new(vec![Machine::new(); count]),
            queue: Mutex::new(VecDeque::new()),
            wake: Condvar::new(),
            services: Mutex::new(services),
            watcher: Mutex::new(None),
            parents: Mutex::new(None),
            waiting: Arc::new(Mutex::new(HashMap::new())),
            reconciler: Mutex::new(reconciler),
            notifiers: Mutex::new(HashMap::new()),
            keep: Mutex::new(Vec::new()),
            exclude,
            stopping: AtomicBool::new(false),
        });
        let signaller = Signaller {
            inner: Arc::clone(&inner),
        };
        let _ = bind.set(signaller.clone());
        let watcher = Watcher::start(
            PausableSink::new(sink, pause),
            WatcherSignals(signaller.clone()),
        )?;
        *lock(&inner.watcher) = Some(watcher);
        let worker_inner = Arc::clone(&inner);
        let worker = std::thread::Builder::new()
            .name(String::from("peekbuddy-lifecycle"))
            .spawn(move || work(&worker_inner))?;
        for watch in &inner.watches {
            inner.post(Job::Open(watch.root));
        }
        Ok(Self {
            inner,
            worker: Some(worker),
        })
    }

    /// The observer to give the reconciler.
    #[must_use]
    pub fn observer(target: &Arc<OnceLock<Signaller>>) -> Arc<dyn Observer> {
        struct Late(Arc<OnceLock<Signaller>>);
        impl Observer for Late {
            fn started(&self, root: RootIdx) {
                if let Some(s) = self.0.get() {
                    ReconcileSignals(s.clone()).started(root);
                }
            }
            fn finished(
                &self,
                root: RootIdx,
                summary: Option<&peekbuddy_core::reconcile::walk::Summary>,
            ) {
                if let Some(s) = self.0.get() {
                    ReconcileSignals(s.clone()).finished(root, summary);
                }
            }
        }
        Arc::new(Late(Arc::clone(target)))
    }

    /// A handle to post signals.
    #[must_use]
    pub fn signaller(&self) -> Signaller {
        Signaller {
            inner: Arc::clone(&self.inner),
        }
    }

    /// Waits until the queue is empty and the worker is idle, at most
    /// `timeout`; for tests.
    #[must_use]
    pub fn wait_quiet(&self, timeout: Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let quiet = lock(&self.inner.queue).is_empty();
            if quiet {
                // One more look after the worker had time to act on the last job.
                std::thread::sleep(Duration::from_millis(30));
                if lock(&self.inner.queue).is_empty() {
                    return true;
                }
            }
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Keeps `guard` (a registration) alive until the host stops.
    pub fn keep(&self, guard: Box<dyn Send + Sync>) {
        lock(&self.inner.keep).push(guard);
    }

    /// Stops the worker, the watchers and the reconciler.
    pub fn stop(&mut self) {
        self.inner.stopping.store(true, Ordering::Release);
        // No more notifications: the callbacks must not post to a dead queue.
        lock(&self.inner.keep).clear();
        lock(&self.inner.notifiers).clear();
        self.inner.post(Job::Stop);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if let Some(mut reconciler) = lock(&self.inner.reconciler).take() {
            reconciler.stop();
        }
        if let Some(watcher) = lock(&self.inner.watcher).take() {
            let _ = watcher.stop();
        }
        if let Some(parents) = lock(&self.inner.parents).take() {
            let _ = parents.stop();
        }
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.stop();
    }
}

fn spec_of(inner: &Inner, root: RootIdx) -> Option<&WatchSpec> {
    inner.watches.iter().find(|w| w.root == root)
}

fn index_of(inner: &Inner, root: RootIdx) -> Option<usize> {
    inner.watches.iter().position(|w| w.root == root)
}

fn work(inner: &Arc<Inner>) {
    loop {
        let job = {
            let mut queue = lock(&inner.queue);
            loop {
                if let Some(job) = queue.pop_front() {
                    break job;
                }
                queue = inner
                    .wake
                    .wait(queue)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
        };
        match job {
            Job::Stop => return,
            Job::Open(root) => open(inner, root),
            Job::Recheck(root) => recheck(inner, root),
            Job::Signal(root, signal) => {
                if !inner.stopping.load(Ordering::Acquire) {
                    handle(inner, root, signal);
                }
            }
        }
    }
}

fn rdcw_spec(inner: &Inner, spec: &WatchSpec) -> RdcwRoot {
    RdcwRoot {
        root: spec.root,
        vol: spec.vol,
        path: spec.plain.clone(),
        recursive: spec.recursive,
        last_access: spec.last_access,
        buffer_bytes: spec.buffer_bytes,
        exclude: inner.exclude.clone(),
        reject: Vec::new(),
    }
}

/// Opens a watch; a failure is the machine's business.
fn open(inner: &Arc<Inner>, root: RootIdx) {
    let Some(spec) = spec_of(inner, root) else {
        return;
    };
    guard_root(inner, spec);
    let result = lock(&inner.watcher)
        .as_ref()
        .map(|watcher| watcher.add_root(rdcw_spec(inner, spec)));
    if matches!(result, Some(Ok(_))) {
        register_notifier(inner, spec);
    } else {
        handle(inner, root, Signal::ArmFailed);
    }
    // On success the watcher reports `Live`, which becomes `Armed`.
}

fn register_notifier(inner: &Inner, spec: &WatchSpec) {
    let guard = lock(&inner.services)
        .notifier
        .as_ref()
        .and_then(|factory| factory(spec.root, &spec.long));
    if let Some(guard) = guard {
        lock(&inner.notifiers).insert(spec.root.0, guard);
    }
}

fn recheck(inner: &Arc<Inner>, root: RootIdx) {
    let Some(spec) = spec_of(inner, root) else {
        return;
    };
    let probe = (lock(&inner.services).probe)(&spec.long);
    if matches!(probe, Probe::Ok) {
        inner.post(Job::Signal(root, Signal::RootRecreated));
    }
}

fn handle(inner: &Arc<Inner>, root: RootIdx, signal: Signal) {
    let Some(index) = index_of(inner, root) else {
        return;
    };
    let (out, vol) = {
        let mut machines = lock(&inner.machines);
        let Some(machine) = machines.get_mut(index) else {
            return;
        };
        (machine.signal(signal), spec_of(inner, root).map(|w| w.vol))
    };
    if let Some(transition) = out.transition {
        let mut services = lock(&inner.services);
        let (ts, qpc) = (services.clock)();
        services
            .rows
            .row(transition.to_event(root, vol.unwrap_or(VolIdx(0)), ts, qpc));
    }
    for action in out.actions() {
        act(inner, root, action);
    }
}

fn act(inner: &Arc<Inner>, root: RootIdx, action: Action) {
    let Some(spec) = spec_of(inner, root) else {
        return;
    };
    match action {
        Action::CloseHandle => {
            // The registration holds a handle too: the volume cannot go
            // while it is open.
            lock(&inner.notifiers).remove(&root.0);
            if let Some(watcher) = lock(&inner.watcher).as_ref() {
                let _ = watcher.remove_root(root);
            }
        }
        Action::Reopen => {
            lock(&inner.notifiers).remove(&root.0);
            let result = {
                let guard = lock(&inner.watcher);
                guard.as_ref().map(|watcher| {
                    let _ = watcher.remove_root(root);
                    watcher.add_root(rdcw_spec(inner, spec))
                })
            };
            if matches!(result, Some(Ok(_))) {
                register_notifier(inner, spec);
            } else {
                inner.post(Job::Signal(root, Signal::ArmFailed));
            }
        }
        Action::Reconcile(reason) => {
            if let Some(reconciler) = lock(&inner.reconciler).as_ref() {
                reconciler.trigger(root, reason);
            }
        }
        Action::Verify => {
            let probe = (lock(&inner.services).probe)(&spec.long);
            let signal = probe
                .verdict()
                .map_or(Signal::RootDeleted, Signal::Verified);
            inner.post(Job::Signal(root, signal));
        }
        Action::WatchParent => watch_parent(inner, spec),
        Action::UnwatchParent => unwatch_parent(inner, spec),
        Action::FlushNow => (lock(&inner.services).flush)(),
    }
}

/// The nearest existing ancestor of the root and the rest of the path below
/// it (upper-cased, backslash separated).
fn ancestor_and_rest(inner: &Inner, spec: &WatchSpec) -> Option<(Vec<u16>, Vec<u16>)> {
    let mut head = spec.plain.clone();
    let mut rest: Vec<u16> = Vec::new();
    while let Some(at) = head.iter().rposition(|&u| u == SEPARATOR) {
        let leaf: Vec<u16> = head.get(at + 1..)?.iter().map(|&u| fold_upper(u)).collect();
        head.truncate(at);
        let mut joined = leaf;
        if !rest.is_empty() {
            joined.push(SEPARATOR);
            joined.extend_from_slice(&rest);
        }
        rest = joined;
        let mut candidate = head.clone();
        if candidate.len() == 2 && candidate.get(1) == Some(&u16::from(b':')) {
            // A drive needs its separator back to name its root.
            candidate.push(SEPARATOR);
        }
        let long = peekbuddy_win::io::path::to_long_path(&candidate).ok()?;
        let probe = (lock(&inner.services).probe)(&long);
        if matches!(probe, Probe::Ok) {
            return Some((candidate, rest));
        }
    }
    None
}

/// Starts the watcher of parent directories if it is not running yet.
fn ensure_parents<'a>(inner: &Arc<Inner>, parents: &'a mut Option<Watcher>) -> Option<&'a Watcher> {
    if parents.is_none() {
        let sink = ParentSink {
            waiting: Arc::clone(&inner.waiting),
            target: Signaller {
                inner: Arc::clone(inner),
            },
        };
        *parents = Watcher::start(sink, ParentLife).ok();
    }
    parents.as_ref()
}

fn watch_parent(inner: &Arc<Inner>, spec: &WatchSpec) {
    let Some((ancestor, rest)) = ancestor_and_rest(inner, spec) else {
        return;
    };
    let wait_root = RootIdx(WAIT_BIT | spec.root.0);
    lock(&inner.waiting).insert(wait_root.0, (spec.root, rest));
    let mut parents = lock(&inner.parents);
    let Some(watcher) = ensure_parents(inner, &mut parents) else {
        return;
    };
    let _ = watcher.remove_root(wait_root);
    let added = watcher.add_root(RdcwRoot {
        root: wait_root,
        vol: spec.vol,
        path: ancestor,
        recursive: true,
        last_access: false,
        buffer_bytes: PARENT_BUFFER,
        exclude: Vec::new(),
        reject: Vec::new(),
    });
    drop(parents);
    if added.is_ok() {
        // The root may have come back between the loss and this watch.
        inner.post(Job::Recheck(spec.root));
    }
}

/// Watches the parent of a live root (not its subtree), so that the root
/// being renamed or deleted is an event. A drive root has no parent.
fn guard_root(inner: &Arc<Inner>, spec: &WatchSpec) {
    let Some(at) = spec.plain.iter().rposition(|&u| u == SEPARATOR) else {
        return;
    };
    let (Some(parent), Some(leaf)) = (spec.plain.get(..at), spec.plain.get(at + 1..)) else {
        return;
    };
    if leaf.is_empty() || parent.len() < 3 {
        return;
    }
    let guard_root = RootIdx(GUARD_BIT | spec.root.0);
    let leaf: Vec<u16> = leaf.iter().map(|&u| fold_upper(u)).collect();
    lock(&inner.waiting).insert(guard_root.0, (spec.root, leaf));
    let mut parents = lock(&inner.parents);
    let Some(watcher) = ensure_parents(inner, &mut parents) else {
        return;
    };
    let _ = watcher.remove_root(guard_root);
    let _ = watcher.add_root(RdcwRoot {
        root: guard_root,
        vol: spec.vol,
        path: parent.to_vec(),
        recursive: false,
        last_access: false,
        buffer_bytes: PARENT_BUFFER,
        exclude: Vec::new(),
        reject: Vec::new(),
    });
}

fn unwatch_parent(inner: &Inner, spec: &WatchSpec) {
    let parent_root = RootIdx(WAIT_BIT | spec.root.0);
    lock(&inner.waiting).remove(&parent_root.0);
    if let Some(parents) = lock(&inner.parents).as_ref() {
        let _ = parents.remove_root(parent_root);
    }
}

/// The parent watch has no lifecycle of its own: losing it only means the
/// root is not noticed coming back until the next start.
struct ParentLife;

impl LifecycleSink for ParentLife {
    fn state(&mut self, _: RootIdx, _: WatchState, _: Detail) {}
    fn reconcile(&mut self, _: RootIdx, _: GapReason) {}
}

/// A volume notice as the signal for the root it came for.
#[must_use]
pub const fn signal_of_notice(notice: peekbuddy_win::pnp::Notice) -> Signal {
    use peekbuddy_win::pnp::Notice;
    match notice {
        Notice::QueryRemove => Signal::QueryRemove,
        Notice::QueryRemoveFailed => Signal::QueryRemoveFailed,
        Notice::RemovePending => Signal::RemovePending,
        Notice::RemoveComplete => Signal::RemoveComplete,
        Notice::Lock => Signal::VolumeLock,
        Notice::LockFailed | Notice::DismountFailed => Signal::LockFailed,
        Notice::Dismount => Signal::Dismount,
        Notice::Unlock | Notice::Mount => Signal::VolumeBack,
        // Locked or unlocked is not in the message: ask the volume.
        Notice::FveChange => Signal::IoError,
    }
}

/// The notifier factory of the engine: one `CM_Register_Notification` per
/// root, posting to the host once it is bound.
pub fn notifier_factory(bind: &Arc<OnceLock<Signaller>>) -> NotifierFactory {
    let bind = Arc::clone(bind);
    Box::new(move |root, long| {
        let target = Arc::clone(&bind);
        let watch = peekbuddy_win::pnp::HandleWatch::register(
            long,
            Arc::new(move |notice| {
                if let Some(signaller) = target.get() {
                    signaller.signal(root, signal_of_notice(notice));
                }
            }),
        )
        .ok()?;
        Some(Box::new(watch) as Box<dyn Send + Sync>)
    })
}

/// Registers the host-wide notifications: volume arrival and power. The
/// returned guards live in the host.
pub fn watch_system(host: &Host) {
    let signaller = host.signaller();
    let on_volume = signaller.clone();
    if let Ok(watch) = peekbuddy_win::pnp::InterfaceWatch::register(Arc::new(move |arrival| {
        if arrival {
            on_volume.signal_all(Signal::VolumeBack);
        }
    })) {
        host.keep(Box::new(watch));
    }
    if let Ok(watch) = peekbuddy_win::pnp::PowerWatch::register(Arc::new(move |event| {
        signaller.signal_all(match event {
            peekbuddy_win::pnp::PowerEvent::Suspend => Signal::PowerSuspend,
            peekbuddy_win::pnp::PowerEvent::Resume => Signal::PowerResume,
        });
    })) {
        host.keep(Box::new(watch));
    }
}
