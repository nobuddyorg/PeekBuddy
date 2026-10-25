//! The engine's hidden top-level window and its message loop.
//!
//! The window is top-level, never shown, and not `HWND_MESSAGE`: message-only
//! windows do not receive broadcasts such as `WM_QUERYENDSESSION`,
//! `WM_POWERBROADCAST` and `WM_DEVICECHANGE`. It anchors the tray (#60), the
//! session-end handling and the power and volume signals of #48.

use windows_sys::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, WPARAM,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    FindWindowW, GWLP_USERDATA, GetMessageW, GetWindowLongPtrW, KillTimer, MSG, PostMessageW,
    PostQuitMessage, RegisterClassExW, SMTO_ABORTIFHUNG, SendMessageTimeoutW, SetTimer,
    SetWindowLongPtrW, TranslateMessage, WM_CLOSE, WM_DESTROY, WM_ENDSESSION, WM_NCCREATE,
    WM_QUERYENDSESSION, WM_TIMER, WNDCLASSEXW,
};

/// Id of the single timer a hidden window owns.
const TIMER_ID: usize = 1;

/// What the window procedure reports to the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowEvent {
    /// `WM_QUERYENDSESSION`: always answered TRUE at once; the flush runs on
    /// [`WindowEvent::EndSession`].
    QueryEndSession,
    /// `WM_ENDSESSION`; `ending` is false when the shutdown was cancelled.
    EndSession {
        /// Whether the session really ends.
        ending: bool,
    },
    /// `WM_CLOSE`: Quit, pipe shutdown or Ctrl+C. The window is destroyed
    /// after the handler returns.
    Close,
    /// The one-shot timer set with [`HiddenWindow::set_one_shot_timer`] fired.
    Timer,
}

type Handler = Box<dyn FnMut(WindowEvent, WindowHandle)>;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// A window handle that may be used from any thread and any process:
/// posting and sending messages is thread-safe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowHandle(isize);

impl WindowHandle {
    /// Finds a top-level window of `class`.
    #[must_use]
    pub fn find(class: &str) -> Option<Self> {
        let class = wide(class);
        // SAFETY: `class` is NUL-terminated and outlives the call; a null
        // title matches any window name.
        let hwnd = unsafe { FindWindowW(class.as_ptr(), std::ptr::null()) };
        (!hwnd.is_null()).then_some(Self(hwnd as isize))
    }

    /// The raw handle value, for hand-off to a console control handler.
    #[must_use]
    pub const fn raw(self) -> isize {
        self.0
    }

    fn hwnd(self) -> HWND {
        self.0 as HWND
    }

