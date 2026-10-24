//! Directory handles (#43). [`DirHandle::open`] is the only place capture
//! opens a directory.

use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle as _, OwnedHandle, RawHandle};

use super::open::{LIST_DIRECTORY, OpenOptions, SHARE_ALL};
use super::slot::IoSlot;
use super::sys::{CancelIoEx, CreateFileW, ERROR_NOT_FOUND, INVALID_HANDLE_VALUE, OPEN_EXISTING};

/// What [`DirHandle::cancel`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelOutcome {
    /// The cancel was requested. The completion (`ERROR_OPERATION_ABORTED`,
    /// or the real result if it won the race) still arrives on the port, and
    /// the slot may only be reused after it was dequeued.
    Requested,
    /// Nothing was pending (`ERROR_NOT_FOUND`): the I/O already completed;
    /// its completion is on the port or was dequeued.
    AlreadyCompleted,
}

/// An open directory, closed on drop.
#[derive(Debug)]
pub struct DirHandle {
    handle: OwnedHandle,
}

impl DirHandle {
    /// Opens the directory at `long_path` (see
    /// [`to_long_path`](super::path::to_long_path)) for listing changes,
    /// overlapped, sharing everything.
    ///
    /// # Errors
    ///
    /// The OS error when the directory cannot be opened, or
    /// `InvalidInput` for a path with a NUL.
    pub fn open(long_path: &[u16], options: OpenOptions) -> io::Result<Self> {
        if long_path.contains(&0) {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let mut name = Vec::with_capacity(long_path.len() + 1);
        name.extend_from_slice(long_path);
        name.push(0);
        // SAFETY: `name` is NUL-terminated and outlives the call; no security
        // attributes or template; INVALID_HANDLE_VALUE is handled below.
        let raw = unsafe {
            CreateFileW(
                name.as_ptr(),
                LIST_DIRECTORY,
                SHARE_ALL,
                std::ptr::null(),
                OPEN_EXISTING,
                options.flags(),
                std::ptr::null_mut(),
            )
        };
        if raw == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: CreateFileW succeeded, so `raw` is an owned file handle.
        let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
        Ok(Self { handle })
    }

    /// Asks the kernel to cancel the I/O pending in `slot`.
    ///
    /// The slot's memory stays in use until the completion is dequeued.
    ///
    /// # Errors
    ///
    /// The OS error for a failure other than `ERROR_NOT_FOUND`.
    pub fn cancel(&self, slot: &IoSlot) -> io::Result<CancelOutcome> {
        // SAFETY: the handle is live; the address is the OVERLAPPED of
        // `slot`, which owns it on the heap and is borrowed for the call.
        let ok = unsafe {
            CancelIoEx(
                self.handle.as_raw_handle(),
                slot.overlapped_addr() as *const _,
            )
        };
        if ok != 0 {
            return Ok(CancelOutcome::Requested);
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_NOT_FOUND.cast_signed()) {
            Ok(CancelOutcome::AlreadyCompleted)
        } else {
            Err(error)
        }
    }
}

impl AsRawHandle for DirHandle {
    fn as_raw_handle(&self) -> RawHandle {
        self.handle.as_raw_handle()
    }
}
