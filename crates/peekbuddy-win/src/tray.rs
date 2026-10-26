//! The notification-area icon and its context menu (#60), on raw
//! `Shell_NotifyIconW` and `TrackPopupMenuEx`.
//!
//! The icon is identified by window handle and `uID` (not `guidItem`, which
//! ties it to the exe path and breaks for a portable exe). Messages arrive
//! through the engine's hidden window with `NOTIFYICON_VERSION_4` semantics.
//! Nothing here owns a timer: the menu is built when it is shown and the icon
//! only changes when the engine says so (F6).

use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject};
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE,
    NIM_MODIFY, NIM_SETVERSION, NOTIFYICON_VERSION_4, NOTIFYICONDATAW, NOTIFYICONIDENTIFIER,
    Shell_NotifyIconGetRect, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, DestroyIcon, DestroyMenu, GetCursorPos,
    GetMenuItemCount, HICON, HMENU, ICONINFO, MF_CHECKED, MF_DEFAULT, MF_DISABLED, MF_GRAYED,
    MF_POPUP, MF_SEPARATOR, MF_STRING, PostMessageW, RegisterWindowMessageW, SetForegroundWindow,
    SetMenuDefaultItem, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenuEx, WM_APP, WM_NULL,
};

use crate::window::WindowHandle;

/// Message the shell sends to the window about the icon (`WM_APP + 1`).
pub const CALLBACK_MESSAGE: u32 = WM_APP + 1;
/// Identity of the one icon, with the window handle.
const ICON_ID: u32 = 1;

fn last_error() -> std::io::Error {
    std::io::Error::last_os_error()
}

fn copy_units(text: &str, into: &mut [u16]) {
    // Always leaves room for the terminating NUL.
    let room = into.len().saturating_sub(1);
    for (slot, unit) in into.iter_mut().zip(text.encode_utf16().take(room)) {
        *slot = unit;
    }
}

/// An icon created from pixels; destroyed on drop.
#[derive(Debug)]
pub struct Icon(HICON);

// SAFETY: an HICON is a process-wide handle, not tied to a thread.
unsafe impl Send for Icon {}

impl Icon {
    /// Creates a `size` x `size` icon from premultiplied BGRA pixels.
    ///
    /// # Errors
    ///
    /// An error when the pixel count does not match or GDI refuses.
    pub fn from_bgra(size: u32, pixels: &[u8]) -> std::io::Result<Self> {
        let edge = i32::try_from(size).map_err(std::io::Error::other)?;
        let count = (size as usize) * (size as usize) * 4;
        if pixels.len() != count {
            return Err(std::io::Error::other("the pixel buffer has the wrong size"));
        }
        // SAFETY: `pixels` holds size * size 32-bit pixels, checked above.
        let colour = unsafe { CreateBitmap(edge, edge, 1, 32, pixels.as_ptr().cast()) };
        // SAFETY: a null bits pointer makes an uninitialised monochrome mask,
        // which is ignored for 32-bit colour with alpha.
        let mask = unsafe { CreateBitmap(edge, edge, 1, 1, std::ptr::null()) };
        if colour.is_null() || mask.is_null() {
            return Err(last_error());
        }
        let info = ICONINFO {
            fIcon: 1,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: colour,
        };
        // SAFETY: both bitmaps are valid; the icon copies them.
        let icon = unsafe { CreateIconIndirect(&raw const info) };
        let error = last_error();
        // SAFETY: the bitmaps are ours and no longer needed.
        unsafe { DeleteObject(colour) };
        // SAFETY: as above.
        unsafe { DeleteObject(mask) };
        if icon.is_null() {
            return Err(error);
        }
        Ok(Self(icon))
    }
}

impl Drop for Icon {
    fn drop(&mut self) {
        // SAFETY: the icon was created by CreateIconIndirect and is ours.
        unsafe { DestroyIcon(self.0) };
    }
}

/// The notification-area icon of one window.
#[derive(Debug)]
pub struct Tray {
    hwnd: HWND,
}

impl Tray {
    /// Binds the icon to `window`; nothing is shown until [`Tray::add`].
    #[must_use]
    pub fn new(window: WindowHandle) -> Self {
        Self {
            hwnd: window.raw() as HWND,
        }
    }

