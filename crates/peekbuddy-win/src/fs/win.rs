//! The Win32 calls behind [`super`].

use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt as _, OsStringExt as _};
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
use std::path::Path;

use windows_sys::Win32::Foundation::{GENERIC_WRITE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, GetFileAttributesW,
    GetShortPathNameW, GetVolumeInformationW, GetVolumePathNameW, INVALID_FILE_ATTRIBUTES,
    OPEN_EXISTING, SetFileAttributesW,
};
use windows_sys::Win32::System::IO::DeviceIoControl;

use super::{junction_names, mount_point_buffer, toggled_archive, verbatim};

/// `FSCTL_SET_REPARSE_POINT` (kept local to avoid the `Win32_System_Ioctl`
/// feature).
const FSCTL_SET_REPARSE_POINT: u32 = 0x0009_00A4;

/// Longest path in UTF-16 units, plus its NUL.
const PATH_UNITS: usize = 32_768;

fn check(ok: i32) -> std::io::Result<()> {
    if ok == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// `path` made absolute, in `\\?\` form, NUL-terminated.
fn wide(path: &Path) -> std::io::Result<Vec<u16>> {
    let absolute = std::path::absolute(path)?;
    let units: Vec<u16> = absolute.as_os_str().encode_wide().collect();
    let mut out = verbatim(&units);
    out.push(0);
    Ok(out)
}

fn invalid(message: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, message)
}

/// Creates the directory `junction` as a junction (mount point) to the
/// directory `target`. The junction is the link object itself: nothing
/// follows it (F18). On failure the directory is removed again.
///
/// # Errors
///
/// Returns the OS error when the directory cannot be created or the reparse
/// point cannot be set.
pub fn create_junction(junction: &Path, target: &Path) -> std::io::Result<()> {
    let absolute = std::path::absolute(target)?;
    let units: Vec<u16> = absolute.as_os_str().encode_wide().collect();
    let (substitute, print) = junction_names(&units);
    let buffer = mount_point_buffer(&substitute, &print)
        .ok_or_else(|| invalid("junction target too long"))?;
    let size = u32::try_from(buffer.len()).map_err(|_| invalid("reparse buffer too large"))?;
    let name = wide(junction)?;
    std::fs::create_dir(junction)?;
    let result = set_mount_point(&name, &buffer, size);
    if result.is_err() {
        let _ = std::fs::remove_dir(junction);
    }
    result
}

fn set_mount_point(name: &[u16], buffer: &[u8], size: u32) -> std::io::Result<()> {
    // SAFETY: `name` is NUL-terminated and outlives the call; no security
    // attributes or template; INVALID_HANDLE_VALUE is handled below.
    let raw = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_WRITE,
            0,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        )
    };
    if raw == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: CreateFileW succeeded, so `raw` is an owned file handle.
    let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut returned = 0u32;
    // SAFETY: `handle` is live; `buffer` holds `size` readable bytes; no
    // output buffer; `returned` is a valid out pointer; synchronous call.
    check(unsafe {
        DeviceIoControl(
            handle.as_raw_handle(),
            FSCTL_SET_REPARSE_POINT,
            buffer.as_ptr().cast(),
            size,
            std::ptr::null_mut(),
            0,
            &raw mut returned,
            std::ptr::null_mut(),
        )
    })
}

/// Flips `FILE_ATTRIBUTE_ARCHIVE` on `path`: an attribute change that
/// produces a modified notification and leaves the file writable.
///
/// # Errors
///
/// Returns the OS error when the attributes cannot be read or written.
pub fn toggle_archive(path: &Path) -> std::io::Result<()> {
    let name = wide(path)?;
    // SAFETY: `name` is NUL-terminated and outlives the call.
    let attributes = unsafe { GetFileAttributesW(name.as_ptr()) };
    if attributes == INVALID_FILE_ATTRIBUTES {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: as above.
    check(unsafe { SetFileAttributesW(name.as_ptr(), toggled_archive(attributes)) })
}

/// The file-system name (`NTFS`, `ReFS`, ...) of the volume holding `path`.
///
/// # Errors
///
/// Returns the OS error when the volume cannot be queried.
pub fn file_system_name(path: &Path) -> std::io::Result<String> {
    let name = wide(path)?;
    let mut volume = vec![0u16; PATH_UNITS];
    let capacity = u32::try_from(volume.len()).unwrap_or(u32::MAX);
    // SAFETY: `name` is NUL-terminated; `volume` holds `capacity` units.
    check(unsafe { GetVolumePathNameW(name.as_ptr(), volume.as_mut_ptr(), capacity) })?;
    let mut file_system = [0u16; 261];
    let fs_capacity = u32::try_from(file_system.len()).unwrap_or(u32::MAX);
    // SAFETY: `volume` was NUL-terminated by the call above; the optional
    // outputs are null; `file_system` holds `fs_capacity` units.
    check(unsafe {
        GetVolumeInformationW(
            volume.as_ptr(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            file_system.as_mut_ptr(),
            fs_capacity,
        )
    })?;
    let len = file_system
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(file_system.len());
    Ok(String::from_utf16_lossy(
        file_system.get(..len).unwrap_or_default(),
    ))
}

/// Whether the volume generates 8.3 short names in `dir`: creates a probe
/// file with a long name, asks for its short path and removes it again.
///
/// # Errors
///
/// Returns the OS error when the probe cannot be created or queried.
pub fn short_names_generated(dir: &Path) -> std::io::Result<bool> {
    let long = "PeekBuddy short-name probe.tmp";
    let probe = dir.join(long);
    std::fs::File::create(&probe)?;
    let result = short_path(&probe);
    std::fs::remove_file(&probe)?;
    let short = result?;
    let file_name = short
        .rsplit(|&unit| unit == u16::from(b'\\'))
        .next()
        .unwrap_or_default();
    Ok(OsString::from_wide(file_name) != long)
}

fn short_path(path: &Path) -> std::io::Result<Vec<u16>> {
    let name = wide(path)?;
    let mut short = vec![0u16; PATH_UNITS];
    let capacity = u32::try_from(short.len()).unwrap_or(u32::MAX);
    // SAFETY: `name` is NUL-terminated; `short` holds `capacity` units.
    let len = unsafe { GetShortPathNameW(name.as_ptr(), short.as_mut_ptr(), capacity) };
    if len == 0 {
        return Err(std::io::Error::last_os_error());
    }
    short.truncate(usize::try_from(len).unwrap_or(0));
    Ok(short)
}
