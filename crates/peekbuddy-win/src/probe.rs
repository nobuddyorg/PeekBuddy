//! Is the watched root still there? (#48)
//!
//! After an unexpected completion status the lifecycle code does not guess:
//! it asks the volume. [`probe_root`] opens the root, then checks that its
//! volume is mounted (`FSCTL_IS_VOLUME_MOUNTED`) and, when the root cannot be
//! opened, whether the volume's own root can, to tell a deleted root from a
//! vanished device.

use std::os::windows::io::AsRawHandle;

use peekbuddy_core::lifecycle::Verdict;
use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_DELETE_PENDING, ERROR_FILE_NOT_FOUND, ERROR_NOT_READY,
    ERROR_PATH_NOT_FOUND,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::FSCTL_IS_VOLUME_MOUNTED;

use crate::io::dir::DirHandle;
use crate::io::open::OpenOptions;

/// What a probe found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    /// The root and its volume are fine.
    Ok,
    /// The volume is there but not ready (locked, medium not ready).
    NotReady,
    /// The volume is gone.
    VolumeGone,
    /// The volume is fine but the root directory is gone.
    RootGone,
}

impl Probe {
    /// The verdict for the lifecycle machine; a gone root is not a verdict
    /// about the volume and is reported by the caller as a deleted root.
    #[must_use]
    pub const fn verdict(self) -> Option<Verdict> {
        match self {
            Self::Ok => Some(Verdict::Ok),
            Self::NotReady => Some(Verdict::NotReady),
            Self::VolumeGone => Some(Verdict::Gone),
            Self::RootGone => None,
        }
    }
}

fn code(error: &std::io::Error) -> u32 {
    error.raw_os_error().map_or(0, i32::cast_unsigned)
}

/// The volume root of a `\\?\X:\...` path (`\\?\X:\`), if it has one.
fn volume_root(path: &[u16]) -> Option<Vec<u16>> {
    // `\\?\` + drive letter + `:` is 6 units.
    let head = path.get(..6)?;
    let letter = head.get(4).copied()?;
    let is_drive = u8::try_from(letter).is_ok_and(|b| b.is_ascii_alphabetic())
        && head.get(5) == Some(&u16::from(b':'));
    if !is_drive {
        return None;
    }
    let mut root = head.to_vec();
    root.push(u16::from(b'\\'));
    Some(root)
}

