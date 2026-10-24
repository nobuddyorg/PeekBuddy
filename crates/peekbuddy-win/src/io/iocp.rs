//! An I/O completion port (#43).
//!
//! One thread blocks in [`Iocp::dequeue`] with no timeout and wakes only for
//! a completion or a posted control message: an idle engine has no periodic
//! wakeups. Dequeueing reuses a [`Batch`] allocated once, so nothing here
//! allocates after arming.

use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle as _, OwnedHandle};
use std::time::Duration;

use super::dir::DirHandle;
use super::error::WinError;
use super::sys::{
    CreateIoCompletionPort, GetQueuedCompletionStatusEx, INVALID_HANDLE_VALUE, OVERLAPPED,
    OVERLAPPED_ENTRY, PostQueuedCompletionStatus, RtlNtStatusToDosError, WAIT_TIMEOUT,
};

/// `INFINITE`.
const INFINITE: u32 = u32::MAX;

/// One dequeued completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Completion {
    /// The key the handle was associated with, or the key of a posted
    /// message.
    pub key: usize,
    /// The address of the OVERLAPPED (see
    /// [`IoSlot::overlapped_addr`](super::slot::IoSlot::overlapped_addr)),
    /// or what a posted message carried.
    pub overlapped: usize,
    /// The bytes transferred.
    pub bytes: u32,
    /// The result of the I/O; a posted message is [`WinError::Other`] of 0.
    pub status: WinError,
}

impl Completion {
    /// Whether the I/O succeeded.
    #[must_use]
    pub const fn is_success(&self) -> bool {
        self.status.code() == 0
    }
}

/// A reusable array of completions for [`Iocp::dequeue`].
pub struct Batch {
    entries: Box<[OVERLAPPED_ENTRY]>,
    len: usize,
}

impl std::fmt::Debug for Batch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Batch")
            .field("capacity", &self.entries.len())
            .field("len", &self.len)
            .finish()
    }
}

impl Batch {
    /// A batch that takes up to `capacity` completions at once (at least 1).
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: vec![OVERLAPPED_ENTRY::default(); capacity.max(1)].into_boxed_slice(),
            len: 0,
        }
    }

    /// The completions of the last dequeue.
    pub fn iter(&self) -> impl Iterator<Item = Completion> + '_ {
        self.entries.iter().take(self.len).map(|entry| {
            // The NTSTATUS sits in the low 32 bits of `Internal`.
            let status = u32::try_from(entry.Internal & 0xFFFF_FFFF).unwrap_or(0);
            Completion {
                key: entry.lpCompletionKey,
                overlapped: entry.lpOverlapped as usize,
                bytes: entry.dwNumberOfBytesTransferred,
                // SAFETY: RtlNtStatusToDosError only maps a number.
                status: WinError::from_code(unsafe { RtlNtStatusToDosError(status.cast_signed()) }),
            }
        })
    }

    /// The number of completions of the last dequeue.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether the last dequeue returned nothing.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// A completion port, closed on drop.
#[derive(Debug)]
pub struct Iocp {
    handle: OwnedHandle,
}

impl Iocp {
    /// A new port; `concurrency` is the number of threads that may run at
    /// once (0 means the number of processors).
    ///
    /// # Errors
    ///
    /// The OS error when the port cannot be created.
    pub fn new(concurrency: u32) -> io::Result<Self> {
        // SAFETY: creates a new port; no file handle is passed.
        let raw = unsafe {
            CreateIoCompletionPort(INVALID_HANDLE_VALUE, std::ptr::null_mut(), 0, concurrency)
        };
        // This API reports failure with NULL, not INVALID_HANDLE_VALUE.
        if raw.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the call succeeded, so `raw` is a new handle we own.
        let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
        Ok(Self { handle })
    }

    /// Makes completions of `dir` arrive on this port with `key`.
    ///
    /// # Errors
    ///
    /// The OS error when the association fails (for example a handle opened
    /// without `FILE_FLAG_OVERLAPPED`).
    pub fn associate(&self, dir: &DirHandle, key: usize) -> io::Result<()> {
        // SAFETY: both handles are live for the call.
        let raw = unsafe {
            CreateIoCompletionPort(dir.as_raw_handle(), self.handle.as_raw_handle(), key, 0)
        };
        if raw.is_null() {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    /// Posts a control message: it comes out of [`Iocp::dequeue`] with this
    /// key, byte count and `overlapped` value.
    ///
    /// # Errors
    ///
    /// The OS error when the message cannot be queued.
    pub fn post(&self, key: usize, bytes: u32, overlapped: usize) -> io::Result<()> {
        // SAFETY: the port is live; the OVERLAPPED value is only handed back
        // to the dequeuer, never dereferenced by the kernel.
        let ok = unsafe {
            PostQueuedCompletionStatus(
                self.handle.as_raw_handle(),
                bytes,
                key,
                overlapped as *const OVERLAPPED,
            )
        };
        if ok == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    /// Waits for completions and fills `batch`; returns how many. `None` waits
    /// without a timeout; a timeout with nothing to report returns 0.
    ///
    /// # Errors
    ///
    /// The OS error for a failure other than a timeout.
    pub fn dequeue(&self, batch: &mut Batch, timeout: Option<Duration>) -> io::Result<usize> {
        let millis = timeout.map_or(INFINITE, |t| {
            // INFINITE is u32::MAX: a long timeout stops one short of it.
            u32::try_from(t.as_millis()).map_or(INFINITE - 1, |ms| ms.min(INFINITE - 1))
        });
        let capacity = u32::try_from(batch.entries.len()).unwrap_or(u32::MAX);
        let mut removed = 0_u32;
        // SAFETY: the port is live; `entries` holds `capacity` entries;
        // `removed` is a valid out pointer; not alertable.
        let ok = unsafe {
            GetQueuedCompletionStatusEx(
                self.handle.as_raw_handle(),
                batch.entries.as_mut_ptr(),
                capacity,
                &raw mut removed,
                millis,
                0,
            )
        };
        if ok == 0 {
            let error = io::Error::last_os_error();
            batch.len = 0;
            return if error.raw_os_error() == Some(WAIT_TIMEOUT.cast_signed()) {
                Ok(0)
            } else {
                Err(error)
            };
        }
        batch.len = usize::try_from(removed)
            .unwrap_or(0)
            .min(batch.entries.len());
        Ok(batch.len)
    }
}
