//! The USN journal as a capture source of the engine (#212, feature `usn`).
//!
//! One [`UsnRunner`] watches one root through the journal of its volume. Its
//! thread opens the journal, plans the resume from the saved [`Checkpoint`]
//! (an intact journal continues without a gap, anything else writes exactly
//! one gap row and asks for a reconcile), fills the [`Scope`] from a
//! directory walk and then reads with a pending read that costs no wakeups
//! while the volume is idle. Every buffer goes through the [`Feed`]; after the
//! events of a buffer were offered to the sink the next USN is saved, so a
//! checkpoint never moves ahead of the rows it covers (the sink and the saved
//! cursor are ordered by the same writer queue).
//!
//! On `ReFS` and Dev Drive the overlapped read fails with error 1784 and the
//! runner switches to a [`SyncReader`] thread; the loop below does not care.
//!
//! The journal is the primary source only where it exists: [`UsnRunner::start`]
//! returns an error for a volume without one and the caller stays on
//! `ReadDirectoryChangesExW`.

use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use peekbuddy_core::capture::{EventSink, Stamp};
use peekbuddy_core::event::{GapReason, RootIdx, VolIdx};
use peekbuddy_core::usn::checkpoint::{
    Checkpoint, JournalData, Resume, gap_for_error, needs_synchronous_reads, plan_resume,
};
use peekbuddy_core::usn::feed::{Counters, Feed, Names, relative_to};
use peekbuddy_core::usn::scope::Scope;
use peekbuddy_core::usn::walk::walk_into;
use peekbuddy_win::io::dir::DirHandle;
use peekbuddy_win::io::iocp::{Batch, Completion, Iocp};
use peekbuddy_win::io::open::OpenOptions;
use peekbuddy_win::io::path::to_long_path;
use peekbuddy_win::io::volume::volume_identity;
use peekbuddy_win::reconcile::DirLister;
use peekbuddy_win::usn::{DEFAULT_BUFFER, Journal, SyncReader};

use crate::pause_ctl::Clock;

/// The completion key of journal reads.
const KEY_READ: usize = 1;
/// The completion key of the stop message.
const KEY_STOP: usize = 2;
/// `ERROR_OPERATION_ABORTED`.
const ERROR_OPERATION_ABORTED: i32 = 995;
/// How long a journal that is being recreated is waited for, and how often
/// it is asked.
const REQUERY_FOR: Duration = Duration::from_secs(20);
const REQUERY_EVERY: Duration = Duration::from_millis(100);
/// How long a cancelled read may take to report.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(5);
/// The longest volume-relative path resolved for a name.
const PATH_UNITS: usize = 32_768;

/// Where the position of the read is kept.
pub trait Checkpoints: Send {
    /// The saved position of this root, when there is one.
    fn load(&self) -> Option<Checkpoint>;

    /// Records that every event before `checkpoint.next_usn` was offered to
    /// the sink. The implementation orders this after those events (the
    /// writer queue does).
    fn save(&self, checkpoint: Checkpoint);
}

/// What a runner needs to start.
pub struct UsnSpec<'a> {
    /// The volume index events carry.
    pub vol: VolIdx,
    /// The root index events carry.
    pub root: RootIdx,
    /// The absolute path of the watched directory.
    pub path: &'a Path,
}

/// Called when a gap row was written and the root needs a reconcile.
pub type OnGap = Box<dyn Fn(GapReason) + Send>;

/// A running USN source.
pub struct UsnRunner {
    iocp: Arc<Iocp>,
    stop: Arc<AtomicBool>,
    counters: Arc<Mutex<Counters>>,
    synchronous: Arc<AtomicBool>,
    thread: Option<JoinHandle<Option<io::Error>>>,
}

impl std::fmt::Debug for UsnRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UsnRunner").finish_non_exhaustive()
    }
}