fn is_mounted(dir: &DirHandle) -> Result<(), u32> {
    let mut returned = 0_u32;
    // SAFETY: the handle is open; the control code takes no input and no
    // output buffer; `returned` is a valid out pointer.
    let ok = unsafe {
        DeviceIoControl(
            dir.as_raw_handle(),
            FSCTL_IS_VOLUME_MOUNTED,
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            0,
            &raw mut returned,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        Err(code(&std::io::Error::last_os_error()))
    } else {
        Ok(())
    }
}

/// Probes the root at `root_long` (a `\\?\` path).
#[must_use]
pub fn probe_root(root_long: &[u16]) -> Probe {
    let options = OpenOptions {
        reparse_point: true,
        no_recall: true,
    };
    match DirHandle::open(root_long, options) {
        Ok(dir) => match is_mounted(&dir) {
            Ok(()) => Probe::Ok,
            Err(ERROR_NOT_READY) => Probe::NotReady,
            Err(_) => Probe::VolumeGone,
        },
        Err(error) => match code(&error) {
            ERROR_NOT_READY => Probe::NotReady,
            ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND | ERROR_ACCESS_DENIED
            | ERROR_DELETE_PENDING => {
                // The root cannot be opened: is its volume still there?
                match volume_root(root_long) {
                    Some(volume) => match DirHandle::open(&volume, options) {
                        Ok(_) => Probe::RootGone,
                        Err(inner) if code(&inner) == ERROR_NOT_READY => Probe::NotReady,
                        Err(_) => Probe::VolumeGone,
                    },
                    // A UNC or volume-GUID path: no drive letter to ask.
                    None => Probe::RootGone,
                }
            }
            _ => Probe::VolumeGone,
        },
    }
}

#[cfg(test)]
mod tests {
    use peekbuddy_core::lifecycle::Verdict;

    use super::{Probe, code, probe_root, volume_root};

    fn w(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    #[test]
    fn a_drive_path_has_a_volume_root() {
        assert_eq!(volume_root(&w(r"\\?\C:\dir\sub")), Some(w(r"\\?\C:\")));
        assert_eq!(volume_root(&w(r"\\?\z:\x")), Some(w(r"\\?\z:\")));
        // Just the drive, no further path.
        assert_eq!(volume_root(&w(r"\\?\D:")), Some(w(r"\\?\D:\")));
    }

    #[test]
    fn anything_else_has_no_drive_to_ask() {
        assert_eq!(volume_root(&w(r"\\?\UNC\server\share\x")), None);
        assert_eq!(volume_root(&w(r"\\?\Volume{1234}\x")), None);
        // Not a letter, no colon, too short.
        assert_eq!(volume_root(&w(r"\\?\1:\x")), None);
        assert_eq!(volume_root(&w(r"\\?\C-\x")), None);
        assert_eq!(volume_root(&w(r"\\?\C")), None);
        assert_eq!(volume_root(&w("")), None);
        // A non-ASCII "letter" is no drive.
        assert_eq!(volume_root(&w("\\\\?\\\u{E4}:\\x")), None);
    }

    #[test]
    fn an_os_error_is_its_code_and_other_errors_are_zero() {
        assert_eq!(code(&std::io::Error::from_raw_os_error(5)), 5);
        assert_eq!(code(&std::io::Error::from_raw_os_error(2)), 2);
        assert_eq!(code(&std::io::Error::other("no code")), 0);
    }

    #[test]
    fn the_verdicts_of_the_probes() {
        assert_eq!(Probe::Ok.verdict(), Some(Verdict::Ok));
        assert_eq!(Probe::NotReady.verdict(), Some(Verdict::NotReady));
        assert_eq!(Probe::VolumeGone.verdict(), Some(Verdict::Gone));
        assert_eq!(Probe::RootGone.verdict(), None);
    }

    fn long(path: &std::path::Path) -> Vec<u16> {
        let canonical = std::fs::canonicalize(path).unwrap();
        canonical.to_string_lossy().encode_utf16().collect()
    }

    #[test]
    fn an_existing_directory_is_ok() {
        let dir = std::env::temp_dir().join(format!("pb-probe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(probe_root(&long(&dir)), Probe::Ok);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_deleted_directory_on_a_live_volume_is_a_gone_root() {
        let dir = std::env::temp_dir().join(format!("pb-probe-gone-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = long(&dir);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(probe_root(&path), Probe::RootGone);
    }

    #[test]
    fn a_path_below_a_file_is_a_gone_root_too() {
        let dir = std::env::temp_dir().join(format!("pb-probe-file-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("f.txt");
        std::fs::write(&file, b"x").unwrap();
        let mut path = long(&file);
        path.extend(r"\below".encode_utf16());
        assert_eq!(probe_root(&path), Probe::RootGone);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_volume_that_does_not_exist_is_gone() {
        // The first drive letter that is not in use.
        // SAFETY: GetLogicalDrives takes no arguments.
        let used = unsafe { windows_sys::Win32::Storage::FileSystem::GetLogicalDrives() };
        let free = (0..26u8).rev().find(|bit| used & (1 << bit) == 0).unwrap();
        let path = format!(r"\\?\{}:\gone\dir", char::from(b'A' + free));
        assert_eq!(probe_root(&w(&path)), Probe::VolumeGone);
    }

    #[test]
    fn a_unc_or_guid_path_that_is_missing_is_a_gone_root() {
        // No drive letter to ask: the root is reported, not the volume.
        assert_eq!(
            probe_root(&w(r"\\?\Volume{00000000-0000-0000-0000-000000000000}\x")),
            Probe::RootGone
        );
    }
}
