//! A native folder picker (#68): `IFileOpenDialog` in folder mode.
//!
//! The dialog is a COM object, so this runs on a thread of its own with its
//! own single-threaded apartment: the viewer starts one thread per request
//! and the egui loop keeps painting while the dialog is open. The dialog
//! allows several folders (the keyboard works as in any Explorer dialog),
//! only real file system folders, and returns nothing when it is cancelled.

use std::io;

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoTaskMemFree, CoUninitialize,
};
use windows::Win32::UI::Shell::{
    FOS_ALLOWMULTISELECT, FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST, FOS_PICKFOLDERS, FileOpenDialog,
    IFileOpenDialog, IShellItemArray, SIGDN_FILESYSPATH,
};

/// `HRESULT_FROM_WIN32(ERROR_CANCELLED)`: what `Show` returns when the user
/// closes the dialog.
const CANCELLED: u32 = 0x8007_04C7;

/// `RPC_E_CHANGED_MODE`: the thread already has a COM apartment of another
/// kind. Usable, but not ours to uninitialise.
const CHANGED_MODE: u32 = 0x8001_0106;

/// Ends the apartment this thread created.
struct Apartment(bool);

impl Drop for Apartment {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: balances the successful `CoInitializeEx` of `Apartment::enter`
            // on this same thread; every COM object of the dialog is dropped
            // before the guard.
            unsafe { CoUninitialize() };
        }
    }
}

impl Apartment {
    fn enter() -> io::Result<Self> {
        // SAFETY: plain call with no pointers; the result is checked and the
        // matching `CoUninitialize` runs in `Drop` on the same thread.
        let result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if result.is_ok() {
            Ok(Self(true))
        } else if result.0.cast_unsigned() == CHANGED_MODE {
            Ok(Self(false))
        } else {
            Err(io::Error::other(format!("COM: {result:?}")))
        }
    }
}

fn other(error: &windows::core::Error) -> io::Error {
    io::Error::other(error.to_string())
}

/// Creates the dialog and sets it to pick existing folders, several allowed.
fn create(title: &[u16]) -> io::Result<IFileOpenDialog> {
    // SAFETY: the class id is a constant and the out pointer is the one the
    // generic wrapper makes; the returned interface owns itself.
    let dialog: IFileOpenDialog =
        unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER) }
            .map_err(|e| other(&e))?;
    // SAFETY: a call on the live interface created above.
    let options = unsafe { dialog.GetOptions() }.map_err(|e| other(&e))?;
    let wanted =
        options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST | FOS_ALLOWMULTISELECT;
    // SAFETY: a call on the live interface created above.
    unsafe { dialog.SetOptions(wanted) }.map_err(|e| other(&e))?;
    // SAFETY: `title` is NUL-terminated and outlives the call.
    unsafe { dialog.SetTitle(windows::core::PCWSTR(title.as_ptr())) }.map_err(|e| other(&e))?;
    Ok(dialog)
}

/// The path of the shell item `index` of `results`, freed after copying.
fn path_at(results: &IShellItemArray, index: u32) -> io::Result<Option<String>> {
    // SAFETY: a call on the live array; `index` is below its count.
    let item = unsafe { results.GetItemAt(index) }.map_err(|e| other(&e))?;
    // SAFETY: a call on the live item.
    let name = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) }.map_err(|e| other(&e))?;
    // SAFETY: `name` is the NUL-terminated string the call returned and is
    // not freed yet.
    let text = unsafe { name.to_string() };
    // SAFETY: the shell allocated `name` with the COM task allocator and
    // documents that the caller frees it; it is not used afterwards.
    unsafe { CoTaskMemFree(Some(name.as_ptr().cast_const().cast())) };
    Ok(text.ok())
}

/// Shows the dialog over the window `owner` (an `HWND` as an integer) and
/// returns the folders picked: none when cancelled. Blocks until the dialog
/// closes, so call it from a thread of its own.
///
/// # Errors
///
/// The COM error when the dialog cannot be created or read.
pub fn pick_folders(owner: isize, title: &str) -> io::Result<Vec<String>> {
    let _apartment = Apartment::enter()?;
    let owner = HWND(std::ptr::with_exposed_provenance_mut(owner.cast_unsigned()));
    let title: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
    let dialog = create(&title)?;
    // SAFETY: a call on the live interface; `owner` is a window handle or
    // null, both of which `Show` accepts.
    if let Err(error) = unsafe { dialog.Show(Some(owner)) } {
        return if error.code().0.cast_unsigned() == CANCELLED {
            Ok(Vec::new())
        } else {
            Err(other(&error))
        };
    }
    // SAFETY: a call on the live interface, after a successful `Show`.
    let results = unsafe { dialog.GetResults() }.map_err(|e| other(&e))?;
    // SAFETY: a call on the live array.
    let count = unsafe { results.GetCount() }.map_err(|e| other(&e))?;
    let mut folders = Vec::new();
    for index in 0..count {
        folders.extend(path_at(&results, index)?);
    }
    Ok(folders)
}
