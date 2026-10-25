//! Facts about the running Windows that the engine records at startup.

use windows_sys::Wdk::Storage::FileSystem::RtlSetProcessPlaceholderCompatibilityMode;
use windows_sys::Wdk::System::SystemServices::RtlGetVersion;
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

#[cfg(test)]
mod tests {
    use super::{expose_placeholders, os_build};

    #[test]
    fn the_os_build_is_a_windows_10_or_11_version() {
        let build = os_build();
        assert!(build.starts_with("10.0."), "{build}");
    }

    #[test]
    fn placeholder_mode_can_be_set() {
        assert!(expose_placeholders());
    }
}
