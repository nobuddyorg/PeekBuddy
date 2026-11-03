//! Showing a folder in Explorer (#69).
//!
//! Only Explorer is started, with a folder or `/select,<item>`; the item
//! itself is never opened, read or executed, so a cloud placeholder is not
//! hydrated and a hostile file does not run.

use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// Starts `explorer.exe` with `parameters` (for example `"C:\work"` or
/// `/select,"C:\work\a.txt"`). Does not wait for it.
///
/// # Errors
///
/// An error carrying the `ShellExecuteW` code when Explorer cannot be started.
pub fn explorer(parameters: &str) -> std::io::Result<()> {
    let (verb, file, params) = (wide("open"), wide("explorer.exe"), wide(parameters));
    // SAFETY: all three strings are NUL-terminated and outlive the call; a
    // null window and directory are allowed.
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            params.as_ptr(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    // The "instance handle" it returns is an error code up to 32.
    let code = result as isize;
    if code > 32 {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "ShellExecuteW could not start explorer.exe (code {code})"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::wide;

    #[test]
    fn a_wide_string_ends_with_a_nul() {
        assert_eq!(wide("ab"), [0x61, 0x62, 0]);
        assert_eq!(wide(""), [0]);
    }
}
