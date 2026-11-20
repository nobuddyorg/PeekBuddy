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
use peekbuddy_core::reconcile::DirSource;
use peekbuddy_core::usn::checkpoint::{
    Checkpoint, JournalData, Resume, gap_for_error, needs_synchronous_reads, plan_resume,
};
use peekbuddy_core::usn::feed::{Counters, Feed, Names, relative_to};
use peekbuddy_core::usn::scope::Scope;
use peekbuddy_core::usn::walk::walk_into;
use peekbuddy_win::io::dir::{CancelOutcome, DirHandle};
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
            requery_for: REQUERY_FOR,
            requery_every: REQUERY_EVERY,
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
    requery_for: Duration,
    requery_every: Duration,
}

/// The overlapped reader of the journal as the reading loop uses it. The
/// signatures are those of [`Journal`], which is the one real
/// implementation; the tests drive the loop with a scripted fake.
trait JournalRead {
    fn begin_wait(&mut self, journal_id: u64, start_usn: i64) -> io::Result<()>;
    fn is_pending(&self) -> bool;
    fn cancel(&self) -> io::Result<CancelOutcome>;
    fn finish(&mut self, completion: &Completion) -> io::Result<&[u8]>;
}

impl JournalRead for Journal {
    fn begin_wait(&mut self, journal_id: u64, start_usn: i64) -> io::Result<()> {
        Self::begin_wait(self, journal_id, start_usn)
    }

    fn is_pending(&self) -> bool {
        Self::is_pending(self)
    }

    fn cancel(&self) -> io::Result<CancelOutcome> {
        Self::cancel(self)
    }

    fn finish(&mut self, completion: &Completion) -> io::Result<&[u8]> {
        Self::finish(self, completion)
    }
}

/// One query of the journal's data on a handle of its own.
trait JournalQuery {
    fn query(&mut self) -> io::Result<JournalData>;
}

impl JournalQuery for Journal {
    fn query(&mut self) -> io::Result<JournalData> {
        Self::query(self)
    }
}

/// The synchronous reader (`ReFS`, Dev Drive) as the loop uses it.
trait SyncSource {
    fn take(&self, out: &mut Vec<u8>) -> Option<io::Result<()>>;
    fn stop(&mut self);
}

impl SyncSource for SyncReader {
    fn take(&self, out: &mut Vec<u8>) -> Option<io::Result<()>> {
        Self::take(self, out)
    }

    fn stop(&mut self) {
        Self::stop(self);
    }
}

/// The control handle of the volume: names, fresh queries, the fallback
/// reader.
trait JournalControl {
    fn resolve_path(&self, frn: u128, out: &mut [u16]) -> io::Result<usize>;
    fn open_query(&self) -> io::Result<Box<dyn JournalQuery>>;
    fn spawn_sync(
        &self,
        journal_id: u64,
        next: i64,
        iocp: Arc<Iocp>,
    ) -> io::Result<Box<dyn SyncSource>>;
}

impl JournalControl for Journal {
    fn resolve_path(&self, frn: u128, out: &mut [u16]) -> io::Result<usize> {
        Self::resolve_path(self, frn, out)
    }

    fn open_query(&self) -> io::Result<Box<dyn JournalQuery>> {
        Ok(Box::new(Self::open(self.guid_path(), DEFAULT_BUFFER)?))
    }

    fn spawn_sync(
        &self,
        journal_id: u64,
        next: i64,
        iocp: Arc<Iocp>,
    ) -> io::Result<Box<dyn SyncSource>> {
        Ok(Box::new(SyncReader::spawn(
            self.guid_path(),
            DEFAULT_BUFFER,
            journal_id,
            next,
            iocp,
            KEY_READ,
        )?))
    }
}

/// Name lookups for the feed: the control handle resolves reference numbers,
/// the lister walks directories.
struct Lookup<'a, L> {
    control: &'a dyn JournalControl,
    lister: L,
    root_path: &'a [u16],
    units: Vec<u16>,
}

