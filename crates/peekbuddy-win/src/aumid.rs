//! The Start-menu shortcut that gives PeekBuddy an identity for toasts (#81).
//!
//! An unpackaged program cannot show a toast under its own name unless a
//! shortcut with `System.AppUserModel.ID` exists; without one the toast is
//! dropped silently. The shortcut is created only when the user enables
//! desktop notifications and removed when they switch it off. It is a
//! COM object, so this runs on a thread with its own apartment.

use std::io;
use std::mem::ManuallyDrop;
use std::path::Path;

use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::System::Com::StructuredStorage::{
    PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize, IPersistFile,
};
use windows::Win32::System::Variant::VT_LPWSTR;
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::Win32::UI::Shell::{IShellLinkW, SHStrDupW, ShellLink};
use windows::core::{Interface as _, PCWSTR, PWSTR};

/// `System.AppUserModel.ID`: {9F4C2855-9F79-4B39-A8D0-E1D42DE1D5F3}, 5.
const PKEY_APP_USER_MODEL_ID: PROPERTYKEY = PROPERTYKEY {
    fmtid: windows::core::GUID::from_u128(0x9F4C2855_9F79_4B39_A8D0_E1D42DE1D5F3),
    pid: 5,
};

/// `RPC_E_CHANGED_MODE`: the thread has an apartment of another kind, which
/// is usable but not ours to end.
const CHANGED_MODE: u32 = 0x8001_0106;

struct Apartment(bool);

impl Drop for Apartment {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: balances the successful `CoInitializeEx` of `enter` on
            // this thread; every COM object is dropped before the guard.
            unsafe { CoUninitialize() };
        }
    }
}

impl Apartment {
    fn enter() -> io::Result<Self> {
        // SAFETY: no pointers; the result is checked and `Drop` balances it.
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

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// A `VT_LPWSTR` value, which is what the shell wants for the id.
fn string_value(text: &str) -> io::Result<PROPVARIANT> {
    let units = wide(text);
    // SAFETY: `units` is NUL-terminated and outlives the call; the copy is
    // allocated with `CoTaskMemAlloc`, which `PropVariantClear` (the drop of
    // `PROPVARIANT`) frees.
    let copy: PWSTR = unsafe { SHStrDupW(PCWSTR(units.as_ptr())) }.map_err(|e| other(&e))?;
    Ok(PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_LPWSTR,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: PROPVARIANT_0_0_0 { pwszVal: copy },
            }),
        },
    })
}

/// Creates (or replaces) the shortcut `link` to `exe` carrying `aumid`.
///
/// # Errors
///
/// The COM error as an `io::Error`, for example when the folder of `link`
/// does not exist.
pub fn create_shortcut(link: &Path, exe: &Path, aumid: &str) -> io::Result<()> {
    let _apartment = Apartment::enter()?;
    let exe = wide(&exe.to_string_lossy());
    let link = wide(&link.to_string_lossy());
    // SAFETY: the class and interface are valid; the result is checked.
    let shell_link: IShellLinkW =
        unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }
            .map_err(|e| other(&e))?;
    // SAFETY: `exe` is a NUL-terminated buffer that outlives the call.
    unsafe { shell_link.SetPath(PCWSTR(exe.as_ptr())) }.map_err(|e| other(&e))?;
    let store: IPropertyStore = shell_link.cast().map_err(|e| other(&e))?;
    let value = string_value(aumid)?;
    let key = PKEY_APP_USER_MODEL_ID;
    // SAFETY: the key and the value are live for the call.
    unsafe { store.SetValue(&raw const key, &raw const value) }.map_err(|e| other(&e))?;
    // SAFETY: plain call on the live store.
    unsafe { store.Commit() }.map_err(|e| other(&e))?;
    let file: IPersistFile = shell_link.cast().map_err(|e| other(&e))?;
    // SAFETY: `link` is a NUL-terminated buffer that outlives the call.
    unsafe { file.Save(PCWSTR(link.as_ptr()), true) }.map_err(|e| other(&e))
}

/// The id stored in the shortcut `link`, or `None` when it has none.
///
/// # Errors
///
/// The COM error as an `io::Error`, for example when the file is missing.
pub fn shortcut_id(link: &Path) -> io::Result<Option<String>> {
    let _apartment = Apartment::enter()?;
    let link = wide(&link.to_string_lossy());
    // SAFETY: the class and interface are valid; the result is checked.
    let shell_link: IShellLinkW =
        unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }
            .map_err(|e| other(&e))?;
    let file: IPersistFile = shell_link.cast().map_err(|e| other(&e))?;
    // SAFETY: `link` is a NUL-terminated buffer that outlives the call;
    // 0 is STGM_READ.
    unsafe { file.Load(PCWSTR(link.as_ptr()), windows::Win32::System::Com::STGM(0)) }
        .map_err(|e| other(&e))?;
    let store: IPropertyStore = shell_link.cast().map_err(|e| other(&e))?;
    let key = PKEY_APP_USER_MODEL_ID;
    // SAFETY: the key is live for the call.
    let value = unsafe { store.GetValue(&raw const key) }.map_err(|e| other(&e))?;
    Ok((!value.is_empty()).then(|| value.to_string()))
}
