//! The synchronous journal reader for `ReFS` and Dev Drive volumes (#212).
//!
//! On those volumes the overlapped read is refused with
//! `ERROR_INVALID_USER_BUFFER` (1784, see
//! `peekbuddy_core::usn::checkpoint::needs_synchronous_reads`). The volume is
//! then opened without `FILE_FLAG_OVERLAPPED` and one dedicated thread blocks
//! in the waiting read (`BytesToWaitFor = 1`, so an idle volume costs no
//! wakeups). Each result is handed over under a lock and announced on the
//! engine's completion port, so the consumer loop is the same as for the
//! overlapped reader. [`SyncReader::stop`] sets a stop flag and calls
//! `CancelSynchronousIo` on the thread until it has left the kernel.

use std::io;
use std::os::windows::io::AsRawHandle as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use windows_sys::Win32::System::IO::{CancelSynchronousIo, DeviceIoControl};
use windows_sys::Win32::System::Ioctl::{
    FSCTL_READ_UNPRIVILEGED_USN_JOURNAL, READ_USN_JOURNAL_DATA_V1,
};

use super::{ALL_REASONS, MAX_MAJOR, MIN_MAJOR, request_bytes};
use crate::io::dir::DirHandle;
use crate::io::iocp::Iocp;
use crate::io::open::OpenOptions;

/// How often [`SyncReader::stop`] repeats the cancel while the thread has
/// not left yet. The cancel is lost when it lands before the thread entered
/// the call, so it is repeated; this is the shutdown path, not an idle timer.
const STOP_RETRY: Duration = Duration::from_millis(5);

#[derive(Debug)]
struct Handoff {
    /// `Some` while a result waits to be taken.
    ready: Option<io::Result<()>>,
    /// The bytes of that result: the next USN, then records.
    buf: Vec<u8>,
}

#[derive(Debug)]
struct Shared {
    stop: AtomicBool,
    handoff: Mutex<Handoff>,
    changed: Condvar,
}

fn lock(shared: &Shared) -> MutexGuard<'_, Handoff> {
    shared
        .handoff
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// A journal read loop on its own thread.
#[derive(Debug)]
pub struct SyncReader {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl SyncReader {
    /// Opens the volume at `guid_path` for blocking reads and starts the
    /// thread, reading from `start_usn`. Every result posts a completion with
    /// `key`, zero `overlapped` and the byte count to `iocp`; [`take`] gets
    /// the bytes.
    ///
    /// [`take`]: SyncReader::take
    ///
    /// # Errors
    ///
    /// The OS error when the volume cannot be opened or the thread started.
    pub fn spawn(
        guid_path: &[u16],
        buffer_bytes: usize,
        journal_id: u64,
        start_usn: i64,
        iocp: Arc<Iocp>,
        key: usize,
    ) -> io::Result<Self> {
        let volume = DirHandle::open_synchronous(guid_path, OpenOptions::default())?;
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            handoff: Mutex::new(Handoff {
                ready: None,
                buf: Vec::new(),
            }),
            changed: Condvar::new(),
        });
        let worker = Worker {
            shared: Arc::clone(&shared),
            volume,
            iocp,
            key,
            journal_id,
            capacity: buffer_bytes,
        };
        let thread = std::thread::Builder::new()
            .name("pb-usn-sync".into())
            .spawn(move || worker.run(start_usn))?;
        Ok(Self {
            shared,
            thread: Some(thread),
        })
    }

    /// Takes the result announced on the port: swaps the bytes into `out`
    /// (whose old buffer is reused for the next read, so nothing is
    /// allocated per batch once the buffers have grown) and lets the thread
    /// read on. `None` when no result is waiting.
    ///
    /// # Errors
    ///
    /// The OS error of a failed read; the thread has ended then and the
    /// caller reopens the journal (and records a gap if it must).
    pub fn take(&self, out: &mut Vec<u8>) -> Option<io::Result<()>> {
        let mut handoff = lock(&self.shared);
        let result = handoff.ready.take()?;
        std::mem::swap(&mut handoff.buf, out);
        drop(handoff);
        self.shared.changed.notify_all();
        Some(result)
    }

    /// Ends the thread: sets the stop flag and cancels its blocking read
    /// until it has left, then joins it. Idempotent.
    pub fn stop(&mut self) {
        let Some(thread) = self.thread.take() else {
            return;
        };
        {
            let _guard = lock(&self.shared);
            self.shared.stop.store(true, Ordering::SeqCst);
        }
        self.shared.changed.notify_all();
        while !thread.is_finished() {
            // SAFETY: the handle belongs to `thread`, which is joined only
            // after this loop, so it is live; the call has no other input.
            // It fails when the thread is in no synchronous call, which is
            // the case when it waits on the condvar; the flag is set then.
            let _ = unsafe { CancelSynchronousIo(thread.as_raw_handle()) };
            self.shared.changed.notify_all();
            std::thread::sleep(STOP_RETRY);
        }
        let _ = thread.join();
    }
}