impl UsnRunner {
    /// Opens the journal of `spec.path`'s volume, plans the resume and starts
    /// reading on a thread of its own.
    ///
    /// # Errors
    ///
    /// The OS error when the volume has no journal (`ERROR_JOURNAL_NOT_ACTIVE`
    /// or `ERROR_INVALID_FUNCTION`), cannot be opened, or the root cannot be
    /// walked; nothing was started then.
    pub fn start(
        spec: &UsnSpec<'_>,
        checkpoints: Box<dyn Checkpoints>,
        sink: Box<dyn EventSink + Send>,
        on_gap: OnGap,
        clock: Clock,
    ) -> io::Result<Self> {
        let iocp = Arc::new(Iocp::new(1)?);
        let stop = Arc::new(AtomicBool::new(false));
        let counters = Arc::new(Mutex::new(Counters::default()));
        let synchronous = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = mpsc::channel();
        let job = Job {
            vol: spec.vol,
            root: spec.root,
            path: spec.path.to_path_buf(),
            checkpoints,
            sink,
            on_gap,
            clock,
            iocp: Arc::clone(&iocp),
            stop: Arc::clone(&stop),
            counters: Arc::clone(&counters),
            synchronous: Arc::clone(&synchronous),
        };
        // The journal handles hold OVERLAPPED state that must not cross
        // threads, so everything is opened on the thread that uses it; the
        // setup result comes back here.
        let thread = std::thread::Builder::new()
            .name("pb-usn".into())
            .spawn(move || job.run(&ready_tx))?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                iocp,
                stop,
                counters,
                synchronous,
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(_) => {
                let _ = thread.join();
                Err(io::Error::other("the USN thread ended during setup"))
            }
        }
    }

    /// What the feed counted so far.
    #[must_use]
    pub fn counters(&self) -> Counters {
        *self.counters.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether the volume refused overlapped reads (`ReFS`, Dev Drive) and
    /// the runner reads on a thread of its own.
    #[must_use]
    pub fn uses_synchronous_reads(&self) -> bool {
        self.synchronous.load(Ordering::SeqCst)
    }

    /// Stops reading: the pending read is cancelled and the thread joined.
    /// Returns the error that ended the thread earlier, if one did.
    pub fn stop(&mut self) -> Option<io::Error> {
        let thread = self.thread.take()?;
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.iocp.post(KEY_STOP, 0, 0);
        thread.join().ok().flatten()
    }
}

impl Drop for UsnRunner {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

struct Job {
    vol: VolIdx,
    root: RootIdx,
    path: std::path::PathBuf,
    checkpoints: Box<dyn Checkpoints>,
    sink: Box<dyn EventSink + Send>,
    on_gap: OnGap,
    clock: Clock,
    iocp: Arc<Iocp>,
    stop: Arc<AtomicBool>,
    counters: Arc<Mutex<Counters>>,
    synchronous: Arc<AtomicBool>,
}

/// Name lookups for the feed: the control handle resolves reference numbers,
/// the lister walks directories.
struct Lookup<'a> {
    control: &'a Journal,
    lister: DirLister,
    root_path: &'a [u16],
    units: Vec<u16>,
}

impl Names for Lookup<'_> {
    fn path_of(&mut self, frn: u128, out: &mut Vec<u16>) -> bool {
        self.units.resize(PATH_UNITS, 0);
        match self.control.resolve_path(frn, &mut self.units) {
            Ok(len) => {
                out.clear();
                out.extend_from_slice(self.units.get(..len).unwrap_or_default());
                true
            }
            Err(_) => false,
        }
    }

    fn walk(&mut self, frn: u128, scope: &mut Scope) {
        let mut full = Vec::new();
        if !self.path_of(frn, &mut full) {
            return;
        }
        let Some(rel) = relative_to(&full, self.root_path) else {
            return;
        };
        let _ = walk_into(&mut self.lister, scope, frn, rel);
    }
}

