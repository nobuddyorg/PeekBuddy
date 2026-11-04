//! A named event between two processes of one session.
//!
//! The second viewer launch uses it to tell the first one to come to the
//! front (#65): it raises the event, the first viewer's waiting thread wakes
//! and asks eframe to focus the window. The thread blocks in the kernel, so
//! an idle viewer has no wakeups from it.

use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};

use windows_sys::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::System::Threading::{
    CreateEventW, INFINITE, SetEvent, WaitForSingleObject,
};

/// An auto-reset event with a name in the session's local namespace
/// (`Local\...`). The first process to create it makes it; the others open
/// the same one.
///
/// # Contract
///
/// Owns one event handle, closed when the value is dropped. Raising and
/// waiting are thread-safe kernel calls, so one `Signal` may be shared by
/// reference between a waiting thread and any other thread.
#[derive(Debug)]
pub struct Signal {
    handle: OwnedHandle,
}

impl Signal {
    /// Creates the event `name`, or opens it when it exists.
    ///
    /// # Errors
    ///
    /// The OS error when the event cannot be created or opened.
    pub fn open(name: &str) -> std::io::Result<Self> {
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        // SAFETY: `wide` is NUL-terminated and outlives the call; default
        // security: the session's user.
        let handle = unsafe { CreateEventW(std::ptr::null(), 0, 0, wide.as_ptr()) };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: CreateEventW succeeded, so `handle` is an owned handle.
        let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
        Ok(Self { handle })
    }

    /// Sets the event: one waiter wakes.
    ///
    /// # Errors
    ///
    /// The OS error.
    pub fn raise(&self) -> std::io::Result<()> {
        // SAFETY: the handle is live.
        if unsafe { SetEvent(self.handle.as_raw_handle()) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    /// Waits until the event is set. `None` waits forever. Returns whether
    /// it was set (false on a timeout).
    ///
    /// # Errors
    ///
    /// The OS error when the wait itself fails.
    pub fn wait(&self, timeout_ms: Option<u32>) -> std::io::Result<bool> {
        // SAFETY: the handle is live.
        let result = unsafe {
            WaitForSingleObject(self.handle.as_raw_handle(), timeout_ms.unwrap_or(INFINITE))
        };
        match result {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            _ => Err(std::io::Error::last_os_error()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Signal;

    fn name(tag: &str) -> String {
        format!(r"Local\PeekBuddy.Test.Signal.{tag}.{}", std::process::id())
    }

    #[test]
    fn a_raise_wakes_one_wait_and_resets() {
        let signal = Signal::open(&name("one")).unwrap();
        assert!(!signal.wait(Some(10)).unwrap());
        signal.raise().unwrap();
        assert!(signal.wait(Some(1_000)).unwrap());
        assert!(!signal.wait(Some(10)).unwrap(), "auto-reset");
    }

    #[test]
    fn two_opens_of_one_name_share_the_event() {
        let first = Signal::open(&name("two")).unwrap();
        let second = Signal::open(&name("two")).unwrap();
        second.raise().unwrap();
        assert!(first.wait(Some(1_000)).unwrap());
    }

    #[test]
    fn a_waiting_thread_is_woken_from_another() {
        let signal = std::sync::Arc::new(Signal::open(&name("thread")).unwrap());
        let waiter = {
            let signal = std::sync::Arc::clone(&signal);
            std::thread::spawn(move || signal.wait(Some(5_000)).unwrap())
        };
        std::thread::sleep(std::time::Duration::from_millis(50));
        signal.raise().unwrap();
        assert!(waiter.join().unwrap());
    }
}