impl Drop for SyncReader {
    fn drop(&mut self) {
        self.stop();
    }
}

struct Worker {
    shared: Arc<Shared>,
    volume: DirHandle,
    iocp: Arc<Iocp>,
    key: usize,
    journal_id: u64,
    capacity: usize,
}

impl Worker {
    fn stopping(&self) -> bool {
        self.shared.stop.load(Ordering::SeqCst)
    }

    fn run(self, mut next: i64) {
        let mut buf: Vec<u8> = Vec::new();
        while !self.stopping() {
            buf.resize(self.capacity, 0);
            let result = self.read(next, &mut buf);
            if self.stopping() {
                return;
            }
            let (bytes, outcome) = match result {
                Ok(bytes) => {
                    buf.truncate(bytes);
                    // The first eight bytes are the USN to continue from.
                    if let Some(head) = buf.get(..8).and_then(|b| <[u8; 8]>::try_from(b).ok()) {
                        next = i64::from_le_bytes(head);
                    }
                    (bytes, Ok(()))
                }
                Err(error) => (0, Err(error)),
            };
            let failed = outcome.is_err();
            {
                let mut handoff = lock(&self.shared);
                std::mem::swap(&mut handoff.buf, &mut buf);
                handoff.ready = Some(outcome);
            }
            if self
                .iocp
                .post(self.key, u32::try_from(bytes).unwrap_or(0), 0)
                .is_err()
            {
                return;
            }
            // Wait until the consumer has taken the result (or we must go).
            let mut handoff = lock(&self.shared);
            while handoff.ready.is_some() && !self.stopping() {
                handoff = self
                    .shared
                    .changed
                    .wait(handoff)
                    .unwrap_or_else(PoisonError::into_inner);
            }
            drop(handoff);
            if failed {
                return;
            }
        }
    }

    /// One blocking read that returns once at least one record exists.
    fn read(&self, start_usn: i64, buf: &mut [u8]) -> io::Result<usize> {
        let request = READ_USN_JOURNAL_DATA_V1 {
            StartUsn: start_usn,
            ReasonMask: ALL_REASONS,
            ReturnOnlyOnClose: 0,
            Timeout: 0,
            BytesToWaitFor: 1,
            UsnJournalID: self.journal_id,
            MinMajorVersion: MIN_MAJOR,
            MaxMajorVersion: MAX_MAJOR,
        };
        let input = request_bytes(&request);
        let input_len = u32::try_from(input.len()).unwrap_or(u32::MAX);
        let out_len = u32::try_from(buf.len()).unwrap_or(u32::MAX);
        let mut returned: u32 = 0;
        // SAFETY: the handle is live (owned by the worker) and was opened
        // without FILE_FLAG_OVERLAPPED, so the call blocks and no OVERLAPPED
        // is passed; `input` is readable for `input_len` bytes and `buf` is
        // writable for `out_len` bytes for the whole call, which returns
        // before either is touched again.
        let ok = unsafe {
            DeviceIoControl(
                self.volume.as_raw_handle(),
                FSCTL_READ_UNPRIVILEGED_USN_JOURNAL,
                input.as_ptr().cast(),
                input_len,
                buf.as_mut_ptr().cast(),
                out_len,
                &raw mut returned,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(usize::try_from(returned).unwrap_or(0))
    }
}
