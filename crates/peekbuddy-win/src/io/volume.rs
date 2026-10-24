//! Volume identity and final path names (#43).
//!
//! A drive letter and a mount folder can name the same volume, and a drive
//! letter can change. A volume is therefore identified by its GUID path and
//! its serial number, never by a letter or a device number.

use std::io;
use std::os::windows::io::AsRawHandle;

use super::dir::DirHandle;
use super::sys::{
    FILE_NAME_NORMALIZED, GetFinalPathNameByHandleW, GetVolumeInformationByHandleW,
    GetVolumeNameForVolumeMountPointW, GetVolumePathNameW, VOLUME_NAME_DOS,
};

/// Longest path in UTF-16 units, plus its NUL.
const PATH_UNITS: usize = 32_768;
/// `MAX_PATH`, enough for a volume name and a file-system name.
const SMALL_UNITS: usize = 261;

/// Who a volume is.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VolumeIdentity {
    /// The volume GUID path, `\\?\Volume{...}\`.
    pub guid_path: Vec<u16>,
    /// The volume serial number.
    pub serial: u32,
    /// The file-system name (`NTFS`, `ReFS`, ...).
    pub file_system: String,
}

fn nul_terminated(units: &[u16]) -> io::Result<Vec<u16>> {
    if units.contains(&0) {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let mut out = Vec::with_capacity(units.len() + 1);
    out.extend_from_slice(units);
    out.push(0);
    Ok(out)
}

fn until_nul(units: &[u16]) -> &[u16] {
    let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
    units.get(..end).unwrap_or(units)
}

/// The identity of the volume that holds `path` (reached through the open
/// directory `dir`, which supplies the serial number and the file-system
/// name).
///
/// # Errors
///
/// The OS error when the volume cannot be queried.
pub fn volume_identity(dir: &DirHandle, path: &[u16]) -> io::Result<VolumeIdentity> {
    let name = nul_terminated(path)?;
    let mut root = vec![0_u16; PATH_UNITS];
    let root_len = u32::try_from(root.len()).unwrap_or(u32::MAX);
    // SAFETY: `name` is NUL-terminated; `root` holds `root_len` units.
    if unsafe { GetVolumePathNameW(name.as_ptr(), root.as_mut_ptr(), root_len) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut guid = vec![0_u16; SMALL_UNITS];
    let guid_len = u32::try_from(guid.len()).unwrap_or(u32::MAX);
    // SAFETY: `root` was NUL-terminated by the call above; `guid` holds
    // `guid_len` units.
    if unsafe { GetVolumeNameForVolumeMountPointW(root.as_ptr(), guid.as_mut_ptr(), guid_len) } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let mut serial = 0_u32;
    let mut file_system = vec![0_u16; SMALL_UNITS];
    let fs_len = u32::try_from(file_system.len()).unwrap_or(u32::MAX);
    // SAFETY: the handle is live; the optional outputs are null; `serial` is
    // a valid out pointer; `file_system` holds `fs_len` units.
    let ok = unsafe {
        GetVolumeInformationByHandleW(
            dir.as_raw_handle(),
            std::ptr::null_mut(),
            0,
            &raw mut serial,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            file_system.as_mut_ptr(),
            fs_len,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(VolumeIdentity {
        guid_path: until_nul(&guid).to_vec(),
        serial,
        file_system: String::from_utf16_lossy(until_nul(&file_system)),
    })
}

/// The canonical path of an open directory (`\\?\C:\dir`), resolved through
/// links and mount points.
///
/// # Errors
///
/// The OS error when the name cannot be resolved.
pub fn final_path(dir: &DirHandle) -> io::Result<Vec<u16>> {
    let mut buffer = vec![0_u16; PATH_UNITS];
    let capacity = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
    // SAFETY: the handle is live; `buffer` holds `capacity` units.
    let len = unsafe {
        GetFinalPathNameByHandleW(
            dir.as_raw_handle(),
            buffer.as_mut_ptr(),
            capacity,
            FILE_NAME_NORMALIZED + VOLUME_NAME_DOS,
        )
    };
    let len = usize::try_from(len).unwrap_or(usize::MAX);
    if len == 0 {
        return Err(io::Error::last_os_error());
    }
    if len >= buffer.len() {
        return Err(io::Error::from(io::ErrorKind::OutOfMemory));
    }
    buffer.truncate(len);
    Ok(buffer)
}