    fn data(&self) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: u32::try_from(size_of::<NOTIFYICONDATAW>()).unwrap_or(0),
            hWnd: self.hwnd,
            uID: ICON_ID,
            ..NOTIFYICONDATAW::default()
        }
    }

    fn notify(message: u32, data: &NOTIFYICONDATAW) -> std::io::Result<()> {
        // SAFETY: `data` is a fully initialised NOTIFYICONDATAW with its size.
        if unsafe { Shell_NotifyIconW(message, data) } == 0 {
            return Err(last_error());
        }
        Ok(())
    }

    /// Shows the icon (or updates it when it is already there) and selects
    /// version-4 callbacks.
    ///
    /// # Errors
    ///
    /// The shell's error, for example while the taskbar does not exist yet;
    /// the engine retries on `TaskbarCreated`.
    pub fn add(&self, icon: &Icon, tip: &str) -> std::io::Result<()> {
        let mut data = self.data();
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
        data.uCallbackMessage = CALLBACK_MESSAGE;
        data.hIcon = icon.0;
        copy_units(tip, &mut data.szTip);
        if Self::notify(NIM_ADD, &data).is_err() {
            // Already added (a repeated TaskbarCreated): modify instead.
            Self::notify(NIM_MODIFY, &data)?;
        }
        let mut version = self.data();
        version.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        Self::notify(NIM_SETVERSION, &version)
    }

    /// Changes icon and tooltip.
    ///
    /// # Errors
    ///
    /// The shell's error.
    pub fn update(&self, icon: &Icon, tip: &str) -> std::io::Result<()> {
        let mut data = self.data();
        data.uFlags = NIF_ICON | NIF_TIP | NIF_SHOWTIP;
        data.hIcon = icon.0;
        copy_units(tip, &mut data.szTip);
        Self::notify(NIM_MODIFY, &data)
    }

    /// Shows one balloon (the first-run hint).
    ///
    /// # Errors
    ///
    /// The shell's error.
    pub fn balloon(&self, title: &str, text: &str) -> std::io::Result<()> {
        let mut data = self.data();
        data.uFlags = NIF_INFO;
        data.dwInfoFlags = NIIF_INFO;
        copy_units(title, &mut data.szInfoTitle);
        copy_units(text, &mut data.szInfo);
        Self::notify(NIM_MODIFY, &data)
    }

    /// Removes the icon. Safe to call when it is not there.
    pub fn remove(&self) {
        let _ = Self::notify(NIM_DELETE, &self.data());
    }

    /// Whether the shell knows the icon (it can be located on screen).
    #[must_use]
    pub fn is_present(&self) -> bool {
        let id = NOTIFYICONIDENTIFIER {
            cbSize: u32::try_from(size_of::<NOTIFYICONIDENTIFIER>()).unwrap_or(0),
            hWnd: self.hwnd,
            uID: ICON_ID,
            ..NOTIFYICONIDENTIFIER::default()
        };
        let mut rect = RECT::default();
        // SAFETY: `id` and `rect` are valid for the call.
        unsafe { Shell_NotifyIconGetRect(&raw const id, &raw mut rect) >= 0 }
    }
}

/// The id of the `TaskbarCreated` broadcast (explorer restarted).
#[must_use]
pub fn taskbar_created_message() -> u32 {
    let name: Vec<u16> = "TaskbarCreated".encode_utf16().chain(Some(0)).collect();
    // SAFETY: `name` is NUL-terminated and outlives the call.
    unsafe { RegisterWindowMessageW(name.as_ptr()) }
}

/// Whether the taskbar uses the light theme (`SystemUsesLightTheme`).
#[must_use]
pub fn taskbar_uses_light_theme() -> bool {
    crate::osinfo::current_user_dword(
        r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
        "SystemUsesLightTheme",
    ) == Some(1)
}

/// One menu entry, mirrored from the engine's model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// A command.
    Item {
        /// Menu id.
        id: u32,
        /// Label.
        label: String,
        /// The default (bold) item.
        default: bool,
        /// Shown checked.
        checked: bool,
        /// Greyed out when false.
        enabled: bool,
    },
    /// A separator.
    Separator,
    /// A submenu.
    Submenu {
        /// Label.
        label: String,
        /// Entries.
        items: Vec<Entry>,
        /// Greyed out when false.
        enabled: bool,
    },
}

/// A built popup menu; destroyed on drop (which also destroys submenus).
#[derive(Debug)]
pub struct Menu(HMENU);

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

fn fill(menu: HMENU, entries: &[Entry]) -> std::io::Result<()> {
    for entry in entries {
        match entry {
            Entry::Separator => {
                // SAFETY: a separator takes no id and no text.
                if unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null()) } == 0 {
                    return Err(last_error());
                }
            }
            Entry::Item {
                id,
                label,
                default,
                checked,
                enabled,
            } => {
                let mut flags = MF_STRING;
                if *checked {
                    flags |= MF_CHECKED;
                }
                if *default {
                    flags |= MF_DEFAULT;
                }
                if !*enabled {
                    flags |= MF_GRAYED | MF_DISABLED;
                }
                let text = wide(label);
                // SAFETY: `text` is NUL-terminated and outlives the call.
                if unsafe { AppendMenuW(menu, flags, *id as usize, text.as_ptr()) } == 0 {
                    return Err(last_error());
                }
            }
            Entry::Submenu {
                label,
                items,
                enabled,
            } => {
                // SAFETY: no preconditions.
                let sub = unsafe { CreatePopupMenu() };
                if sub.is_null() {
                    return Err(last_error());
                }
                fill(sub, items)?;
                let mut flags = MF_POPUP | MF_STRING;
                if !*enabled {
                    flags |= MF_GRAYED | MF_DISABLED;
                }
                let text = wide(label);
                // SAFETY: `sub` is a valid popup menu whose ownership passes
                // to `menu`; `text` is NUL-terminated.
                if unsafe { AppendMenuW(menu, flags, sub as usize, text.as_ptr()) } == 0 {
                    return Err(last_error());
                }
            }
        }
    }
    Ok(())
}