impl<L: DirSource> Names for Lookup<'_, L> {
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
    reader: Box<dyn JournalRead>,
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
    Sync(Box<dyn SyncSource>),
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
            reader: Box::new(reader),
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
    fn read_loop<L: DirSource>(
        &mut self,
        st: &mut Loop,
        names: &mut Lookup<'_, L>,
    ) -> Option<io::Error> {
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
        if let Some(done) = self.drain_read(st.reader.as_ref()) {
            let _ = st.reader.finish(&done);
        }
    }

    /// Feeds one buffer, saves the position and publishes the counters.
    fn deliver<L: DirSource>(&mut self, st: &mut Loop, names: &mut Lookup<'_, L>) {
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
    fn handle(
        &mut self,
        error: io::Error,
        st: &mut Loop,
        control: &dyn JournalControl,
    ) -> Option<io::Error> {
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
        match control.spawn_sync(st.journal_id, st.next, Arc::clone(&self.iocp)) {
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
    fn drain_read(&self, reader: &dyn JournalRead) -> Option<Completion> {
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

    /// Decides what a failed read means: a gap (the journal is queried
    /// again and the read goes on from its end), the `ReFS` fallback, or the
    /// end of the source.
    fn recover(
        &mut self,
        error: &io::Error,
        control: &dyn JournalControl,
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
    fn requery(&self, control: &dyn JournalControl) -> Option<JournalData> {
        let mut query = control.open_query().ok()?;
        let deadline = Instant::now() + self.requery_for;
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
                    std::thread::sleep(self.requery_every);
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

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::collections::{HashMap, VecDeque};
    use std::path::PathBuf;
    use std::rc::Rc;

    use peekbuddy_core::event::{Event, Kind};
    use peekbuddy_core::reconcile::dirinfo::ATTR_DIRECTORY;
    use peekbuddy_core::reconcile::{ListError, Listing, Meta};

    use super::*;

    const ID: u64 = 42;
    const KEY_OTHER: usize = 9;

    fn units(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    /// A buffer that carries only the next USN.
    fn header(next: i64) -> Vec<u8> {
        next.to_le_bytes().to_vec()
    }

    fn os(code: i32) -> io::Error {
        io::Error::from_raw_os_error(code)
    }

    fn journal(id: u64, next: i64) -> JournalData {
        JournalData {
            journal_id: id,
            first_usn: 0,
            next_usn: next,
            lowest_valid_usn: 0,
        }
    }

    /// How a scripted read ends. `Plain` is an error without an OS code.
    #[derive(Clone)]
    enum Fail {
        Os(i32),
        Plain,
    }

    impl Fail {
        fn error(&self) -> io::Error {
            match self {
                Self::Os(code) => os(*code),
                Self::Plain => io::Error::other("no os code"),
            }
        }
    }

    /// What the next `begin_wait` of the fake does.
    enum Step {
        /// The call cannot start.
        Refuse(Fail),
        /// A read that finishes with these bytes.
        Data(Vec<u8>),
        /// A read that finishes with an error.
        Fails(Fail),
        /// A read and the stop message arrive together.
        DataAndStop(Vec<u8>),
    }

    type Log = Rc<RefCell<Vec<String>>>;
    type Replies = Rc<RefCell<VecDeque<Result<JournalData, i32>>>>;

    /// A journal reader that follows a script and then asks the loop to stop.
    struct FakeReader {
        iocp: Arc<Iocp>,
        steps: VecDeque<Step>,
        outcome: Option<Result<Vec<u8>, Fail>>,
        pending: Cell<bool>,
        buf: Vec<u8>,
        log: Log,
    }

    impl FakeReader {
        fn new(iocp: &Arc<Iocp>, steps: Vec<Step>, log: &Log) -> Self {
            Self {
                iocp: Arc::clone(iocp),
                steps: steps.into(),
                outcome: None,
                pending: Cell::new(false),
                buf: Vec::new(),
                log: Rc::clone(log),
            }
        }
    }

    impl JournalRead for FakeReader {
        fn begin_wait(&mut self, journal_id: u64, start_usn: i64) -> io::Result<()> {
            self.log
                .borrow_mut()
                .push(format!("begin {journal_id} {start_usn}"));
            match self.steps.pop_front() {
                Some(Step::Refuse(fail)) => return Err(fail.error()),
                Some(Step::Data(bytes)) => {
                    self.outcome = Some(Ok(bytes));
                    self.iocp.post(KEY_READ, 1, 0)?;
                }
                Some(Step::Fails(fail)) => {
                    self.outcome = Some(Err(fail));
                    self.iocp.post(KEY_READ, 0, 0)?;
                }
                Some(Step::DataAndStop(bytes)) => {
                    self.outcome = Some(Ok(bytes));
                    self.iocp.post(KEY_READ, 1, 0)?;
                    self.iocp.post(KEY_STOP, 0, 0)?;
                }
                None => {
                    // The script is over: the read stays pending and the
                    // loop is told to stop.
                    self.outcome = None;
                    self.iocp.post(KEY_STOP, 0, 0)?;
                }
            }
            self.pending.set(true);
            Ok(())
        }

        fn is_pending(&self) -> bool {
            self.pending.get()
        }

        fn cancel(&self) -> io::Result<CancelOutcome> {
            self.log.borrow_mut().push("cancel".into());
            // A cancelled read reports on the port.
            self.iocp.post(KEY_READ, 0, 0)?;
            Ok(CancelOutcome::Requested)
        }

        fn finish(&mut self, _completion: &Completion) -> io::Result<&[u8]> {
            self.log.borrow_mut().push("finish".into());
            self.pending.set(false);
            match self.outcome.take() {
                Some(Ok(bytes)) => {
                    self.buf = bytes;
                    Ok(&self.buf)
                }
                Some(Err(fail)) => Err(fail.error()),
                None => Err(os(ERROR_OPERATION_ABORTED)),
            }
        }
    }

    struct FakeQuery {
        replies: Replies,
        asked: Rc<Cell<usize>>,
    }

    impl JournalQuery for FakeQuery {
        fn query(&mut self) -> io::Result<JournalData> {
            self.asked.set(self.asked.get() + 1);
            match self.replies.borrow_mut().pop_front() {
                Some(Ok(data)) => Ok(data),
                Some(Err(code)) => Err(os(code)),
                None => Err(os(5)),
            }
        }
    }

    struct FakeSync {
        takes: RefCell<VecDeque<io::Result<Vec<u8>>>>,
        stopped: Rc<Cell<usize>>,
        iocp: Arc<Iocp>,
    }

    impl SyncSource for FakeSync {
        fn take(&self, out: &mut Vec<u8>) -> Option<io::Result<()>> {
            let taken = self.takes.borrow_mut().pop_front();
            // The last result is followed by the stop message; a failed read
            // is followed by what the loop does about it.
            if self.takes.borrow().is_empty() && !matches!(taken, Some(Err(_))) {
                self.iocp.post(KEY_STOP, 0, 0).unwrap();
            }
            match taken? {
                Ok(bytes) => {
                    *out = bytes;
                    Some(Ok(()))
                }
                Err(error) => Some(Err(error)),
            }
        }

        fn stop(&mut self) {
            self.stopped.set(self.stopped.get() + 1);
        }
    }

    /// One synchronous reader to hand out: what its takes give, or the
    /// error code of the spawn.
    type Spawn = Result<Vec<io::Result<Vec<u8>>>, i32>;

    #[derive(Default)]
    struct FakeControl {
        names: HashMap<u128, Vec<u16>>,
        replies: Replies,
        opens: RefCell<VecDeque<Result<(), i32>>>,
        asked: Rc<Cell<usize>>,
        spawns: RefCell<VecDeque<Spawn>>,
        spawned: RefCell<Vec<(u64, i64)>>,
        stopped: Rc<Cell<usize>>,
    }

    impl JournalControl for FakeControl {
        fn resolve_path(&self, frn: u128, out: &mut [u16]) -> io::Result<usize> {
            let name = self.names.get(&frn).ok_or_else(|| os(2))?;
            let dest = out.get_mut(..name.len()).ok_or_else(|| os(122))?;
            dest.copy_from_slice(name);
            Ok(name.len())
        }

        fn open_query(&self) -> io::Result<Box<dyn JournalQuery>> {
            match self.opens.borrow_mut().pop_front() {
                Some(Err(code)) => Err(os(code)),
                _ => Ok(Box::new(FakeQuery {
                    replies: Rc::clone(&self.replies),
                    asked: Rc::clone(&self.asked),
                })),
            }
        }

        fn spawn_sync(
            &self,
            journal_id: u64,
            next: i64,
            iocp: Arc<Iocp>,
        ) -> io::Result<Box<dyn SyncSource>> {
            self.spawned.borrow_mut().push((journal_id, next));
            let takes = self.spawns.borrow_mut().pop_front().unwrap_or(Ok(vec![]));
            let takes = takes.map_err(os)?;
            // Every result is announced on the port.
            for _ in &takes {
                iocp.post(KEY_READ, 1, 0)?;
            }
            if takes.is_empty() {
                iocp.post(KEY_STOP, 0, 0)?;
            }
            Ok(Box::new(FakeSync {
                takes: RefCell::new(takes.into()),
                stopped: Rc::clone(&self.stopped),
                iocp,
            }))
        }
    }

    /// What the sink saw.
    #[derive(Default)]
    struct Seen {
        events: Vec<Event>,
        batch_ends: usize,
    }

    struct FakeSink(Arc<Mutex<Seen>>);

    impl EventSink for FakeSink {
        fn push(&mut self, event: &Event, _name: &[u16]) -> bool {
            self.0.lock().unwrap().events.push(*event);
            true
        }

        fn batch_end(&mut self) {
            self.0.lock().unwrap().batch_ends += 1;
        }
    }

    struct FakeCheckpoints(Arc<Mutex<Vec<Checkpoint>>>);

    impl Checkpoints for FakeCheckpoints {
        fn load(&self) -> Option<Checkpoint> {
            None
        }

        fn save(&self, checkpoint: Checkpoint) {
            self.0.lock().unwrap().push(checkpoint);
        }
    }

    #[derive(Default)]
    struct FakeDirs {
        dirs: HashMap<String, Vec<(&'static str, u128)>>,
        asked: Vec<String>,
    }

    impl DirSource for FakeDirs {
        fn list(&mut self, path: &[u16], out: &mut Listing) -> Result<(), ListError> {
            let key = String::from_utf16_lossy(path);
            self.asked.push(key.clone());
            for (name, id) in self.dirs.get(&key).ok_or(ListError::Gone)? {
                let meta = Meta {
                    file_id: *id,
                    attributes: ATTR_DIRECTORY,
                    ..Meta::default()
                };
                out.push(&units(name), meta, 0);
            }
            Ok(())
        }

        fn exists(&mut self, path: &[u16]) -> bool {
            self.dirs.contains_key(&String::from_utf16_lossy(path))
        }
    }

    fn clock() -> (u64, u64) {
        (7, 9)
    }

    /// A job and a loop state on a fake journal, and what they report to.
    struct Rig {
        job: Job,
        st: Loop,
        control: FakeControl,
        log: Log,
        seen: Arc<Mutex<Seen>>,
        saved: Arc<Mutex<Vec<Checkpoint>>>,
        gaps: Arc<Mutex<Vec<GapReason>>>,
    }

    impl Rig {
        fn new(steps: Vec<Step>) -> Self {
            let iocp = Arc::new(Iocp::new(1).unwrap());
            let log = Log::default();
            let seen = Arc::<Mutex<Seen>>::default();
            let saved = Arc::<Mutex<Vec<Checkpoint>>>::default();
            let gaps = Arc::<Mutex<Vec<GapReason>>>::default();
            let on_gap: OnGap = {
                let gaps = Arc::clone(&gaps);
                Box::new(move |reason| gaps.lock().unwrap().push(reason))
            };
            let job = Job {
                vol: VolIdx(1),
                root: RootIdx(2),
                path: PathBuf::new(),
                checkpoints: Box::new(FakeCheckpoints(Arc::clone(&saved))),
                sink: Box::new(FakeSink(Arc::clone(&seen))),
                on_gap,
                clock,
                iocp: Arc::clone(&iocp),
                stop: Arc::new(AtomicBool::new(false)),
                counters: Arc::default(),
                synchronous: Arc::default(),
                requery_for: Duration::from_millis(40),
                requery_every: Duration::from_millis(1),
            };
            let st = Loop {
                reader: Box::new(FakeReader::new(&iocp, steps, &log)),
                mode: Reader::Overlapped,
                feed: Feed::new(Scope::new(1), VolIdx(1), RootIdx(2), &units(r"\root")),
                next: 100,
                journal_id: ID,
                batch: Batch::new(4),
                bytes: Vec::new(),
            };
            Self {
                job,
                st,
                control: FakeControl::default(),
                log,
                seen,
                saved,
                gaps,
            }
        }

        /// Runs the loop to its end. A loop that does not end within 5 s is
        /// stopped, so a broken loop fails its test's assertions instead of
        /// waiting on the port for ever.
        fn run(&mut self) -> Option<io::Error> {
            let (done, watch) = mpsc::channel::<()>();
            let guard = {
                let iocp = Arc::clone(&self.job.iocp);
                let stop = Arc::clone(&self.job.stop);
                std::thread::spawn(move || {
                    if watch.recv_timeout(Duration::from_secs(5))
                        == Err(mpsc::RecvTimeoutError::Timeout)
                    {
                        stop.store(true, Ordering::SeqCst);
                        let _ = iocp.post(KEY_STOP, 0, 0);
                    }
                })
            };
            let mut names = Lookup {
                control: &self.control,
                lister: FakeDirs::default(),
                root_path: &[],
                units: Vec::new(),
            };
            let result = self.job.read_loop(&mut self.st, &mut names);
            drop(done);
            guard.join().unwrap();
            result
        }

        fn saved(&self) -> Vec<(u64, i64)> {
            self.saved
                .lock()
                .unwrap()
                .iter()
                .map(|c| (c.journal_id, c.next_usn))
                .collect()
        }

        fn gaps(&self) -> Vec<GapReason> {
            self.gaps.lock().unwrap().clone()
        }

        fn gap_rows(&self) -> Vec<GapReason> {
            self.seen
                .lock()
                .unwrap()
                .events
                .iter()
                .filter_map(|event| match event.kind {
                    Kind::Overflow(reason) => Some(reason),
                    _ => None,
                })
                .collect()
        }

        fn calls(&self, prefix: &str) -> Vec<String> {
            self.log
                .borrow()
                .iter()
                .filter(|line| line.starts_with(prefix))
                .cloned()
                .collect()
        }
    }

    const NONE: [&str; 0] = [];

    #[test]
    fn only_the_refused_overlapped_read_asks_for_the_synchronous_reader() {
        let refused = io::Error::from_raw_os_error(1784);
        assert!(needs_sync_error(&refused));
        assert!(!needs_sync_error(&io::Error::from_raw_os_error(5)));
        assert!(!needs_sync_error(&io::Error::other("no os code")));
        assert!(!needs_sync_error(&io::Error::from_raw_os_error(-1784)));
    }

    #[test]
    fn buffers_are_fed_saved_and_counted_until_the_stop() {
        let mut rig = Rig::new(vec![Step::Data(header(500)), Step::Data(header(900))]);
        assert!(rig.run().is_none());
        assert_eq!(rig.saved(), [(ID, 500), (ID, 900)]);
        // Each read starts where the last buffer ended; the third is the
        // one the stop interrupts.
        assert_eq!(
            rig.calls("begin"),
            ["begin 42 100", "begin 42 500", "begin 42 900"]
        );
        assert_eq!(rig.st.next, 900);
        assert_eq!(rig.job.counters.lock().unwrap().batches, 2);
        assert_eq!(rig.gaps(), []);
        // The stop cancelled the pending read and took its completion.
        assert_eq!(rig.calls("cancel").len(), 1);
        assert!(!rig.st.reader.is_pending());
    }

    #[test]
    fn a_stopped_runner_reads_nothing() {
        let mut rig = Rig::new(vec![Step::Data(header(500))]);
        rig.job.stop.store(true, Ordering::SeqCst);
        assert!(rig.run().is_none());
        assert_eq!(rig.calls("begin"), NONE);
        assert_eq!(rig.saved(), []);
    }

    #[test]
    fn a_cancelled_read_is_not_an_error() {
        let mut rig = Rig::new(vec![Step::Fails(Fail::Os(995)), Step::Data(header(500))]);
        assert!(rig.run().is_none());
        assert_eq!(rig.saved(), [(ID, 500)]);
        assert_eq!(rig.gaps(), []);
        assert_eq!(rig.gap_rows(), []);
    }

    #[test]
    fn an_unknown_read_error_ends_the_source_with_that_error() {
        let mut rig = Rig::new(vec![Step::Fails(Fail::Os(5)), Step::Data(header(500))]);
        let error = rig.run().unwrap();
        assert_eq!(error.raw_os_error(), Some(5));
        assert_eq!(rig.saved(), []);
        assert_eq!(rig.gaps(), []);
        assert_eq!(rig.gap_rows(), []);
    }

    #[test]
    fn an_error_without_an_os_code_ends_the_source() {
        let mut rig = Rig::new(vec![Step::Fails(Fail::Plain)]);
        let error = rig.run().unwrap();
        assert_eq!(error.raw_os_error(), None);
        assert_eq!(rig.gaps(), []);
    }

    #[test]
    fn a_read_that_cannot_start_ends_the_source_with_its_error() {
        let mut rig = Rig::new(vec![Step::Refuse(Fail::Os(6))]);
        assert_eq!(rig.run().unwrap().raw_os_error(), Some(6));
    }

    #[test]
    fn a_wrapped_journal_writes_one_gap_and_reads_on_from_its_end() {
        let mut rig = Rig::new(vec![Step::Fails(Fail::Os(1181)), Step::Data(header(4100))]);
        rig.control
            .replies
            .borrow_mut()
            .push_back(Ok(journal(77, 4000)));
        assert!(rig.run().is_none());
        assert_eq!(rig.gap_rows(), [GapReason::JournalEntryDeleted]);
        assert_eq!(rig.gaps(), [GapReason::JournalEntryDeleted]);
        assert_eq!(rig.control.asked.get(), 1);
        // The position and the journal id come from the new query, and are
        // saved before the next read.
        assert_eq!(rig.saved(), [(77, 4000), (77, 4100)]);
        assert_eq!(
            rig.calls("begin"),
            ["begin 42 100", "begin 77 4000", "begin 77 4100"]
        );
        // The overlapped reader stays; no synchronous reader was started.
        assert!(rig.control.spawned.borrow().is_empty());
        assert!(!rig.job.synchronous.load(Ordering::SeqCst));
        assert!(matches!(rig.st.mode, Reader::Overlapped));
    }

    #[test]
    fn a_journal_that_is_not_active_is_a_changed_journal_and_is_waited_for() {
        let mut rig = Rig::new(vec![Step::Refuse(Fail::Os(1179)), Step::Data(header(10))]);
        {
            let mut replies = rig.control.replies.borrow_mut();
            replies.push_back(Err(1179));
            replies.push_back(Err(1178));
            replies.push_back(Ok(journal(78, 5)));
        }
        assert!(rig.run().is_none());
        assert_eq!(rig.gap_rows(), [GapReason::JournalIdChanged]);
        assert_eq!(rig.gaps(), [GapReason::JournalIdChanged]);
        assert_eq!(rig.control.asked.get(), 3);
        assert_eq!(rig.saved(), [(78, 5), (78, 10)]);
        assert_eq!(rig.calls("begin")[1], "begin 78 5");
    }

    #[test]
    fn a_journal_that_stays_gone_ends_the_source_with_the_read_error() {
        let mut rig = Rig::new(vec![Step::Fails(Fail::Os(1178))]);
        for _ in 0..1000 {
            rig.control.replies.borrow_mut().push_back(Err(1178));
        }
        let error = rig.run().unwrap();
        assert_eq!(error.raw_os_error(), Some(1178));
        // The gap is recorded first, then the wait runs out.
        assert_eq!(rig.gaps(), [GapReason::JournalIdChanged]);
        assert!(rig.control.asked.get() > 2);
        assert_eq!(rig.saved(), []);
    }

    #[test]
    fn a_journal_that_cannot_be_queried_ends_the_source_after_the_gap() {
        let mut rig = Rig::new(vec![Step::Fails(Fail::Os(1181))]);
        // ERROR_INVALID_FUNCTION: the volume has no journal after all.
        rig.control.replies.borrow_mut().push_back(Err(1));
        let error = rig.run().unwrap();
        assert_eq!(error.raw_os_error(), Some(1181));
        assert_eq!(rig.gaps(), [GapReason::JournalEntryDeleted]);
        assert_eq!(rig.control.asked.get(), 1);
        assert_eq!(rig.saved(), []);
    }

    #[test]
    fn a_journal_that_cannot_be_reopened_ends_the_source() {
        let mut rig = Rig::new(vec![Step::Fails(Fail::Os(1181))]);
        rig.control.opens.borrow_mut().push_back(Err(5));
        assert_eq!(rig.run().unwrap().raw_os_error(), Some(1181));
        assert_eq!(rig.control.asked.get(), 0);
    }

    #[test]
    fn the_wait_for_a_new_journal_ends_with_its_deadline() {
        let rig = Rig::new(vec![]);
        for _ in 0..1000 {
            rig.control.replies.borrow_mut().push_back(Err(1179));
        }
        let started = Instant::now();
        assert!(rig.job.requery(&rig.control).is_none());
        assert!(started.elapsed() >= Duration::from_millis(40));
        assert!(rig.control.asked.get() > 2);
    }

    #[test]
    fn a_requery_gives_up_at_once_for_another_error() {
        let rig = Rig::new(vec![]);
        rig.control.replies.borrow_mut().push_back(Err(5));
        rig.control
            .replies
            .borrow_mut()
            .push_back(Ok(journal(1, 1)));
        assert!(rig.job.requery(&rig.control).is_none());
        assert_eq!(rig.control.asked.get(), 1);
    }

    #[test]
    fn a_requery_gives_up_at_a_stop() {
        let rig = Rig::new(vec![]);
        rig.job.stop.store(true, Ordering::SeqCst);
        rig.control.replies.borrow_mut().push_back(Err(1179));
        rig.control
            .replies
            .borrow_mut()
            .push_back(Ok(journal(1, 1)));
        assert!(rig.job.requery(&rig.control).is_none());
        assert_eq!(rig.control.asked.get(), 1);
    }

    #[test]
    fn a_requery_returns_the_data_it_finds() {
        let rig = Rig::new(vec![]);
        rig.control
            .replies
            .borrow_mut()
            .push_back(Ok(journal(5, 6)));
        assert_eq!(rig.job.requery(&rig.control), Some(journal(5, 6)));
    }

    #[test]
    fn the_refused_overlapped_read_switches_to_the_synchronous_reader() {
        let mut rig = Rig::new(vec![Step::Fails(Fail::Os(1784))]);
        rig.control
            .spawns
            .borrow_mut()
            .push_back(Ok(vec![Ok(header(600))]));
        assert!(rig.run().is_none());
        // No gap: the read was refused, nothing was lost.
        assert_eq!(rig.gap_rows(), []);
        assert_eq!(rig.control.asked.get(), 0);
        assert_eq!(*rig.control.spawned.borrow(), [(ID, 100)]);
        assert!(rig.job.synchronous.load(Ordering::SeqCst));
        assert!(matches!(rig.st.mode, Reader::Sync(_)));
        // The synchronous result was fed; the overlapped reader started
        // nothing more.
        assert_eq!(rig.saved(), [(ID, 600)]);
        assert_eq!(rig.calls("begin").len(), 1);
    }

    #[test]
    fn the_refusal_at_the_start_of_a_read_switches_too() {
        let mut rig = Rig::new(vec![Step::Refuse(Fail::Os(1784))]);
        assert!(rig.run().is_none());
        assert!(rig.job.synchronous.load(Ordering::SeqCst));
        assert_eq!(rig.control.spawned.borrow().len(), 1);
    }

    #[test]
    fn a_synchronous_reader_that_fails_to_start_ends_the_source() {
        let mut rig = Rig::new(vec![Step::Fails(Fail::Os(1784))]);
        rig.control.spawns.borrow_mut().push_back(Err(5));
        assert_eq!(rig.run().unwrap().raw_os_error(), Some(5));
        assert!(!rig.job.synchronous.load(Ordering::SeqCst));
        assert!(matches!(rig.st.mode, Reader::Overlapped));
    }

    #[test]
    fn a_wrap_on_a_synchronous_volume_restarts_the_synchronous_reader() {
        let mut rig = Rig::new(vec![]);
        let stopped = Rc::clone(&rig.control.stopped);
        rig.st.mode = Reader::Sync(Box::new(FakeSync {
            takes: RefCell::new(VecDeque::from([Err(os(1181))])),
            stopped: Rc::clone(&stopped),
            iocp: Arc::clone(&rig.job.iocp),
        }));
        rig.job.iocp.post(KEY_READ, 1, 0).unwrap();
        rig.control
            .replies
            .borrow_mut()
            .push_back(Ok(journal(77, 4000)));
        rig.control
            .spawns
            .borrow_mut()
            .push_back(Ok(vec![Ok(header(4100))]));
        assert!(rig.run().is_none());
        assert_eq!(rig.gap_rows(), [GapReason::JournalEntryDeleted]);
        // The old thread ended with the error and is stopped; the new one
        // starts from the new journal.
        assert_eq!(stopped.get(), 1);
        assert_eq!(*rig.control.spawned.borrow(), [(77, 4000)]);
        assert_eq!(rig.saved(), [(77, 4000), (77, 4100)]);
        // A synchronous volume never starts an overlapped read.
        assert_eq!(rig.calls("begin"), NONE);
    }

    #[test]
    fn an_empty_synchronous_result_is_taken_as_an_empty_buffer() {
        let mut rig = Rig::new(vec![]);
        rig.st.mode = Reader::Sync(Box::new(FakeSync {
            takes: RefCell::new(VecDeque::new()),
            stopped: Rc::default(),
            iocp: Arc::clone(&rig.job.iocp),
        }));
        rig.job.iocp.post(KEY_READ, 1, 0).unwrap();
        // Nothing was taken: the buffer has no header, which is a parser
        // gap, not an error of the source.
        assert!(rig.run().is_none());
        assert_eq!(rig.gaps(), [GapReason::ParserError]);
    }

    #[test]
    fn a_buffer_without_a_header_asks_for_a_reconcile_and_a_gap_row() {
        let mut rig = Rig::new(vec![Step::Data(vec![1, 2, 3]), Step::Data(header(900))]);
        assert!(rig.run().is_none());
        assert_eq!(rig.gaps(), [GapReason::ParserError]);
        // The position did not move for the broken buffer; the row is owed
        // and goes out with the next one.
        assert_eq!(rig.saved(), [(ID, 900)]);
        assert_eq!(rig.calls("begin")[1], "begin 42 100");
        assert_eq!(rig.gap_rows(), [GapReason::ParserError]);
        let counters = *rig.job.counters.lock().unwrap();
        assert_eq!(counters.parse_errors, 1);
        assert_eq!(counters.gap_rows, 1);
        assert_eq!(counters.batches, 2);
    }

    #[test]
    fn a_truncated_record_is_counted_and_the_position_still_moves() {
        let mut buffer = header(700);
        buffer.extend_from_slice(&[0x40, 0, 0, 0, 2, 0]);
        let mut rig = Rig::new(vec![Step::Data(buffer)]);
        assert!(rig.run().is_none());
        assert_eq!(rig.saved(), [(ID, 700)]);
        assert_eq!(rig.job.counters.lock().unwrap().parse_errors, 1);
        assert_eq!(rig.gap_rows(), [GapReason::ParserError]);
    }

    #[test]
    fn a_completion_that_comes_with_the_stop_is_finished_without_a_cancel() {
        let mut rig = Rig::new(vec![Step::DataAndStop(header(500))]);
        assert!(rig.run().is_none());
        assert_eq!(rig.calls("cancel"), NONE);
        assert_eq!(rig.calls("finish").len(), 1);
        assert!(!rig.st.reader.is_pending());
        // The buffer that came with the stop is not fed.
        assert_eq!(rig.saved(), []);
    }

    #[test]
    fn a_synchronous_reader_has_no_pending_read_to_end_at_a_stop() {
        let mut rig = Rig::new(vec![Step::Data(header(1))]);
        rig.st.reader.begin_wait(ID, 0).unwrap();
        rig.st.mode = Reader::Sync(Box::new(FakeSync {
            takes: RefCell::new(VecDeque::new()),
            stopped: Rc::default(),
            iocp: Arc::clone(&rig.job.iocp),
        }));
        rig.job.finish_pending(&mut rig.st, None);
        assert_eq!(rig.calls("cancel"), NONE);
        assert_eq!(rig.calls("finish"), NONE);
    }

    #[test]
    fn an_idle_reader_has_nothing_to_end_at_a_stop() {
        let mut rig = Rig::new(vec![]);
        rig.job.finish_pending(&mut rig.st, None);
        assert_eq!(rig.calls("cancel"), NONE);
        assert_eq!(rig.calls("finish"), NONE);
    }

    #[test]
    fn the_wait_takes_the_read_and_skips_other_keys() {
        let mut rig = Rig::new(vec![]);
        rig.job.iocp.post(KEY_OTHER, 0, 0).unwrap();
        rig.job.iocp.post(KEY_READ, 3, 0).unwrap();
        let Waited::Read(completion) = rig.job.wait(&mut rig.st) else {
            unreachable!("the wait found no read");
        };
        assert_eq!((completion.key, completion.bytes), (KEY_READ, 3));
    }

    #[test]
    fn a_drain_with_nothing_pending_has_nothing_to_wait_for() {
        let rig = Rig::new(vec![]);
        assert!(rig.job.drain_read(rig.st.reader.as_ref()).is_none());
    }

    fn lookup<'a>(
        control: &'a FakeControl,
        dirs: FakeDirs,
        root: &'a [u16],
    ) -> Lookup<'a, FakeDirs> {
        Lookup {
            control,
            lister: dirs,
            root_path: root,
            units: Vec::new(),
        }
    }

    #[test]
    fn a_name_is_resolved_through_the_control_handle() {
        let mut control = FakeControl::default();
        control.names.insert(7, units(r"\root\a.txt"));
        let root = units(r"\root");
        let mut names = lookup(&control, FakeDirs::default(), &root);
        let mut out = units("stale");
        assert!(names.path_of(7, &mut out));
        assert_eq!(String::from_utf16_lossy(&out), r"\root\a.txt");
        assert!(!names.path_of(8, &mut out));
    }

    #[test]
    fn a_directory_that_moved_in_is_walked_below_the_root() {
        let mut control = FakeControl::default();
        control.names.insert(30, units(r"\root\moved"));
        let root = units(r"\root");
        let mut dirs = FakeDirs::default();
        dirs.dirs.insert("moved".into(), vec![("inner", 31)]);
        dirs.dirs.insert(r"moved\inner".into(), vec![]);
        let mut names = lookup(&control, dirs, &root);
        let mut scope = Scope::new(1);
        scope.insert(30, 1, &units("moved"));
        names.walk(30, &mut scope);
        assert!(scope.contains(31));
        assert_eq!(names.lister.asked, ["moved", r"moved\inner"]);
    }

    #[test]
    fn nothing_is_walked_for_a_directory_that_cannot_be_named_or_is_elsewhere() {
        let mut control = FakeControl::default();
        control.names.insert(40, units(r"\elsewhere\dir"));
        let root = units(r"\root");
        let mut dirs = FakeDirs::default();
        dirs.dirs.insert("dir".into(), vec![("inner", 41)]);
        let mut names = lookup(&control, dirs, &root);
        let mut scope = Scope::new(1);
        // Not resolvable.
        names.walk(39, &mut scope);
        // Resolvable, but outside the root.
        names.walk(40, &mut scope);
        assert!(!scope.contains(41));
        assert_eq!(names.lister.asked.len(), 0);
    }
}