/// What the reading loop carries from read to read.
struct Loop {
    reader: Journal,
    mode: Reader,
    feed: Feed,
    next: i64,
    journal_id: u64,
    batch: Batch,
    bytes: Vec<u8>,
}

/// What `wait` found.
enum Waited {
    Read(Completion),
    Stop,
    Failed(io::Error),
}

/// Whether `error` is the one that asks for the synchronous reader.
fn needs_sync_error(error: &io::Error) -> bool {
    error
        .raw_os_error()
        .and_then(|code| u32::try_from(code).ok())
        .is_some_and(needs_synchronous_reads)
}

/// How buffers arrive.
enum Reader {
    Overlapped,
    Sync(SyncReader),
}

impl Job {
    fn run(mut self, ready: &mpsc::Sender<io::Result<()>>) -> Option<io::Error> {
        let Setup {
            control,
            reader,
            root_path,
            feed,
            next,
            journal_id,
        } = match self.setup() {
            Ok(parts) => parts,
            Err(error) => {
                let _ = ready.send(Err(error));
                return None;
            }
        };
        let _ = ready.send(Ok(()));
        let mut names = Lookup {
            control: &control,
            lister: DirLister::new(&self.long_root().unwrap_or_default()),
            root_path: &root_path,
            units: Vec::new(),
        };
        let mut state = Loop {
            reader,
            mode: Reader::Overlapped,
            feed,
            next,
            journal_id,
            batch: Batch::new(4),
            bytes: Vec::new(),
        };
        let failure = self.read_loop(&mut state, &mut names);
        if let Reader::Sync(mut sync) = state.mode {
            sync.stop();
        }
        failure
    }

    /// Reads until stopped; the error that ended the source, if one did.
    fn read_loop(&mut self, st: &mut Loop, names: &mut Lookup<'_>) -> Option<io::Error> {
        while !self.stop.load(Ordering::SeqCst) {
            if matches!(st.mode, Reader::Overlapped)
                && let Err(error) = st.reader.begin_wait(st.journal_id, st.next)
            {
                if let Some(fatal) = self.handle(error, st, names.control) {
                    return Some(fatal);
                }
                continue;
            }
            let completion = match self.wait(st) {
                Waited::Read(completion) => completion,
                Waited::Stop => return None,
                Waited::Failed(error) => return Some(error),
            };
            let result = match &st.mode {
                Reader::Overlapped => st.reader.finish(&completion).map(|data| {
                    st.bytes.clear();
                    st.bytes.extend_from_slice(data);
                }),
                Reader::Sync(sync) => sync.take(&mut st.bytes).unwrap_or(Ok(())),
            };
            match result {
                Ok(()) => self.deliver(st, names),
                Err(error) if error.raw_os_error() == Some(ERROR_OPERATION_ABORTED) => {}
                Err(error) => {
                    if let Some(fatal) = self.handle(error, st, names.control) {
                        return Some(fatal);
                    }
                }
            }
        }
        None
    }

    /// Waits for a finished read or the stop message; a stop cancels the
    /// pending read and takes its completion off the port first.
    fn wait(&self, st: &mut Loop) -> Waited {
        loop {
            let got = match self.iocp.dequeue(&mut st.batch, None) {
                Ok(got) => got,
                Err(error) => return Waited::Failed(error),
            };
            let mut read = None;
            let mut stop = false;
            for completion in st.batch.iter().take(got) {
                if completion.key == KEY_STOP {
                    stop = true;
                } else {
                    read = Some(completion);
                }
            }
            if stop {
                self.finish_pending(st, read.as_ref());
                return Waited::Stop;
            }
            if let Some(read) = read {
                return Waited::Read(read);
            }
        }
    }