    /// Asks the window to close (`WM_CLOSE`), without waiting.
    ///
    /// # Errors
    ///
    /// Returns the OS error when the message cannot be posted.
    pub fn post_close(self) -> std::io::Result<()> {
        // SAFETY: PostMessageW accepts any window handle and fails cleanly on
        // a stale one.
        if unsafe { PostMessageW(self.hwnd(), WM_CLOSE, 0, 0) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    /// Sends a message and waits up to `timeout_ms` for the answer.
    ///
    /// # Errors
    ///
    /// Returns the OS error when the window does not answer in time.
    pub fn send_timeout(
        self,
        message: u32,
        wparam: usize,
        lparam: isize,
        timeout_ms: u32,
    ) -> std::io::Result<isize> {
        let mut result = 0usize;
        // SAFETY: `result` is a valid out pointer; the window may belong to
        // another process, in which case only plain integers cross.
        let sent = unsafe {
            SendMessageTimeoutW(
                self.hwnd(),
                message,
                wparam,
                lparam,
                SMTO_ABORTIFHUNG,
                timeout_ms,
                &raw mut result,
            )
        };
        if sent == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(isize::try_from(result).unwrap_or(isize::MAX))
    }

    /// Sends `WM_QUERYENDSESSION` and returns the window's answer.
    ///
    /// # Errors
    ///
    /// See [`WindowHandle::send_timeout`].
    pub fn query_end_session(self, timeout_ms: u32) -> std::io::Result<bool> {
        self.send_timeout(WM_QUERYENDSESSION, 0, 0, timeout_ms)
            .map(|answer| answer != 0)
    }

    /// Sends `WM_ENDSESSION` (session ending) and waits for the handler.
    ///
    /// # Errors
    ///
    /// See [`WindowHandle::send_timeout`].
    pub fn end_session(self, timeout_ms: u32) -> std::io::Result<()> {
        self.send_timeout(WM_ENDSESSION, 1, 0, timeout_ms).map(drop)
    }
}

/// The hidden window. Dropping it destroys the window.
pub struct HiddenWindow {
    hwnd: HWND,
    // Owned by the window through GWLP_USERDATA; freed in `Drop` after the
    // window is gone.
    state: *mut Handler,
}

impl std::fmt::Debug for HiddenWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HiddenWindow")
            .field("hwnd", &self.hwnd)
            .finish_non_exhaustive()
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        // SAFETY: for WM_NCCREATE `lparam` points to the CREATESTRUCTW the
        // system built for this call.
        let create = unsafe { &*(lparam as *const CREATESTRUCTW) };
        // SAFETY: stores the handler pointer passed to CreateWindowExW.
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize) };
        // SAFETY: default processing for the message we just inspected.
        return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
    }
    // SAFETY: reads the value stored above; null before WM_NCCREATE.
    let state = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut Handler;
    let call = |event: WindowEvent| {
        let handle = WindowHandle(hwnd as isize);
        if !state.is_null() {
            // SAFETY: `state` came from `Box::into_raw` in `create`, lives
            // until `Drop` for `HiddenWindow`, and the window procedure only
            // runs on the thread that owns the window, one message at a time.
            unsafe { (*state)(event, handle) };
        }
    };
    match message {
        WM_QUERYENDSESSION => {
            call(WindowEvent::QueryEndSession);
            1
        }
        WM_ENDSESSION => {
            call(WindowEvent::EndSession {
                ending: wparam != 0,
            });
            0
        }
        WM_TIMER => {
            // SAFETY: kills the one-shot timer of this window.
            unsafe { KillTimer(hwnd, TIMER_ID) };
            call(WindowEvent::Timer);
            0
        }
        WM_CLOSE => {
            call(WindowEvent::Close);
            // SAFETY: `hwnd` is the window this procedure belongs to.
            unsafe { DestroyWindow(hwnd) };
            0
        }
        WM_DESTROY => {
            // SAFETY: no preconditions; ends the loop of this thread.
            unsafe { PostQuitMessage(0) };
            0
        }
        // SAFETY: default processing of the unchanged message.
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

impl HiddenWindow {
    /// Creates the hidden top-level window of `class` on this thread.
    ///
    /// # Errors
    ///
    /// Returns the OS error when the class or window cannot be created.
    pub fn create(
        class: &str,
        handler: impl FnMut(WindowEvent, WindowHandle) + 'static,
    ) -> std::io::Result<Self> {
        let class_name = wide(class);
        // SAFETY: a null module name returns this exe's handle.
        let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
        let class_ex = WNDCLASSEXW {
            cbSize: u32::try_from(size_of::<WNDCLASSEXW>()).unwrap_or(u32::MAX),
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: class_name.as_ptr(),
            ..WNDCLASSEXW::default()
        };
        // SAFETY: `class_ex` is fully initialised and the class name outlives
        // the call; registration copies it.
        let registered = unsafe { RegisterClassExW(&raw const class_ex) };
        // SAFETY: reading the last error right after the call.
        if registered == 0 && unsafe { GetLastError() } != ERROR_CLASS_ALREADY_EXISTS {
            return Err(std::io::Error::last_os_error());
        }
        let state: *mut Handler = Box::into_raw(Box::new(Box::new(handler) as Handler));
        // SAFETY: the class is registered; `state` is passed as creation
        // data and read back in WM_NCCREATE; a null parent makes the window
        // top-level, and it is never shown.
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class_name.as_ptr(),
                class_name.as_ptr(),
                0,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance,
                state.cast(),
            )
        };
        if hwnd.is_null() {
            let error = std::io::Error::last_os_error();
            // SAFETY: the window was not created, so nothing else owns `state`.
            drop(unsafe { Box::from_raw(state) });
            return Err(error);
        }
        Ok(Self { hwnd, state })
    }

    /// Arms a one-shot timer that delivers [`WindowEvent::Timer`] once after
    /// `milliseconds`. It is the only timer the engine owns, and only the
    /// perf probe uses it (F6).
    ///
    /// # Errors
    ///
    /// Returns the OS error when the timer cannot be set.
    pub fn set_one_shot_timer(&self, milliseconds: u32) -> std::io::Result<()> {
        // SAFETY: `hwnd` is this window; a null procedure posts WM_TIMER.
        if unsafe { SetTimer(self.hwnd, TIMER_ID, milliseconds, None) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    /// The thread-safe handle of this window.
    #[must_use]
    pub fn handle(&self) -> WindowHandle {
        WindowHandle(self.hwnd as isize)
    }

    /// Runs the message loop until the window is destroyed; returns the exit
    /// code posted by `WM_DESTROY`.
    #[must_use]
    pub fn run_message_loop(&self) -> i32 {
        let mut message = MSG::default();
        loop {
            // SAFETY: `message` is a valid out pointer; a null window receives
            // every message of this thread.
            let got = unsafe { GetMessageW(&raw mut message, std::ptr::null_mut(), 0, 0) };
            if got <= 0 {
                return i32::try_from(message.wParam).unwrap_or(0);
            }
            // SAFETY: `message` was filled by GetMessageW.
            unsafe { TranslateMessage(&raw const message) };
            // SAFETY: as above.
            unsafe { DispatchMessageW(&raw const message) };
        }
    }
}

impl Drop for HiddenWindow {
    fn drop(&mut self) {
        // SAFETY: destroying an already destroyed window only fails.
        unsafe { DestroyWindow(self.hwnd) };
        // SAFETY: the window is gone, so nothing reads the handler any more.
        drop(unsafe { Box::from_raw(self.state) });
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::{HiddenWindow, WindowEvent, WindowHandle};

    fn class(name: &str) -> String {
        format!("PeekBuddy.Test.{name}.{}", std::process::id())
    }

    #[test]
    fn events_reach_the_handler_and_close_ends_the_loop() {
        let class = class("events");
        let seen = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&seen);
        let window =
            HiddenWindow::create(&class, move |event, _| sink.borrow_mut().push(event)).unwrap();
        let handle = window.handle();
        assert_eq!(WindowHandle::find(&class), Some(handle));
        // The loop runs on this thread, so post instead of send.
        handle.post_close().unwrap();
        assert_eq!(window.run_message_loop(), 0);
        assert_eq!(*seen.borrow(), vec![WindowEvent::Close]);
    }

    #[test]
    fn the_one_shot_timer_fires_once() {
        let class = class("timer");
        let seen = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&seen);
        let window = HiddenWindow::create(&class, move |event, handle| {
            sink.borrow_mut().push(event);
            if event == WindowEvent::Timer {
                handle.post_close().unwrap();
            }
        })
        .unwrap();
        window.set_one_shot_timer(20).unwrap();
        assert_eq!(window.run_message_loop(), 0);
        assert_eq!(*seen.borrow(), vec![WindowEvent::Timer, WindowEvent::Close]);
    }

    #[test]
    fn the_window_is_top_level_and_hidden() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetParent, IsWindowVisible};
        let class = class("hidden");
        let window = HiddenWindow::create(&class, |_, _| {}).unwrap();
        // SAFETY: the handle belongs to the window created above.
        let parent = unsafe { GetParent(window.hwnd) };
        // SAFETY: as above.
        let visible = unsafe { IsWindowVisible(window.hwnd) };
        assert!(parent.is_null());
        assert_eq!(visible, 0);
    }
}
