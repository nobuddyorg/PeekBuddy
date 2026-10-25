//! One process per role: a named mutex in the session's local namespace.

use std::os::windows::io::{FromRawHandle as _, OwnedHandle};

use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
use windows_sys::Win32::System::Threading::CreateMutexW;

/// Holds the named mutex of a role for as long as the value lives.
#[derive(Debug)]
pub struct SingleInstance {
    _mutex: OwnedHandle,
}

impl SingleInstance {
    /// Takes the mutex `name` (for example `Local\PeekBuddy.{GUID}`).
    ///
    /// Returns `Ok(None)` when another process of this session already owns
    /// it: the caller is then the second instance.
    ///
    /// # Errors
    ///
    /// Returns the OS error when the mutex cannot be created.
    pub fn acquire(name: &str) -> std::io::Result<Option<Self>> {
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        // SAFETY: `wide` is a NUL-terminated UTF-16 string that outlives the
        // call; no security attributes are passed.
        let handle = unsafe { CreateMutexW(std::ptr::null(), 0, wide.as_ptr()) };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: reading the thread's last error right after the call.
        let already = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        // SAFETY: CreateMutexW succeeded, so `handle` is an owned handle.
        let mutex = unsafe { OwnedHandle::from_raw_handle(handle) };
        if already {
            return Ok(None);
        }
        Ok(Some(Self { _mutex: mutex }))
    }
}

#[cfg(test)]
mod tests {
    use super::SingleInstance;

    #[test]
    fn the_second_acquire_of_a_name_is_refused() {
        let name = format!("Local\\PeekBuddy.test.{}", std::process::id());
        let first = SingleInstance::acquire(&name).unwrap();
        assert!(first.is_some());
        assert!(SingleInstance::acquire(&name).unwrap().is_none());
        drop(first);
        assert!(SingleInstance::acquire(&name).unwrap().is_some());
    }

    #[test]
    fn different_names_do_not_collide() {
        let id = std::process::id();
        let a = SingleInstance::acquire(&format!("Local\\PeekBuddy.test.a.{id}")).unwrap();
        let b = SingleInstance::acquire(&format!("Local\\PeekBuddy.test.b.{id}")).unwrap();
        assert!(a.is_some() && b.is_some());
    }
}