    /// Ends a pending read at a stop. A completion that came with the stop
    /// message is finished as it is; otherwise the read is cancelled and its
    /// completion taken off the port (for a bounded time: the slot is leaked,
    /// not freed, if the kernel never answers).
    fn finish_pending(&self, st: &mut Loop, read: Option<&Completion>) {
        if !matches!(st.mode, Reader::Overlapped) || !st.reader.is_pending() {
            return;
        }
        if let Some(read) = read {
            let _ = st.reader.finish(read);
            return;
        }
        let _ = st.reader.cancel();
        if let Some(done) = self.drain_read(&st.reader) {
            let _ = st.reader.finish(&done);
        }
    }

    /// Feeds one buffer, saves the position and publishes the counters.
    fn deliver(&mut self, st: &mut Loop, names: &mut Lookup<'_>) {
        let (ts, qpc) = (self.clock)();
        let stamp = Stamp { ts, qpc };
        match st.feed.decode(&st.bytes, stamp, names, &mut self.sink) {
            Ok(after) => {
                st.next = after;
                self.checkpoints.save(Checkpoint {
                    journal_id: st.journal_id,
                    next_usn: after,
                });
            }
            // A buffer without a header: the parser gap is owed and goes out
            // with the next buffer; the root needs a reconcile.
            Err(_) => (self.on_gap)(GapReason::ParserError),
        }
        *self.counters.lock().unwrap_or_else(PoisonError::into_inner) = st.feed.counters();
    }

    /// A failed read: recover and go on (`None`), or give up with the error.
    fn handle(&mut self, error: io::Error, st: &mut Loop, control: &Journal) -> Option<io::Error> {
        match self.recover(
            &error,
            control,
            &mut st.feed,
            &mut st.next,
            &mut st.journal_id,
        ) {
            Recovery::Fatal => return Some(error),
            Recovery::Synchronous | Recovery::Retry
                if matches!(st.mode, Reader::Overlapped) && !needs_sync_error(&error) =>
            {
                return None;
            }
            Recovery::Synchronous | Recovery::Retry => {}
        }
        // The synchronous reader starts (or starts again, its thread ended
        // with the error) at the current position.
        if let Reader::Sync(old) = &mut st.mode {
            old.stop();
        }
        match self.switch_to_sync(control, st.journal_id, st.next) {
            Ok(sync) => {
                st.mode = Reader::Sync(sync);
                self.synchronous.store(true, Ordering::SeqCst);
                None
            }
            Err(error) => Some(error),
        }
    }

