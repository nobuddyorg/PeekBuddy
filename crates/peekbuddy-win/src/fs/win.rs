//! The Win32 calls behind [`super`].

use std::os::windows::ffi::OsStrExt as _;
use std::os::windows::io::{AsRawHandle, FromRawHandle as _, OwnedHandle};
use std::path::Path;

use peekbuddy_core::name::CaseMode;
use windows_sys::Win32::Foundation::{
    ERROR_INSUFFICIENT_BUFFER, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_CASE_SENSITIVE_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_NO_RECALL,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_128, FILE_ID_DESCRIPTOR, FILE_ID_DESCRIPTOR_0,
    FILE_ID_INFO, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    FileCaseSensitiveInfo, FileIdInfo, GetFileAttributesW, GetFileInformationByHandleEx,
    GetFinalPathNameByHandleW, GetShortPathNameW, GetVolumeInformationW, GetVolumePathNameW,
    INVALID_FILE_ATTRIBUTES, OPEN_EXISTING, OpenFileById, SetFileAttributesW, VOLUME_NAME_NONE,
};
use windows_sys::Win32::System::IO::DeviceIoControl;

use super::{
    FileIdKind, case_mode_from_flags, file_id_kind, has_short_name, junction_names,
    means_no_case_sensitivity, mount_point_buffer, toggled_archive, verbatim,
};

/// `FILE_ID_TYPE` values of `FILE_ID_DESCRIPTOR` (kept local: the enum lives
/// in the same module but is a bare `i32`).
const FILE_ID_TYPE: i32 = 0;
const EXTENDED_FILE_ID_TYPE: i32 = 2;

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
            // Disjoint bits: `+` equals `|` and has no equivalent `^` mutant.
            FILE_FLAG_OPEN_REPARSE_POINT + FILE_FLAG_BACKUP_SEMANTICS,
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
    Ok(has_short_name(&result?, long))
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

/// An attribute-only handle for `path` (a file or a directory), without
/// following a reparse point and without recalling cloud files (F18).
fn open_attributes(path: &Path) -> std::io::Result<OwnedHandle> {
    let name = wide(path)?;
    // SAFETY: `name` is NUL-terminated and outlives the call; no security
    // attributes or template; INVALID_HANDLE_VALUE is handled below.
    let raw = unsafe {
        CreateFileW(
            name.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_OPEN_NO_RECALL,
            std::ptr::null_mut(),
        )
    };
    if raw == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: CreateFileW succeeded, so `raw` is an owned file handle.
    Ok(unsafe { OwnedHandle::from_raw_handle(raw) })
}

/// An attribute-only handle for the directory `dir`, the volume hint for
/// [`resolve_long_name`].
///
/// # Errors
///
/// Returns the OS error when the directory cannot be opened.
pub fn open_directory(dir: &Path) -> std::io::Result<OwnedHandle> {
    open_attributes(dir)
}

/// The file id of `path`, widened to 128 bits (NTFS ids sit in the low 64
/// bits, `ReFS` ids use all 128).
///
/// # Errors
///
/// Returns the OS error when the file cannot be opened or the volume does not
/// report 128-bit ids.
pub fn file_id(path: &Path) -> std::io::Result<u128> {
    let handle = open_attributes(path)?;
    let mut info = FILE_ID_INFO {
        VolumeSerialNumber: 0,
        FileId: FILE_ID_128 {
            Identifier: [0; 16],
        },
    };
    // SAFETY: `handle` is live; `info` is a `FILE_ID_INFO` of the size passed.
    check(unsafe {
        GetFileInformationByHandleEx(
            handle.as_raw_handle(),
            FileIdInfo,
            (&raw mut info).cast(),
            u32::try_from(size_of::<FILE_ID_INFO>()).unwrap_or(u32::MAX),
        )
    })?;
    Ok(u128::from_le_bytes(info.FileId.Identifier))
}

