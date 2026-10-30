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

use crate::io::dir::DirHandle;
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
        })
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
