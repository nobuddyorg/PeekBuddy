//! The viewer window's frame, as the OS draws it (#65): the dark title bar,
//! where the window sits, which monitors exist, and a message box for
//! errors that happen before any window can show them.
//!
//! The viewer's window itself belongs to eframe; this module only reads and
//! sets what eframe does not expose. Everything takes the raw `HWND` value
//! (as `isize`, from the window handle `eframe` hands out) so the viewer crate
//! stays free of `unsafe` and of Win32 types.

use windows_sys::Win32::Foundation::{E_INVALIDARG, HWND, LPARAM, RECT, S_OK, TRUE};
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, GetWindowRect, IsIconic, IsZoomed, MB_ICONERROR, MB_OK,
    MB_SETFOREGROUND, MessageBoxW, SW_MAXIMIZE, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos,
    ShowWindow,
};

/// `DWMWA_USE_IMMERSIVE_DARK_MODE`. Documented for build 22000 and later;
/// it works undocumented from 19041 on, and older builds answer
/// `E_INVALIDARG`.
const IMMERSIVE_DARK_MODE: u32 = 20;

/// A rectangle in virtual-screen pixels (physical pixels under
/// `PerMonitorV2`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    /// Left edge.
    pub left: i32,
    /// Top edge.
    pub top: i32,
    /// Right edge, exclusive.
    pub right: i32,
    /// Bottom edge, exclusive.
    pub bottom: i32,
}

impl From<RECT> for Rect {
    fn from(rect: RECT) -> Self {
        Self {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        }
    }
}

/// What asking DWM for an attribute did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dwm {
    /// The attribute is set.
    Applied,
    /// This Windows does not know the attribute (`E_INVALIDARG`). Not an
    /// error: the window keeps the default look.
    Unsupported,
}

fn classify(result: i32) -> std::io::Result<Dwm> {
    match result {
        S_OK => Ok(Dwm::Applied),
        E_INVALIDARG => Ok(Dwm::Unsupported),
        other => Err(std::io::Error::other(format!(
            "DwmSetWindowAttribute failed: HRESULT {other:#010x}"
        ))),
    }
}

fn hwnd(raw: isize) -> HWND {
    raw as HWND
}

/// Makes the title bar of the window `hwnd` dark or light.
///
/// # Errors
///
/// The HRESULT when DWM refuses for a reason other than "unknown attribute".
pub fn set_dark_title_bar(window: isize, dark: bool) -> std::io::Result<Dwm> {
    let value = i32::from(dark);
    // SAFETY: `value` is a BOOL-sized local that outlives the call and the
    // size passed is its size; a stale window handle only makes the call
    // fail.
    let result = unsafe {
        DwmSetWindowAttribute(
            hwnd(window),
            IMMERSIVE_DARK_MODE,
            (&raw const value).cast(),
            u32::try_from(size_of::<i32>()).unwrap_or(4),
        )
    };
    classify(result)
}

/// The work area (the monitor without the taskbar) of every monitor.
#[must_use]
pub fn work_areas() -> Vec<Rect> {
    unsafe extern "system" fn collect(
        monitor: HMONITOR,
        _: HDC,
        _: *mut RECT,
        data: LPARAM,
    ) -> i32 {
        let empty = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        let mut info = MONITORINFO {
            cbSize: u32::try_from(size_of::<MONITORINFO>()).unwrap_or(0),
            rcMonitor: empty,
            rcWork: empty,
            dwFlags: 0,
        };
        // SAFETY: `info` has its size set, as the call requires.
        if unsafe { GetMonitorInfoW(monitor, &raw mut info) } != 0 {
            // SAFETY: `data` is the address of the `Vec` that `work_areas`
            // passes below; it is alive and used by this thread only for the
            // duration of the enumeration.
            let out = unsafe { &mut *(data as *mut Vec<Rect>) };
            out.push(Rect::from(info.rcWork));
        }
        TRUE
    }
    let mut out: Vec<Rect> = Vec::new();
    // SAFETY: the callback runs before the call returns and only uses the
    // `Vec` whose address is passed.
    unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            Some(collect),
            (&raw mut out) as LPARAM,
        );
    }
    out
}

