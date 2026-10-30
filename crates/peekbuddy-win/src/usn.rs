//! Reading the USN change journal without elevation (#47, feature `usn`).
//!
//! Spike 0001 found that `FSCTL_READ_UNPRIVILEGED_USN_JOURNAL` works for a
//! standard user on NTFS and `ReFS` through a directory handle opened on the
//! volume GUID path (`\\?\Volume{...}\`, the `guid-dir` form), and that its
//! records carry file reference numbers but never names. ADR 0018 keeps this
//! backend off in v1; this module is the part that talks to the journal. The
//! record decoder and the scoping live in `peekbuddy_core::usn`.
//!
//! [`Journal`] holds the volume handle and one reused buffer. Reads here wait
//! for the call to finish on a private event; the IOCP reader of the service
//! mode builds on the same two calls.

use std::io;
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};

use peekbuddy_core::usn::checkpoint::JournalData;
use windows_sys::Win32::Foundation::ERROR_IO_PENDING;
use windows_sys::Win32::System::IO::{DeviceIoControl, GetOverlappedResult};
use windows_sys::Win32::System::Ioctl::{
    FSCTL_QUERY_USN_JOURNAL, FSCTL_READ_UNPRIVILEGED_USN_JOURNAL, READ_USN_JOURNAL_DATA_V1,
};
use windows_sys::Win32::System::Threading::CreateEventW;

use crate::io::dir::{CancelOutcome, DirHandle};
use crate::io::iocp::{Completion, Iocp};
use crate::io::open::OpenOptions;
use crate::io::slot::IoSlot;

/// Every `USN_REASON_*` bit.
const ALL_REASONS: u32 = 0xFFFF_FFFF;
/// Version 2 and 3 records (the decoder skips version 4).
const MIN_MAJOR: u16 = 2;
const MAX_MAJOR: u16 = 3;
/// The default read buffer: 64 KiB, the low end of the issue's range.
pub const DEFAULT_BUFFER: usize = 64 * 1024;

/// An open journal of one volume.
#[derive(Debug)]
pub struct Journal {
    volume: DirHandle,
    event: OwnedHandle,
    slot: IoSlot,
    /// The volume handle is on a completion port: reads complete there, so
    /// the waiting calls would race the port and are refused.
    on_port: bool,
}

impl Journal {
    /// Opens the journal of the volume at `guid_path` (`\\?\Volume{...}\`,
    /// from [`volume_identity`](crate::io::volume::volume_identity)).
    ///
    /// # Errors
    ///
    /// The OS error when the volume cannot be opened or the event created.
    pub fn open(guid_path: &[u16], buffer_bytes: usize) -> io::Result<Self> {
        let volume = DirHandle::open(guid_path, OpenOptions::default())?;
        // SAFETY: plain call: no security attributes, manual reset, not
        // signalled, no name; a null result is handled below.
        let raw = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
        if raw.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: CreateEventW succeeded, so `raw` is an owned handle.
        let event = unsafe { OwnedHandle::from_raw_handle(raw) };
        let slot = IoSlot::new(buffer_bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        Ok(Self {
            volume,
            event,
            slot,
            on_port: false,
        })
    }

    /// Makes the volume handle's completions arrive on `iocp` with `key`, for
    /// [`Journal::begin_wait`]. From then on [`Journal::query`] and
    /// [`Journal::read`] are refused: every call completes on the port.
    ///
    /// # Errors
    ///
    /// The OS error when the association fails.
    pub fn associate(&mut self, iocp: &Iocp, key: usize) -> io::Result<()> {
        iocp.associate(&self.volume, key)?;
        self.on_port = true;
        Ok(())
    }

    /// Starts a read from `start_usn` that stays pending until at least one
    /// record is there (`BytesToWaitFor = 1`), so an idle volume costs no
    /// wakeups. Its completion arrives on the port; pass it to
    /// [`Journal::finish`]. `Timeout` is ignored on overlapped handles, so
    /// [`Journal::cancel`] is how a pending read ends.
    ///
    /// # Errors
    ///
    /// `InvalidInput` when the journal is not on a port or a read is already
    /// pending; the OS error when the call cannot start (no completion is
    /// queued then).
    pub fn begin_wait(&mut self, journal_id: u64, start_usn: i64) -> io::Result<()> {
        if !self.on_port {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let request = READ_USN_JOURNAL_DATA_V1 {
            StartUsn: start_usn,
            ReasonMask: ALL_REASONS,
            ReturnOnlyOnClose: 0,
            Timeout: 0,
            BytesToWaitFor: 1,
            UsnJournalID: journal_id,
            MinMajorVersion: MIN_MAJOR,
            MaxMajorVersion: MAX_MAJOR,
        };
        let input = request_bytes(&request);
        let ptrs = self
            .slot
            .begin()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        let input_len = u32::try_from(input.len()).unwrap_or(u32::MAX);
        let mut returned: u32 = 0;
        // SAFETY: the handle is live; `input` is readable for `input_len`
        // bytes and is copied by the I/O manager before the call returns
        // (buffered FSCTL); the output buffer and the zeroed OVERLAPPED
        // belong to the slot, which stays pending until `finish`, so they
        // are neither moved nor read meanwhile. No event is set, so the
        // completion goes to the port.
        let started = unsafe {
            DeviceIoControl(
                self.volume.as_raw_handle(),
                FSCTL_READ_UNPRIVILEGED_USN_JOURNAL,
                input.as_ptr().cast(),
                input_len,
                ptrs.buffer.cast(),
                ptrs.len,
                &raw mut returned,
                ptrs.overlapped,
            )
        };
        if started == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_IO_PENDING.cast_signed()) {
                // It failed to start: nothing will be queued.
                let _ = self.slot.finish();
                return Err(error);
            }
        }
        Ok(())
    }