    fn long_root(&self) -> io::Result<Vec<u16>> {
        use std::os::windows::ffi::OsStrExt as _;
        let wide: Vec<u16> = self.path.as_os_str().encode_wide().collect();
        to_long_path(&wide).map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))
    }

    /// Everything that can fail before the first read.
    fn setup(&mut self) -> io::Result<Setup> {
        let long = self.long_root()?;
        let root_dir = DirHandle::open(&long, OpenOptions::default())?;
        let identity = volume_identity(&root_dir, &long)?;
        drop(root_dir);
        let root_frn = peekbuddy_win::fs::file_id(&self.path)?;

        let control = Journal::open(&identity.guid_path, DEFAULT_BUFFER)?;
        let mut query = Journal::open(&identity.guid_path, DEFAULT_BUFFER)?;
        let data = query.query()?;
        let mut units = vec![0_u16; PATH_UNITS];
        let len = control.resolve_path(root_frn, &mut units)?;
        units.truncate(len);
        let root_path = units;

        let mut reader = Journal::open(&identity.guid_path, DEFAULT_BUFFER)?;
        reader.associate(&self.iocp, KEY_READ)?;

        let mut scope = Scope::new(root_frn);
        let mut lister = DirLister::new(&long);
        walk_into(&mut lister, &mut scope, root_frn, &[])
            .map_err(|error| io::Error::other(format!("walking the root failed: {error:?}")))?;
        let mut feed = Feed::new(scope, self.vol, self.root, &root_path);

        let (ts, qpc) = (self.clock)();
        let stamp = Stamp { ts, qpc };
        let next = match plan_resume(self.checkpoints.load(), &data) {
            Resume::Continue { from } | Resume::Fresh { from } => from,
            Resume::Gap { reason, from } => {
                feed.gap(reason, stamp, &mut self.sink);
                self.sink.batch_end();
                (self.on_gap)(reason);
                from
            }
        };
        self.checkpoints.save(Checkpoint {
            journal_id: data.journal_id,
            next_usn: next,
        });
        Ok(Setup {
            control,
            reader,
            root_path,
            feed,
            next,
            journal_id: data.journal_id,
        })
    }

    /// Takes the read completion off the port after a cancel.
    fn drain_read(&self, reader: &Journal) -> Option<Completion> {
        let mut batch = Batch::new(4);
        while reader.is_pending() {
            let got = self.iocp.dequeue(&mut batch, Some(DRAIN_TIMEOUT)).ok()?;
            if got == 0 {
                return None;
            }
            if let Some(found) = batch
                .iter()
                .take(got)
                .find(|completion| completion.key == KEY_READ)
            {
                return Some(found);
            }
        }
        None
    }

    fn switch_to_sync(
        &self,
        control: &Journal,
        journal_id: u64,
        next: i64,
    ) -> io::Result<SyncReader> {
        let guid = control.guid_path();
        SyncReader::spawn(
            guid,
            DEFAULT_BUFFER,
            journal_id,
            next,
            Arc::clone(&self.iocp),
            KEY_READ,
        )
    }

    /// Decides what a failed read means: a gap (the journal is queried
    /// again and the read goes on from its end), the `ReFS` fallback, or the
    /// end of the source.
    fn recover(
        &mut self,
        error: &io::Error,
        control: &Journal,
        feed: &mut Feed,
        next: &mut i64,
        journal_id: &mut u64,
    ) -> Recovery {
        let Some(code) = error.raw_os_error().and_then(|c| u32::try_from(c).ok()) else {
            return Recovery::Fatal;
        };
        if needs_synchronous_reads(code) {
            return Recovery::Synchronous;
        }
        let Some(reason) = gap_for_error(code) else {
            return Recovery::Fatal;
        };
        let (ts, qpc) = (self.clock)();
        feed.gap(reason, Stamp { ts, qpc }, &mut self.sink);
        self.sink.batch_end();
        (self.on_gap)(reason);
        let Some(data) = self.requery(control) else {
            return Recovery::Fatal;
        };
        *next = data.next_usn;
        *journal_id = data.journal_id;
        self.checkpoints.save(Checkpoint {
            journal_id: data.journal_id,
            next_usn: data.next_usn,
        });
        Recovery::Retry
    }
}

/// The state the reading loop starts from.
struct Setup {
    control: Journal,
    reader: Journal,
    root_path: Vec<u16>,
    feed: Feed,
    next: i64,
    journal_id: u64,
}

impl Job {
    /// The journal's data after a gap. While a journal is being recreated
    /// (deleted, not yet active) the query fails with 1178 or 1179; it is
    /// repeated for a bounded time, the only wait outside the idle path.
    fn requery(&self, control: &Journal) -> Option<JournalData> {
        let mut query = Journal::open(control.guid_path(), DEFAULT_BUFFER).ok()?;
        let deadline = Instant::now() + REQUERY_FOR;
        loop {
            match query.query() {
                Ok(data) => return Some(data),
                Err(error) => {
                    let recreating = error
                        .raw_os_error()
                        .and_then(|code| u32::try_from(code).ok())
                        .is_some_and(|code| gap_for_error(code).is_some());
                    if !recreating || Instant::now() >= deadline || self.stop.load(Ordering::SeqCst)
                    {
                        return None;
                    }
                    std::thread::sleep(REQUERY_EVERY);
                }
            }
        }
    }
}

enum Recovery {
    Retry,
    Synchronous,
    Fatal,
}
