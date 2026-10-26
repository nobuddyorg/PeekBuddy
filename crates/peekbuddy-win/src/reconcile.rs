//! Listing directories for the reconcile walk (#46).
//!
//! [`DirLister`] implements [`DirSource`] over a root: each directory is
//! opened with the #43 helper (`FILE_LIST_DIRECTORY`, backup semantics, reparse
//! points opened as themselves, cloud placeholders not recalled) and read
//! with `GetFileInformationByHandleEx(FileIdExtdDirectoryInfo)` into one
//! reused 64 KiB buffer. Nothing opens file data.
//!
//! [`BackgroundMode`] puts the calling thread in `THREAD_MODE_BACKGROUND_BEGIN`
//! (low I/O and memory priority) until it is dropped.

use std::os::windows::io::AsRawHandle;

use peekbuddy_core::reconcile::dirinfo;
use peekbuddy_core::reconcile::{DirSource, ListError, Listing, Meta};
use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_DIRECTORY, ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_FILES,
    ERROR_PATH_NOT_FOUND,
};
use windows_sys::Win32::Storage::FileSystem::{
    FileIdExtdDirectoryInfo, FileIdExtdDirectoryRestartInfo, GetFileInformationByHandleEx,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN, THREAD_MODE_BACKGROUND_END,
};

use crate::io::dir::DirHandle;
use crate::io::open::OpenOptions;

/// The listing buffer: 64 KiB, 8-byte aligned.
const BUFFER_UNITS: usize = 8 * 1024;

/// Lists the directories of one root.
#[derive(Debug)]
pub struct DirLister {
    root: Vec<u16>,
    buffer: Box<[u64]>,
}

impl DirLister {
    /// A lister for the root at `root_long`, a `\\?\` path as made by
    /// [`to_long_path`](crate::io::path::to_long_path), without a trailing
    /// separator.
    #[must_use]
    pub fn new(root_long: &[u16]) -> Self {
        let mut root = root_long.to_vec();
        while root.last() == Some(&u16::from(b'\\')) && root.len() > 1 {
            root.pop();
        }
        Self {
            root,
            buffer: vec![0_u64; BUFFER_UNITS].into_boxed_slice(),
        }
    }

    fn full_path(&self, path: &[u16]) -> Vec<u16> {
        let mut full = Vec::with_capacity(self.root.len() + 1 + path.len());
        full.extend_from_slice(&self.root);
        if !path.is_empty() {
            full.push(u16::from(b'\\'));
            full.extend_from_slice(path);
        }
        full
    }

    fn query(&mut self, dir: &DirHandle, restart: bool) -> Result<(), u32> {
        let class = if restart {
            FileIdExtdDirectoryRestartInfo
        } else {
            FileIdExtdDirectoryInfo
        };
        let bytes = u32::try_from(self.buffer.len() * 8).unwrap_or(u32::MAX);
        // SAFETY: the handle is open; the buffer holds `bytes` bytes and is
        // 8-byte aligned, as the class requires.
        let ok = unsafe {
            GetFileInformationByHandleEx(
                dir.as_raw_handle(),
                class,
                self.buffer.as_mut_ptr().cast(),
                bytes,
            )
        };
        if ok == 0 {
            Err(std::io::Error::last_os_error()
                .raw_os_error()
                .map_or(0, i32::cast_unsigned))
        } else {
            Ok(())
        }
    }

    /// The bytes of the buffer, for the parser.
    fn bytes(&self) -> &[u8] {
        // SAFETY: a `[u64]` is 8 * n initialised bytes with no padding, and
        // `u8` has alignment 1.
        unsafe {
            std::slice::from_raw_parts(self.buffer.as_ptr().cast::<u8>(), self.buffer.len() * 8)
        }
    }
}

fn map_error(code: u32) -> ListError {
    match code {
        ERROR_ACCESS_DENIED => ListError::AccessDenied,
        ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND | ERROR_DIRECTORY => ListError::Gone,
        _ => ListError::Other,
    }
}

impl DirSource for DirLister {
    fn list(&mut self, path: &[u16], out: &mut Listing) -> Result<(), ListError> {
        let full = self.full_path(path);
        let options = OpenOptions {
            reparse_point: true,
            no_recall: true,
        };
        let dir = DirHandle::open(&full, options)
            .map_err(|error| map_error(error.raw_os_error().map_or(0, i32::cast_unsigned)))?;
        let mut restart = true;
        loop {
            match self.query(&dir, restart) {
                Ok(()) => {}
                Err(ERROR_NO_MORE_FILES) => return Ok(()),
                Err(code) => return Err(map_error(code)),
            }
            restart = false;
            for item in dirinfo::parse(self.bytes(), self.buffer.len() * 8) {
                let entry = item.map_err(|_| ListError::Other)?;
                if entry.is_dot() {
                    continue;
                }
                out.push_units(
                    entry.name.len(),
                    entry.name.units(),
                    Meta {
                        file_id: entry.file_id,
                        attributes: entry.attributes,
                        reparse_tag: entry.reparse_tag,
                        size: u64::try_from(entry.size).unwrap_or(0),
                        write_time: entry.write_time,
                        change_time: entry.change_time,
                    },
                    0,
                );
            }
        }
    }
}

/// The calling thread in background mode (low I/O and memory priority) until
/// dropped.
#[derive(Debug)]
pub struct BackgroundMode {
    active: bool,
}

impl BackgroundMode {
    /// Enters background mode; if the OS refuses, the guard does nothing.
    #[must_use]
    pub fn enter() -> Self {
        // SAFETY: GetCurrentThread only returns a pseudo-handle, valid on
        // this thread, that needs no closing.
        let thread = unsafe { GetCurrentThread() };
        // SAFETY: the pseudo-handle is valid for the call.
        let ok = unsafe { SetThreadPriority(thread, THREAD_MODE_BACKGROUND_BEGIN) };
        Self { active: ok != 0 }
    }

    /// Whether the thread is in background mode.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.active
    }
}

impl Drop for BackgroundMode {
    fn drop(&mut self) {
        if self.active {
            // SAFETY: as in `enter`.
            let thread = unsafe { GetCurrentThread() };
            // SAFETY: this runs on the thread that entered; the pseudo-handle
            // is valid for the call.
            unsafe { SetThreadPriority(thread, THREAD_MODE_BACKGROUND_END) };
        }
    }
}
