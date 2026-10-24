//! [`IoSlot`]: an OVERLAPPED and its buffer, pinned while I/O is pending
//! (#43).
//!
//! The kernel keeps the OVERLAPPED and the buffer until the completion has
//! been dequeued, even after a cancel. A slot therefore owns both on the heap
//! (their addresses never change when the slot moves), refuses to start twice,
//! and leaks its memory if it is dropped while I/O is pending: freeing it
//! could let the kernel write into freed memory.
//!
//! The buffer is DWORD-aligned (`ReadDirectoryChangesW` fails with
//! `ERROR_NOACCESS` otherwise) and allocated once, in [`IoSlot::new`].

use std::fmt;

use super::sys::OVERLAPPED;

/// Why a slot cannot be created or used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotError {
    /// The buffer size is zero, not a multiple of 4, or over `u32::MAX`.
    BadSize,
    /// The slot has I/O pending.
    Pending,
    /// The slot has no I/O pending.
    NotPending,
}

impl fmt::Display for SlotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::BadSize => "the buffer size must be a non-zero multiple of 4 that fits a DWORD",
            Self::Pending => "the slot has I/O pending",
            Self::NotPending => "the slot has no I/O pending",
        })
    }
}

impl std::error::Error for SlotError {}

/// The pointers for one I/O call, valid until [`IoSlot::finish`].
#[derive(Debug, Clone, Copy)]
pub struct SlotPtrs {
    /// The OVERLAPPED, zeroed by [`IoSlot::begin`].
    pub overlapped: *mut OVERLAPPED,
    /// The first byte of the buffer, DWORD-aligned.
    pub buffer: *mut u8,
    /// The buffer length in bytes.
    pub len: u32,
}

/// An OVERLAPPED and its buffer.
pub struct IoSlot {
    overlapped: Box<OVERLAPPED>,
    buffer: Box<[u32]>,
    pending: bool,
}

impl fmt::Debug for IoSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IoSlot")
            .field("len", &self.len())
            .field("pending", &self.pending)
            .finish_non_exhaustive()
    }
}

impl IoSlot {
    /// A slot with a buffer of `bytes` bytes.
    ///
    /// # Errors
    ///
    /// [`SlotError::BadSize`] unless `bytes` is a non-zero multiple of 4 that
    /// fits a `u32`.
    pub fn new(bytes: usize) -> Result<Self, SlotError> {
        if bytes == 0 || !bytes.is_multiple_of(4) || u32::try_from(bytes).is_err() {
            return Err(SlotError::BadSize);
        }
        Ok(Self {
            overlapped: Box::new(OVERLAPPED::default()),
            buffer: vec![0_u32; bytes / 4].into_boxed_slice(),
            pending: false,
        })
    }

    /// The buffer length in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.buffer.len() * 4
    }

    /// Always false: a slot has a buffer.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Whether I/O is pending.
    #[must_use]
    pub const fn is_pending(&self) -> bool {
        self.pending
    }

    /// The address of the OVERLAPPED: what a completion reports, and the key
    /// to map it back to its slot. Stable for the life of the slot.
    #[must_use]
    pub fn overlapped_addr(&self) -> usize {
        std::ptr::from_ref::<OVERLAPPED>(&self.overlapped) as usize
    }

    /// Marks the slot pending and returns the pointers for the I/O call.
    ///
    /// The OVERLAPPED is zeroed. The caller must issue the I/O with these
    /// pointers; if it fails to start, call [`IoSlot::finish`] at once.
    ///
    /// # Errors
    ///
    /// [`SlotError::Pending`] if the previous I/O has not been finished.
    pub fn begin(&mut self) -> Result<SlotPtrs, SlotError> {
        if self.pending {
            return Err(SlotError::Pending);
        }
        self.pending = true;
        *self.overlapped = OVERLAPPED::default();
        let len = u32::try_from(self.len()).map_err(|_| SlotError::BadSize)?;
        Ok(SlotPtrs {
            overlapped: &raw mut *self.overlapped,
            buffer: self.buffer.as_mut_ptr().cast::<u8>(),
            len,
        })
    }

    /// Marks the I/O finished: call it after its completion was dequeued (or
    /// after the call failed to start). The slot can then be used again.
    ///
    /// # Errors
    ///
    /// [`SlotError::NotPending`] if nothing was pending.
    pub fn finish(&mut self) -> Result<(), SlotError> {
        if !self.pending {
            return Err(SlotError::NotPending);
        }
        self.pending = false;
        Ok(())
    }

    /// The first `bytes` bytes of the buffer, after a completion reported
    /// that many; `None` while I/O is pending or past the buffer.
    #[must_use]
    pub fn data(&self, bytes: usize) -> Option<&[u8]> {
        if self.pending || bytes > self.len() {
            return None;
        }
        // SAFETY: the buffer holds `len()` initialised bytes (a `[u32]` is
        // 4 * n bytes with no padding), `u8` has alignment 1, `bytes` is at
        // most `len()`, and no I/O is pending, so the kernel is not writing.
        Some(unsafe { std::slice::from_raw_parts(self.buffer.as_ptr().cast::<u8>(), bytes) })
    }
}

impl Drop for IoSlot {
    fn drop(&mut self) {
        if self.pending {
            // The kernel may still write: never free the memory.
            let overlapped =
                std::mem::replace(&mut self.overlapped, Box::new(OVERLAPPED::default()));
            let buffer = std::mem::take(&mut self.buffer);
            std::mem::forget(overlapped);
            std::mem::forget(buffer);
        }
    }
}
