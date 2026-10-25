//! Reading the resources of a built exe (the manifest tests of #59).

use std::os::windows::ffi::OsStrExt as _;
use std::path::Path;

use windows_sys::Win32::Foundation::{FreeLibrary, HMODULE, TRUE};
use windows_sys::Win32::System::LibraryLoader::{
    EnumResourceNamesW, FindResourceW, LOAD_LIBRARY_AS_DATAFILE, LoadLibraryExW, LoadResource,
    LockResource, SizeofResource,
};

/// `RT_MANIFEST` as an integer resource type (`MAKEINTRESOURCE(24)`).
const RT_MANIFEST: *const u16 = 24 as *const u16;

unsafe extern "system" fn collect(
    _module: HMODULE,
    _kind: *const u16,
    name: *const u16,
    param: isize,
) -> i32 {
    // SAFETY: `param` is the `Vec<usize>` passed by `manifests`, alive for the
    // whole enumeration.
    let names = unsafe { &mut *(param as *mut Vec<usize>) };
    names.push(name as usize);
    TRUE
}

/// Returns the XML of every `RT_MANIFEST` resource in the exe or DLL at
/// `path`. A correctly linked exe has exactly one; an exe without a manifest
/// yields an empty list.
///
/// # Errors
///
/// Returns the OS error when the file cannot be loaded as a data file.
pub fn manifests(path: &Path) -> std::io::Result<Vec<String>> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: `wide` is NUL-terminated; a data-file load runs no code.
    let module = unsafe {
        LoadLibraryExW(
            wide.as_ptr(),
            std::ptr::null_mut(),
            LOAD_LIBRARY_AS_DATAFILE,
        )
    };
    if module.is_null() {
        return Err(std::io::Error::last_os_error());
    }
    let mut names: Vec<usize> = Vec::new();
    // SAFETY: `module` is loaded; `collect` only pushes into `names`, which
    // outlives the call. A failure means "no such resource type".
    unsafe {
        EnumResourceNamesW(
            module,
            RT_MANIFEST,
            Some(collect),
            (&raw mut names) as isize,
        )
    };
    let mut found = Vec::new();
    for name in names {
        // SAFETY: `name` is an integer id or a pointer handed out by the
        // enumeration of this still-loaded module.
        let resource = unsafe { FindResourceW(module, name as *const u16, RT_MANIFEST) };
        if resource.is_null() {
            continue;
        }
        // SAFETY: `resource` was found in `module`.
        let size = unsafe { SizeofResource(module, resource) } as usize;
        // SAFETY: as above.
        let loaded = unsafe { LoadResource(module, resource) };
        if loaded.is_null() {
            continue;
        }
        // SAFETY: `loaded` is a resource handle of `module`.
        let data = unsafe { LockResource(loaded) }.cast::<u8>();
        if data.is_null() {
            continue;
        }
        // SAFETY: the resource has `size` readable bytes while `module` stays
        // loaded; the bytes are copied before FreeLibrary.
        let bytes = unsafe { std::slice::from_raw_parts(data, size) };
        found.push(String::from_utf8_lossy(bytes).into_owned());
    }
    // SAFETY: `module` is ours and no pointer into it is used afterwards.
    unsafe { FreeLibrary(module) };
    Ok(found)
}
