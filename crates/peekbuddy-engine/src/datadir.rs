//! The data directory (F16, ADR 0008).

use std::fs::OpenOptions;
use std::io;
use std::os::windows::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};

use known_folders::{KnownFolder, get_known_folder_path};

use crate::portable::{self, Decision, MARKER, Probe};

/// `FILE_FLAG_DELETE_ON_CLOSE`: the probe file vanishes when it is closed.
const DELETE_ON_CLOSE: u32 = 0x0400_0000;

/// The data directory and the paths inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataDir {
    /// Canonical root. Exposed to the capture layer as the prefix that is
    /// always excluded from every watch.
    pub root: PathBuf,
    /// Day segments (`segments\`).
    pub segments: PathBuf,
    /// Log files and crash logs (`logs\`).
    pub logs: PathBuf,
    /// `settings.toml` (#62).
    pub settings: PathBuf,
    /// How the root was chosen.
    pub decision: Decision,
}

impl DataDir {
    /// Creates `root` with its `segments` and `logs` folders, marks all three
    /// `NOT_CONTENT_INDEXED` before anything else is written, and resolves the
    /// canonical root.
    ///
    /// # Errors
    ///
    /// Returns the I/O error when a folder cannot be created or marked.
    pub fn open(root: &Path, decision: Decision) -> io::Result<Self> {
        std::fs::create_dir_all(root)?;
        peekbuddy_win::attrs::set_not_content_indexed(root)?;
        let root = std::fs::canonicalize(root)?;
        let segments = root.join("segments");
        let logs = root.join("logs");
        for folder in [&segments, &logs] {
            std::fs::create_dir_all(folder)?;
            peekbuddy_win::attrs::set_not_content_indexed(folder)?;
        }
        let settings = root.join("settings.toml");
        Ok(Self {
            root,
            segments,
            logs,
            settings,
            decision,
        })
    }
}

/// Whether a file can be created in `dir`. The file is opened with
/// `FILE_FLAG_DELETE_ON_CLOSE`, so nothing is left behind.
#[must_use]
pub fn is_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".peekbuddy-write-probe-{}", std::process::id()));
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .custom_flags(DELETE_ON_CLOSE)
        .open(probe)
        .is_ok()
}

fn protected_folders() -> Vec<PathBuf> {
    [
        KnownFolder::ProgramFiles,
        KnownFolder::ProgramFilesX86,
        KnownFolder::Desktop,
        KnownFolder::Documents,
        KnownFolder::Downloads,
    ]
    .into_iter()
    .filter_map(get_known_folder_path)
    .filter_map(|folder| std::fs::canonicalize(folder).ok())
    .collect()
}

/// Runs the three portable-mode probes for the exe in `exe_dir`.
#[must_use]
pub fn probe(exe_dir: &Path) -> Probe {
    let canonical = std::fs::canonicalize(exe_dir).unwrap_or_else(|_| exe_dir.to_path_buf());
    Probe {
        marker_present: exe_dir.join(MARKER).is_file(),
        writable: is_writable(exe_dir),
        in_protected_folder: protected_folders()
            .iter()
            .any(|folder| portable::is_under(&canonical, folder)),
    }
}

/// Chooses the data root for the exe at `exe`.
///
/// # Errors
///
/// Returns an error when the exe has no folder or `%LOCALAPPDATA%` cannot be
/// resolved.
pub fn locate(exe: &Path) -> io::Result<(PathBuf, Decision)> {
    let exe_dir = exe
        .parent()
        .ok_or_else(|| io::Error::other("the exe has no parent folder"))?;
    let decision = portable::decide(probe(exe_dir));
    let root = match decision {
        Decision::Portable => exe_dir.join("data"),
        Decision::Installed(_) => get_known_folder_path(KnownFolder::LocalAppData)
            .ok_or_else(|| io::Error::other("LocalAppData is not available"))?
            .join("nobuddy")
            .join("PeekBuddy"),
    };
    Ok((root, decision))
}

#[cfg(test)]
mod tests {
    use super::{DataDir, is_writable, locate, probe};
    use crate::portable::Decision;

    fn temp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("pb-datadir-{name}-{}", std::process::id()))
    }

    #[test]
    fn open_creates_the_layout_and_marks_every_folder() {
        let root = temp("open");
        let data = DataDir::open(&root, Decision::Installed(None)).unwrap();
        for folder in [&data.root, &data.segments, &data.logs] {
            assert!(folder.is_dir(), "{folder:?}");
            assert!(
                peekbuddy_win::attrs::is_not_content_indexed(folder).unwrap(),
                "{folder:?}"
            );
        }
        assert_eq!(data.settings.file_name().unwrap(), "settings.toml");
        // Canonical: the verbatim prefix the capture layer compares against.
        assert!(data.root.to_string_lossy().starts_with(r"\\?\"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn writable_probe_leaves_nothing_behind() {
        let dir = temp("probe");
        std::fs::create_dir_all(&dir).unwrap();
        assert!(is_writable(&dir));
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        std::fs::remove_dir(&dir).unwrap();
        assert!(!is_writable(&dir));
    }

    #[test]
    fn the_probe_reads_the_marker() {
        let dir = temp("marker");
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!probe(&dir).marker_present);
        std::fs::write(dir.join(crate::portable::MARKER), "").unwrap();
        assert!(probe(&dir).marker_present);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn without_a_marker_the_root_is_under_local_app_data() {
        let exe = temp("exe").join("peekbuddy.exe");
        let (root, decision) = locate(&exe).unwrap();
        assert_eq!(decision, Decision::Installed(None));
        assert!(root.ends_with(r"nobuddy\PeekBuddy"), "{root:?}");
    }
}
