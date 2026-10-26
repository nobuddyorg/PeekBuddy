//! Facts about the running Windows that the engine records at startup.

use windows_sys::Wdk::Storage::FileSystem::RtlSetProcessPlaceholderCompatibilityMode;
use windows_sys::Wdk::System::SystemServices::RtlGetVersion;
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RegGetValueW,
};
use windows_sys::Win32::System::SystemInformation::OSVERSIONINFOW;

/// `PHCM_EXPOSE_PLACEHOLDERS`: absent from windows-sys 0.61.2 (#36).
const PHCM_EXPOSE_PLACEHOLDERS: i8 = 2;

/// Makes cloud-file placeholders visible to this process instead of having
/// the system hydrate them behind its back. Called early in engine startup;
/// the policy that uses it is owned by #49.
///
/// Returns whether the mode is now active.
#[must_use]
pub fn expose_placeholders() -> bool {
    // SAFETY: takes a plain integer mode and has no pointer arguments.
    let previous = unsafe { RtlSetProcessPlaceholderCompatibilityMode(PHCM_EXPOSE_PLACEHOLDERS) };
    // The call returns the previous mode, or a negative value on failure.
    previous >= 0
}

/// The OS version as `major.minor.build`, for crash logs and diagnostics.
/// `RtlGetVersion` is used because `GetVersionExW` lies without a manifest
/// entry. Returns `unknown` when the call fails.
#[must_use]
pub fn os_build() -> String {
    let mut info = OSVERSIONINFOW {
        dwOSVersionInfoSize: u32::try_from(size_of::<OSVERSIONINFOW>()).unwrap_or(0),
        ..OSVERSIONINFOW::default()
    };
    // SAFETY: `info` is a valid OSVERSIONINFOW with its size field set.
    let status = unsafe { RtlGetVersion(&raw mut info) };
    if status < 0 {
        return String::from("unknown");
    }
    format!(
        "{}.{}.{}",
        info.dwMajorVersion, info.dwMinorVersion, info.dwBuildNumber
    )
}

/// Reads a `REG_DWORD` below `root` without opening a key handle.
fn registry_dword(root: HKEY, subkey: &str, value: &str) -> Option<u32> {
    let subkey: Vec<u16> = subkey.encode_utf16().chain(Some(0)).collect();
    let value: Vec<u16> = value.encode_utf16().chain(Some(0)).collect();
    let mut data = 0u32;
    let mut size = u32::try_from(size_of::<u32>()).unwrap_or(4);
    // SAFETY: both names are NUL-terminated UTF-16; `data` and `size` are
    // valid out pointers describing a 4-byte buffer.
    let status = unsafe {
        RegGetValueW(
            root,
            subkey.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&raw mut data).cast(),
            &raw mut size,
        )
    };
    (status == 0).then_some(data)
}

/// A `REG_DWORD` below `HKEY_CURRENT_USER`, such as the personalisation
/// switches. `None` when it is not set.
#[must_use]
pub fn current_user_dword(subkey: &str, value: &str) -> Option<u32> {
    registry_dword(HKEY_CURRENT_USER, subkey, value)
}

/// The update build revision (`UBR`) that follows the build number, such as
/// the `5039` in `26100.5039`. `None` when the registry does not say.
#[must_use]
pub fn update_build_revision() -> Option<u32> {
    registry_dword(
        HKEY_LOCAL_MACHINE,
        r"SOFTWARE\Microsoft\Windows NT\CurrentVersion",
        "UBR",
    )
}

/// Whether Defender real-time protection is on, when the registry says so
/// (it does not on machines with another antivirus or with Tamper Protection
/// hiding the value). Results must state this next to their numbers.
#[must_use]
pub fn defender_realtime_enabled() -> Option<bool> {
    registry_dword(
        HKEY_LOCAL_MACHINE,
        r"SOFTWARE\Microsoft\Windows Defender\Real-Time Protection",
        "DisableRealtimeMonitoring",
    )
    .map(|disabled| disabled == 0)
}

#[cfg(test)]
mod tests {
    use super::{expose_placeholders, os_build, update_build_revision};

    #[test]
    fn the_os_build_is_a_windows_10_or_11_version() {
        let build = os_build();
        assert!(build.starts_with("10.0."), "{build}");
    }

    #[test]
    fn the_update_build_revision_is_readable() {
        assert!(update_build_revision().is_some());
    }

    #[test]
    fn placeholder_mode_can_be_set() {
        assert!(expose_placeholders());
    }
}
