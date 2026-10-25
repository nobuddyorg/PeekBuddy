//! File attributes the data directory needs.

use std::os::windows::ffi::OsStrExt as _;
use std::os::windows::fs::MetadataExt as _;
use std::path::Path;

use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_NOT_CONTENT_INDEXED, SetFileAttributesW,
};

/// Sets `FILE_ATTRIBUTE_NOT_CONTENT_INDEXED` on `path`, keeping its other
/// attributes. Done before the first SQLite file is created in it (ADR 0008).
///
/// # Errors
///
/// Returns the OS error when the attributes cannot be read or written.
pub fn set_not_content_indexed(path: &Path) -> std::io::Result<()> {
    let current = std::fs::metadata(path)?.file_attributes();
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: `wide` is a NUL-terminated UTF-16 path that outlives the call.
    let ok =
        unsafe { SetFileAttributesW(wide.as_ptr(), current | FILE_ATTRIBUTE_NOT_CONTENT_INDEXED) };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Whether `path` carries `FILE_ATTRIBUTE_NOT_CONTENT_INDEXED`.
///
/// # Errors
///
/// Returns the OS error when the attributes cannot be read.
pub fn is_not_content_indexed(path: &Path) -> std::io::Result<bool> {
    let attributes = std::fs::metadata(path)?.file_attributes();
    Ok(attributes & FILE_ATTRIBUTE_NOT_CONTENT_INDEXED != 0)
}

#[cfg(test)]
mod tests {
    use super::{is_not_content_indexed, set_not_content_indexed};

    #[test]
    fn the_attribute_is_set_and_the_directory_stays_one() {
        let dir = std::env::temp_dir().join(format!("pb-attrs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!is_not_content_indexed(&dir).unwrap());
        set_not_content_indexed(&dir).unwrap();
        assert!(is_not_content_indexed(&dir).unwrap());
        assert!(dir.is_dir());
        std::fs::remove_dir(&dir).unwrap();
    }
}