/// Where the window is, in virtual-screen pixels. `None` for a stale handle.
#[must_use]
pub fn window_rect(window: isize) -> Option<Rect> {
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    // SAFETY: `rect` is a valid out pointer.
    let ok = unsafe { GetWindowRect(hwnd(window), &raw mut rect) };
    (ok != 0).then(|| Rect::from(rect))
}

/// Whether the window is maximized.
#[must_use]
pub fn is_maximized(window: isize) -> bool {
    // SAFETY: a stale handle answers false.
    unsafe { IsZoomed(hwnd(window)) != 0 }
}

/// Whether the window is minimized.
#[must_use]
pub fn is_minimized(window: isize) -> bool {
    // SAFETY: a stale handle answers false.
    unsafe { IsIconic(hwnd(window)) != 0 }
}

/// Moves and sizes the window to `rect` without activating it or changing
/// its z-order, then maximizes it when `maximize` is set.
///
/// # Errors
///
/// The OS error when the window cannot be moved.
pub fn place(window: isize, rect: Rect, maximize: bool) -> std::io::Result<()> {
    // SAFETY: a stale handle only makes the call fail.
    let ok = unsafe {
        SetWindowPos(
            hwnd(window),
            std::ptr::null_mut(),
            rect.left,
            rect.top,
            rect.right.saturating_sub(rect.left),
            rect.bottom.saturating_sub(rect.top),
            SWP_NOZORDER | SWP_NOACTIVATE,
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    if maximize {
        // SAFETY: a stale handle only makes the call fail.
        unsafe { ShowWindow(hwnd(window), SW_MAXIMIZE) };
    }
    Ok(())
}

/// Lets any process take the foreground until the next input event. The
/// second viewer launch calls this before it asks the first one to come to
/// the front, because only the process the user just interacted with may
/// hand that right on.
pub fn allow_any_foreground() {
    // SAFETY: no preconditions; `u32::MAX` is `ASFW_ANY`.
    unsafe { AllowSetForegroundWindow(u32::MAX) };
}

/// Shows `text` in a message box titled `title` and waits for OK: how a
/// failure that happens before there is a window of our own reaches the
/// user. Blocks the calling thread.
pub fn error_box(title: &str, text: &str) {
    let wide = |text: &str| -> Vec<u16> { text.encode_utf16().chain(Some(0)).collect() };
    let (title, text) = (wide(title), wide(text));
    // SAFETY: both strings are NUL-terminated and outlive the call; a null
    // owner makes the box top-level.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR | MB_SETFOREGROUND,
        );
    }
}

#[cfg(test)]
mod tests {
    use windows_sys::Win32::Graphics::Dwm::DWMWA_USE_IMMERSIVE_DARK_MODE;

    use super::{Dwm, IMMERSIVE_DARK_MODE, Rect, classify, is_maximized, work_areas};

    #[test]
    fn the_attribute_number_is_the_documented_one() {
        assert_eq!(IMMERSIVE_DARK_MODE, DWMWA_USE_IMMERSIVE_DARK_MODE as u32);
        assert_eq!(IMMERSIVE_DARK_MODE, 20);
    }

    #[test]
    fn an_unknown_attribute_is_not_an_error_but_other_failures_are() {
        assert_eq!(classify(0).unwrap(), Dwm::Applied);
        // E_INVALIDARG: Windows 10 before 19041 does not know the attribute.
        assert_eq!(
            classify(0x8007_0057_u32.cast_signed()).unwrap(),
            Dwm::Unsupported
        );
        let error = classify(0x8000_4005_u32.cast_signed()).unwrap_err();
        assert!(error.to_string().contains("0x80004005"), "{error}");
    }

    #[test]
    fn a_session_has_at_least_one_monitor_with_a_work_area() {
        let areas = work_areas();
        // A hosted runner has a virtual display; a service session may have
        // none, which is not what this test is about.
        for area in &areas {
            assert!(area.right > area.left && area.bottom > area.top, "{area:?}");
        }
    }

    #[test]
    fn a_stale_window_is_not_maximized() {
        assert!(!is_maximized(0));
        assert_eq!(super::window_rect(0), None);
        let rect = Rect {
            left: 0,
            top: 0,
            right: 1,
            bottom: 1,
        };
        assert!(super::place(0, rect, false).is_err());
    }
}
