//! The registry side of autostart (#63): the Run value and Task Manager's
//! `StartupApproved` switch under HKCU. The rules are in
//! [`peekbuddy_core::autostart`]; this is only the values.
//!
//! The key paths are parameters so tests use a throw-away key under
//! `HKCU\Software\nobuddy\PeekBuddy-test-*` instead of the real Run key.

use std::io;

use peekbuddy_core::autostart::{Registry, VALUE_NAME};
use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_BINARY, REG_OPTION_NON_VOLATILE, REG_SZ,
    RRF_RT_REG_BINARY, RRF_RT_REG_SZ, RegCloseKey, RegCreateKeyExW, RegDeleteTreeW,
    RegDeleteValueW, RegGetValueW, RegOpenKeyExW, RegSetValueExW,
};

/// `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
pub const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// `HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run`.
pub const APPROVED_KEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn check(code: u32) -> io::Result<()> {
    if code == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(code.cast_signed()))
    }
}

/// An open key, closed on drop.
struct Key(HKEY);

impl Key {
    fn create(path: &str) -> io::Result<Self> {
        let name = wide(path);
        let mut key: HKEY = std::ptr::null_mut();
        // SAFETY: `name` is NUL-terminated; the optional pointers are null;
        // `key` is a valid out pointer.
        let code = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                name.as_ptr(),
                0,
                std::ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_READ | KEY_WRITE,
                std::ptr::null(),
                &raw mut key,
                std::ptr::null_mut(),
            )
        };
        check(code)?;
        Ok(Self(key))
    }

    /// The key if it exists.
    fn open(path: &str) -> Option<Self> {
        let name = wide(path);
        let mut key: HKEY = std::ptr::null_mut();
        // SAFETY: `name` is NUL-terminated; `key` is a valid out pointer.
        let code =
            unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, name.as_ptr(), 0, KEY_READ, &raw mut key) };
        (code == ERROR_SUCCESS).then_some(Self(key))
    }

    /// The raw bytes of a value of the given type filter.
    fn get(&self, name: &str, flags: u32) -> Option<Vec<u8>> {
        let name = wide(name);
        let mut len = 0_u32;
        // SAFETY: the key is open; `name` is NUL-terminated; a null data
        // pointer asks for the size only.
        let code = unsafe {
            RegGetValueW(
                self.0,
                std::ptr::null(),
                name.as_ptr(),
                flags,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &raw mut len,
            )
        };
        if code != ERROR_SUCCESS {
            return None;
        }
        let mut data = vec![0_u8; usize::try_from(len).ok()?];
        // SAFETY: as above, with `data` holding `len` bytes.
        let code = unsafe {
            RegGetValueW(
                self.0,
                std::ptr::null(),
                name.as_ptr(),
                flags,
                std::ptr::null_mut(),
                data.as_mut_ptr().cast(),
                &raw mut len,
            )
        };
        if code != ERROR_SUCCESS {
            return None;
        }
        data.truncate(usize::try_from(len).ok()?);
        Some(data)
    }

    fn set(&self, name: &str, kind: u32, data: &[u8]) -> io::Result<()> {
        let name = wide(name);
        let len = u32::try_from(data.len()).map_err(|_| io::ErrorKind::InvalidInput)?;
        // SAFETY: the key is open for writing; `name` is NUL-terminated;
        // `data` holds `len` bytes.
        let code = unsafe { RegSetValueExW(self.0, name.as_ptr(), 0, kind, data.as_ptr(), len) };
        check(code)
    }

    fn delete(&self, name: &str) -> io::Result<()> {
        let name = wide(name);
        // SAFETY: the key is open for writing; `name` is NUL-terminated.
        let code = unsafe { RegDeleteValueW(self.0, name.as_ptr()) };
        if code == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            check(code)
        }
    }
}

impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: the key was opened by this wrapper and is closed once.
        unsafe { RegCloseKey(self.0) };
    }
}

/// The two keys autostart uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunKeys {
    run: String,
    approved: String,
}

impl RunKeys {
    /// The real keys of the current user.
    #[must_use]
    pub fn current_user() -> Self {
        Self {
            run: RUN_KEY.to_owned(),
            approved: APPROVED_KEY.to_owned(),
        }
    }

    /// Keys under `base` (below HKCU): `base\Run` and
    /// `base\StartupApproved\Run`. For tests.
    #[must_use]
    pub fn under(base: &str) -> Self {
        Self {
            run: format!(r"{base}\Run"),
            approved: format!(r"{base}\StartupApproved\Run"),
        }
    }

    /// Deletes a test tree created with [`RunKeys::under`].
    ///
    /// # Errors
    ///
    /// The OS error when the tree cannot be removed.
    pub fn delete_tree(base: &str) -> io::Result<()> {
        let name = wide(base);
        // SAFETY: `name` is NUL-terminated.
        let code = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, name.as_ptr()) };
        if code == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            check(code)
        }
    }
}

fn utf16_value(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&pair| u16::from_le_bytes(pair))
        .take_while(|&unit| unit != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

impl Registry for RunKeys {
    type Error = io::Error;

    fn run_value(&self) -> Option<String> {
        let bytes = Key::open(&self.run)?.get(VALUE_NAME, RRF_RT_REG_SZ)?;
        Some(utf16_value(&bytes))
    }

    fn set_run_value(&mut self, value: &str) -> io::Result<()> {
        let bytes: Vec<u8> = wide(value)
            .iter()
            .flat_map(|unit| unit.to_le_bytes())
            .collect();
        Key::create(&self.run)?.set(VALUE_NAME, REG_SZ, &bytes)
    }

    fn delete_run_value(&mut self) -> io::Result<()> {
        match Key::open(&self.run) {
            // The delete needs write access, which `open` does not ask for.
            Some(_) => Key::create(&self.run)?.delete(VALUE_NAME),
            None => Ok(()),
        }
    }

    fn approved(&self) -> Option<Vec<u8>> {
        Key::open(&self.approved)?.get(VALUE_NAME, RRF_RT_REG_BINARY)
    }

    fn set_approved(&mut self, bytes: &[u8]) -> io::Result<()> {
        Key::create(&self.approved)?.set(VALUE_NAME, REG_BINARY, bytes)
    }
}