    /// Whether a read started by [`Journal::begin_wait`] has not been
    /// finished.
    #[must_use]
    pub fn is_pending(&self) -> bool {
        self.slot.is_pending()
    }

    /// Asks the kernel to cancel the pending read (`CancelIoEx`). Its
    /// completion (error 995, or the data if it won the race) still arrives
    /// on the port and must be dequeued and passed to [`Journal::finish`]
    /// before the next read.
    ///
    /// # Errors
    ///
    /// The OS error for a failure other than "nothing pending".
    pub fn cancel(&self) -> io::Result<CancelOutcome> {
        self.volume.cancel(&self.slot)
    }

    /// Takes the result of a read from its dequeued `completion`: the bytes
    /// (the next USN, then records) on success, the OS error otherwise
    /// (`ERROR_OPERATION_ABORTED`, 995, after a cancel).
    ///
    /// # Errors
    ///
    /// The OS error of the completion; `InvalidInput` for a completion that is
    /// not this journal's.
    pub fn finish(&mut self, completion: &Completion) -> io::Result<&[u8]> {
        if completion.overlapped != self.slot.overlapped_addr() || !self.slot.is_pending() {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let _ = self.slot.finish();
        if !completion.is_success() {
            return Err(io::Error::from_raw_os_error(
                i32::try_from(completion.status.code()).unwrap_or(i32::MAX),
            ));
        }
        self.slot
            .data(usize::try_from(completion.bytes).unwrap_or(0))
            .ok_or_else(|| io::ErrorKind::InvalidData.into())
    }

    /// The volume-relative path of the file with reference number `frn`
    /// (like `\dir\file.txt`), for the records the unprivileged read gives no
    /// name: `OpenFileById` with attribute-only access, no reparse follow, no
    /// cloud recall, then `GetFinalPathNameByHandleW`. Returns the units
    /// written into `out`.
    ///
    /// The whole number is compared, sequence number included, so a deleted
    /// file or a reused slot is an error, never another file's name.
    ///
    /// # Errors
    ///
    /// The OS error when the file is gone or cannot be opened, or
    /// `ERROR_INSUFFICIENT_BUFFER` for a small `out`.
    pub fn resolve_path(&self, frn: u128, out: &mut [u16]) -> io::Result<usize> {
        crate::fs::resolve_long_name(&self.volume, frn, out)
    }

    /// `FSCTL_QUERY_USN_JOURNAL`: the journal's id and range.
    ///
    /// # Errors
    ///
    /// The OS error, for example `ERROR_JOURNAL_NOT_ACTIVE` (1179) or
    /// `ERROR_INVALID_FUNCTION` (1) on a file system without a journal.
    pub fn query(&mut self) -> io::Result<JournalData> {
        let bytes = self.control(FSCTL_QUERY_USN_JOURNAL, &[])?;
        let data = self.slot.data(bytes).ok_or(io::ErrorKind::InvalidData)?;
        // Every version of the reply starts with the four fields needed.
        parse_head(data)
    }

    /// `FSCTL_READ_UNPRIVILEGED_USN_JOURNAL` from `start_usn`, without
    /// waiting for new records. The buffer starts with the next USN and holds
    /// whole records ([`peekbuddy_core::usn::record::records`] walks it).
    ///
    /// # Errors
    ///
    /// The OS error, for example `ERROR_JOURNAL_ENTRY_DELETED` (1181) when
    /// `start_usn` is older than the journal keeps.
    pub fn read(&mut self, journal_id: u64, start_usn: i64) -> io::Result<&[u8]> {
        let request = READ_USN_JOURNAL_DATA_V1 {
            StartUsn: start_usn,
            ReasonMask: ALL_REASONS,
            ReturnOnlyOnClose: 0,
            Timeout: 0,
            BytesToWaitFor: 0,
            UsnJournalID: journal_id,
            MinMajorVersion: MIN_MAJOR,
            MaxMajorVersion: MAX_MAJOR,
        };
        let input = request_bytes(&request);
        let bytes = self.control(FSCTL_READ_UNPRIVILEGED_USN_JOURNAL, &input)?;
        self.slot
            .data(bytes)
            .ok_or_else(|| io::ErrorKind::InvalidData.into())
    }

    /// One overlapped `DeviceIoControl` on the volume, waited for.
    fn control(&mut self, code: u32, input: &[u8]) -> io::Result<usize> {
        if self.on_port {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let ptrs = self
            .slot
            .begin()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        let input_len = u32::try_from(input.len()).unwrap_or(u32::MAX);
        let mut returned: u32 = 0;
        // SAFETY: `ptrs.overlapped` points at the slot's zeroed OVERLAPPED,
        // which lives on the heap as long as the slot; the event handle is
        // live for the whole call.
        unsafe { (*ptrs.overlapped).hEvent = self.event.as_raw_handle() };
        // SAFETY: the handle is live; `input` is readable for `input_len`
        // bytes; the output buffer and OVERLAPPED belong to the slot, which
        // stays pending (and so unmoved and unwritten by us) until the wait
        // below returns.
        let started = unsafe {
            DeviceIoControl(
                self.volume.as_raw_handle(),
                code,
                input.as_ptr().cast(),
                input_len,
                ptrs.buffer.cast(),
                ptrs.len,
                &raw mut returned,
                ptrs.overlapped,
            )
        };
        let outcome = if started != 0 {
            Ok(())
        } else {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_IO_PENDING.cast_signed()) {
                // SAFETY: same handle and OVERLAPPED as the call above; wait
                // for it to finish.
                let done = unsafe {
                    GetOverlappedResult(
                        self.volume.as_raw_handle(),
                        ptrs.overlapped,
                        &raw mut returned,
                        1,
                    )
                };
                if done != 0 {
                    Ok(())
                } else {
                    Err(io::Error::last_os_error())
                }
            } else {
                Err(error)
            }
        };
        // The call finished, one way or the other: the kernel no longer
        // touches the slot.
        let _ = self.slot.finish();
        outcome.map(|()| usize::try_from(returned).unwrap_or(0))
    }
}

/// The four fields every journal data version starts with.
fn parse_head(data: &[u8]) -> io::Result<JournalData> {
    let word = |at: usize| -> io::Result<[u8; 8]> {
        data.get(at..at + 8)
            .and_then(|bytes| <[u8; 8]>::try_from(bytes).ok())
            .ok_or_else(|| io::ErrorKind::InvalidData.into())
    };
    Ok(JournalData {
        journal_id: u64::from_le_bytes(word(0)?),
        first_usn: i64::from_le_bytes(word(8)?),
        next_usn: i64::from_le_bytes(word(16)?),
        lowest_valid_usn: i64::from_le_bytes(word(24)?),
    })
}

/// The bytes of a read request, field by field in the struct's order.
fn request_bytes(request: &READ_USN_JOURNAL_DATA_V1) -> Vec<u8> {
    let mut out = Vec::with_capacity(size_of::<READ_USN_JOURNAL_DATA_V1>());
    out.extend_from_slice(&request.StartUsn.to_le_bytes());
    out.extend_from_slice(&request.ReasonMask.to_le_bytes());
    out.extend_from_slice(&request.ReturnOnlyOnClose.to_le_bytes());
    out.extend_from_slice(&request.Timeout.to_le_bytes());
    out.extend_from_slice(&request.BytesToWaitFor.to_le_bytes());
    out.extend_from_slice(&request.UsnJournalID.to_le_bytes());
    out.extend_from_slice(&request.MinMajorVersion.to_le_bytes());
    out.extend_from_slice(&request.MaxMajorVersion.to_le_bytes());
    // The struct is 8-byte aligned: 4 bytes of tail padding.
    out.resize(size_of::<READ_USN_JOURNAL_DATA_V1>(), 0);
    out
}

#[cfg(test)]
mod tests;