/// Best-effort long-name resolution (#39): opens `file_id` on the volume of
/// `hint` with attribute-only access, without following reparse points and
/// without recalling cloud files, and writes its volume-relative path (for
/// example `\dir\file name.txt`) into `out`. Returns the units written.
///
/// The delivered name is never replaced by this; the caller keeps it and adds
/// the resolved name as an extra field. A failure means "unresolved", never a
/// dropped event.
///
/// # Errors
///
/// Returns the OS error when the file cannot be opened by id (it may already
/// be gone) or `ERROR_INSUFFICIENT_BUFFER` when `out` is too small.
pub fn resolve_long_name(
    hint: &impl AsRawHandle,
    file_id: u128,
    out: &mut [u16],
) -> std::io::Result<usize> {
    let size = u32::try_from(size_of::<FILE_ID_DESCRIPTOR>()).unwrap_or(u32::MAX);
    let descriptor = match file_id_kind(file_id) {
        FileIdKind::Narrow(id) => FILE_ID_DESCRIPTOR {
            dwSize: size,
            Type: FILE_ID_TYPE,
            Anonymous: FILE_ID_DESCRIPTOR_0 { FileId: id },
        },
        FileIdKind::Extended(bytes) => FILE_ID_DESCRIPTOR {
            dwSize: size,
            Type: EXTENDED_FILE_ID_TYPE,
            Anonymous: FILE_ID_DESCRIPTOR_0 {
                ExtendedFileId: FILE_ID_128 { Identifier: bytes },
            },
        },
    };
    // SAFETY: `hint` is a live handle on the volume; `descriptor` is fully
    // initialised with its size; no security attributes; INVALID_HANDLE_VALUE
    // is handled below.
    let raw: HANDLE = unsafe {
        OpenFileById(
            hint.as_raw_handle(),
            &raw const descriptor,
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_OPEN_NO_RECALL,
        )
    };
    if raw == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: OpenFileById succeeded, so `raw` is an owned file handle.
    let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
    let capacity = u32::try_from(out.len()).unwrap_or(u32::MAX);
    // SAFETY: `handle` is live; `out` holds `capacity` writable units.
    let len = unsafe {
        GetFinalPathNameByHandleW(
            handle.as_raw_handle(),
            out.as_mut_ptr(),
            capacity,
            VOLUME_NAME_NONE,
        )
    };
    let len = usize::try_from(len).unwrap_or(usize::MAX);
    if len == 0 {
        Err(std::io::Error::last_os_error())
    } else if len >= out.len() {
        // The call reports the size it needs, including the NUL.
        Err(std::io::Error::from_raw_os_error(
            i32::try_from(ERROR_INSUFFICIENT_BUFFER).unwrap_or(i32::MAX),
        ))
    } else {
        Ok(len)
    }
}

/// The case rule of the directory `dir` (#39): case-sensitive when it carries
/// `FILE_CS_FLAG_CASE_SENSITIVE_DIR`. A volume without per-directory case
/// sensitivity (FAT, older `ReFS`) is case-insensitive.
///
/// # Errors
///
/// Returns the OS error when the directory cannot be opened or queried for
/// another reason than missing support.
pub fn case_mode(dir: &Path) -> std::io::Result<CaseMode> {
    let handle = open_attributes(dir)?;
    let mut info = FILE_CASE_SENSITIVE_INFO { Flags: 0 };
    // SAFETY: `handle` is live; `info` is a `FILE_CASE_SENSITIVE_INFO` of the
    // size passed.
    let ok = unsafe {
        GetFileInformationByHandleEx(
            handle.as_raw_handle(),
            FileCaseSensitiveInfo,
            (&raw mut info).cast(),
            u32::try_from(size_of::<FILE_CASE_SENSITIVE_INFO>()).unwrap_or(u32::MAX),
        )
    };
    if ok == 0 {
        let error = std::io::Error::last_os_error();
        return match error.raw_os_error() {
            Some(code) if means_no_case_sensitivity(code) => Ok(CaseMode::Insensitive),
            _ => Err(error),
        };
    }
    Ok(case_mode_from_flags(info.Flags))
}