impl Menu {
    /// Builds the popup menu for `entries`.
    ///
    /// # Errors
    ///
    /// The OS error when the menu cannot be built.
    pub fn build(entries: &[Entry]) -> std::io::Result<Self> {
        // SAFETY: no preconditions.
        let handle = unsafe { CreatePopupMenu() };
        if handle.is_null() {
            return Err(last_error());
        }
        let menu = Self(handle);
        fill(handle, entries)?;
        Ok(menu)
    }

    /// Top-level entries, separators included.
    #[must_use]
    pub fn item_count(&self) -> i32 {
        // SAFETY: the handle is a live menu.
        unsafe { GetMenuItemCount(self.0) }
    }

    /// Shows the menu at the cursor for `window` and returns the chosen id.
    /// `window` must be foreground first, or the menu does not close when the
    /// user clicks elsewhere; `WM_NULL` afterwards completes the sequence the
    /// documentation asks for.
    #[must_use]
    pub fn track(&self, window: WindowHandle, default_id: Option<u32>) -> Option<u32> {
        let hwnd = window.raw() as HWND;
        let mut at = POINT::default();
        // SAFETY: `at` is a valid out pointer.
        unsafe { GetCursorPos(&raw mut at) };
        if let Some(id) = default_id {
            // SAFETY: the handle is a live menu.
            unsafe { SetMenuDefaultItem(self.0, id, 0) };
        }
        // SAFETY: `hwnd` is the engine's window on this thread.
        unsafe { SetForegroundWindow(hwnd) };
        // SAFETY: the menu and window are live; no exclusion rectangle.
        let chosen = unsafe {
            TrackPopupMenuEx(
                self.0,
                TPM_RETURNCMD | TPM_RIGHTBUTTON,
                at.x,
                at.y,
                hwnd,
                std::ptr::null(),
            )
        };
        // SAFETY: posting a harmless message to our own window.
        unsafe { PostMessageW(hwnd, WM_NULL, 0, 0) };
        u32::try_from(chosen).ok().filter(|id| *id != 0)
    }
}

impl Drop for Menu {
    fn drop(&mut self) {
        // SAFETY: the menu is ours; destroying it destroys its submenus.
        unsafe { DestroyMenu(self.0) };
    }
}

#[cfg(test)]
mod tests {
    use super::{Entry, Icon, Menu, Tray, taskbar_created_message};
    use crate::window::HiddenWindow;

    fn pixels(size: u32) -> Vec<u8> {
        vec![200; (size * size * 4) as usize]
    }

    #[test]
    fn an_icon_is_made_from_pixels_and_a_wrong_size_is_refused() {
        assert!(Icon::from_bgra(16, &pixels(16)).is_ok());
        assert!(Icon::from_bgra(16, &pixels(20)).is_err());
    }

    #[test]
    fn the_menu_is_built_with_submenus_and_counts_its_entries() {
        let entries = vec![
            Entry::Item {
                id: 1,
                label: String::from("Open"),
                default: true,
                checked: false,
                enabled: true,
            },
            Entry::Separator,
            Entry::Submenu {
                label: String::from("Watches"),
                items: vec![Entry::Item {
                    id: 100,
                    label: String::from("Documents"),
                    default: false,
                    checked: true,
                    enabled: true,
                }],
                enabled: true,
            },
            Entry::Item {
                id: 6,
                label: String::from("Quit"),
                default: false,
                checked: false,
                enabled: false,
            },
        ];
        let menu = Menu::build(&entries).unwrap();
        assert_eq!(menu.item_count(), 4);
    }

    #[test]
    fn the_registered_message_is_stable_and_in_the_dynamic_range() {
        let first = taskbar_created_message();
        assert!(first >= 0xC000);
        assert_eq!(first, taskbar_created_message());
    }

    #[test]
    fn the_icon_can_be_added_removed_and_re_added() {
        let class = format!("PeekBuddy.Test.Tray.{}", std::process::id());
        let window = HiddenWindow::create(&class, |_, _| {}).unwrap();
        let tray = Tray::new(window.handle());
        let icon = Icon::from_bgra(16, &pixels(16)).unwrap();
        // Hosted runners show a taskbar; a machine without one makes the add
        // fail, which the engine treats as non-fatal, so there is nothing to
        // assert about the icon then.
        if tray.add(&icon, "PeekBuddy - test").is_err() {
            return;
        }
        assert!(tray.is_present());
        tray.update(&icon, "PeekBuddy - changed").unwrap();
        tray.remove();
        assert!(!tray.is_present(), "a ghost icon is left behind");
        tray.add(&icon, "again").unwrap();
        assert!(tray.is_present());
        tray.remove();
    }
}
