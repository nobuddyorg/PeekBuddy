//! Where the data lives: `%LOCALAPPDATA%` or, opt-in, next to the exe (F16,
//! ADR 0008).

use std::path::{Component, Path};

/// Marker file next to the exe that asks for portable mode.
pub const MARKER: &str = "peekbuddy.portable";

/// What the probes found about the exe's folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Probe {
    /// `peekbuddy.portable` sits next to the exe.
    pub marker_present: bool,
    /// A file can be created (and deleted on close) in the exe's folder.
    pub writable: bool,
    /// The exe's folder is under Program Files, Desktop, Documents or
    /// Downloads.
    pub in_protected_folder: bool,
}

/// Why a requested portable mode was not used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fallback {
    /// The exe's folder is under a protected user or system folder.
    ProtectedFolder,
    /// The exe's folder is not writable.
    NotWritable,
}

impl Fallback {
    /// Text for `--diagnostics`.
    #[must_use]
    pub const fn describe(self) -> &'static str {
        match self {
            Self::ProtectedFolder => {
                "portable marker ignored: the exe is under Program Files, Desktop, Documents or Downloads"
            }
            Self::NotWritable => "portable marker ignored: the exe's folder is not writable",
        }
    }
}

/// The data location decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Data in a `data` folder next to the exe.
    Portable,
    /// Data in `%LOCALAPPDATA%\nobuddy\PeekBuddy`; the reason is set when the
    /// user asked for portable mode and did not get it.
    Installed(Option<Fallback>),
}

/// Portable mode needs the marker AND a writable folder AND a folder that is
/// not a protected one. Anything else falls back to `%LOCALAPPDATA%`.
#[must_use]
pub const fn decide(probe: Probe) -> Decision {
    if !probe.marker_present {
        Decision::Installed(None)
    } else if probe.in_protected_folder {
        Decision::Installed(Some(Fallback::ProtectedFolder))
    } else if !probe.writable {
        Decision::Installed(Some(Fallback::NotWritable))
    } else {
        Decision::Portable
    }
}

fn lowered(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(name) => Some(name.to_string_lossy().to_lowercase()),
            Component::Prefix(prefix) => Some(prefix.as_os_str().to_string_lossy().to_lowercase()),
            _ => None,
        })
        .collect()
}

/// Whether `path` is `root` or below it, ignoring case. Both paths must be
/// canonical (same `\\?\` form) for the comparison to mean anything.
#[must_use]
pub fn is_under(path: &Path, root: &Path) -> bool {
    let root = lowered(root);
    let path = lowered(path);
    !root.is_empty() && path.starts_with(&root)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{Decision, Fallback, Probe, decide, is_under};

    fn probe(marker_present: bool, writable: bool, in_protected_folder: bool) -> Probe {
        Probe {
            marker_present,
            writable,
            in_protected_folder,
        }
    }

    #[test]
    fn without_the_marker_the_data_is_installed() {
        assert_eq!(decide(probe(false, true, false)), Decision::Installed(None));
        // Nothing else matters without the marker.
        assert_eq!(decide(probe(false, false, true)), Decision::Installed(None));
    }

    #[test]
    fn a_marker_in_a_writable_ordinary_folder_is_portable() {
        assert_eq!(decide(probe(true, true, false)), Decision::Portable);
    }

    #[test]
    fn an_unwritable_folder_falls_back_and_says_why() {
        assert_eq!(
            decide(probe(true, false, false)),
            Decision::Installed(Some(Fallback::NotWritable))
        );
    }

    #[test]
    fn a_protected_folder_falls_back_even_if_writable() {
        assert_eq!(
            decide(probe(true, true, true)),
            Decision::Installed(Some(Fallback::ProtectedFolder))
        );
        // The protected folder is named first when both apply.
        assert_eq!(
            decide(probe(true, false, true)),
            Decision::Installed(Some(Fallback::ProtectedFolder))
        );
    }

    #[test]
    fn under_ignores_case_and_requires_whole_components() {
        let docs = Path::new(r"C:\Users\Me\Documents");
        assert!(is_under(Path::new(r"c:\users\me\DOCUMENTS\tools\pb"), docs));
        assert!(is_under(docs, docs));
        assert!(!is_under(Path::new(r"C:\Users\Me\Documents2\pb"), docs));
        assert!(!is_under(Path::new(r"D:\Users\Me\Documents\pb"), docs));
        assert!(!is_under(docs, Path::new("")));
    }
}
