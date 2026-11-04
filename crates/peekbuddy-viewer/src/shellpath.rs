//! Paths for the shell (#69).
//!
//! The log stores paths as Windows reports them, and watched roots may be
//! stored with a `\\?\` prefix. Explorer and the shell APIs do not take that
//! prefix, so it is converted: `\\?\C:\x` becomes `C:\x` and
//! `\\?\UNC\server\share\x` becomes `\\server\share\x`. A deleted file cannot
//! be shown, so the nearest ancestor that still exists is opened instead.
//!
//! Nothing here, or in the actions that use it, opens, reads or executes a
//! file: only the folder is opened and the item selected, so a cloud
//! placeholder is never hydrated and a hostile file never runs.

use std::path::{Path, PathBuf};

/// `path` without the extended-length prefix.
#[must_use]
pub fn plain(path: &str) -> String {
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    if let Some(rest) = path.strip_prefix(r"\\?\") {
        return rest.to_owned();
    }
    path.to_owned()
}

/// The folder part of a path: everything before the last separator.
#[must_use]
pub fn parent(path: &str) -> Option<&str> {
    let at = path.rfind(['\\', '/'])?;
    path.get(..at).filter(|folder| !folder.is_empty())
}

/// The path itself if `exists` says it does, else the nearest ancestor that
/// does. `None` when not even the root exists.
pub fn nearest_existing(path: &str, exists: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    let mut current = path;
    // Every step removes at least one character, so the length of the path
    // bounds the walk, whatever `parent` answers.
    for _ in 0..=path.len() {
        if exists(Path::new(current)) {
            return Some(PathBuf::from(current));
        }
        // `C:` alone is a drive-relative path; `C:\` is the root.
        let next = parent(current)?;
        current = if next.len() == 2 && next.ends_with(':') {
            // Keep the separator of a drive root.
            path.get(..3).unwrap_or(next)
        } else {
            next
        };
        if current.len() <= 3 && current.ends_with(['\\', '/']) {
            return exists(Path::new(current)).then(|| PathBuf::from(current));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{nearest_existing, parent, plain};

    #[test]
    fn the_extended_prefix_is_removed_for_the_shell() {
        assert_eq!(plain(r"\\?\C:\work\a.txt"), r"C:\work\a.txt");
        assert_eq!(
            plain(r"\\?\UNC\server\share\dir\a.txt"),
            r"\\server\share\dir\a.txt"
        );
        assert_eq!(plain(r"C:\already\plain"), r"C:\already\plain");
        assert_eq!(plain(r"\\server\share"), r"\\server\share");
        assert_eq!(plain(""), "");
    }

    #[test]
    fn long_paths_survive_the_conversion() {
        let deep = format!(r"\\?\C:\{}", "very-long-folder-name\\".repeat(20));
        assert!(plain(&deep).len() > 260);
        assert!(plain(&deep).starts_with(r"C:\very"));
    }

    #[test]
    fn the_parent_is_everything_before_the_last_separator() {
        assert_eq!(parent(r"C:\work\a.txt"), Some(r"C:\work"));
        assert_eq!(parent(r"C:\work"), Some("C:"));
        assert_eq!(parent("a.txt"), None);
        assert_eq!(parent(r"\a"), None);
    }

    #[test]
    fn a_deleted_file_opens_the_nearest_folder_that_exists() {
        let exists = |path: &Path| path == Path::new(r"C:\work");
        assert_eq!(
            nearest_existing(r"C:\work\gone\deeper\a.txt", exists),
            Some(r"C:\work".into())
        );
        assert_eq!(
            nearest_existing(r"C:\work", exists),
            Some(r"C:\work".into())
        );
        // Nothing exists, not even the drive.
        assert_eq!(nearest_existing(r"D:\x\y", |_| false), None);
        // Only the drive root does.
        let root = |path: &Path| path == Path::new(r"D:\");
        assert_eq!(nearest_existing(r"D:\x\y", root), Some(r"D:\".into()));
    }

    #[test]
    fn a_doubled_separator_does_not_end_the_walk() {
        // Compared as text: `C:\work\` is not `C:\work` here, so the walk has to
        // go on past the empty component.
        let exact = |path: &Path| path.to_str() == Some(r"C:\work");
        assert_eq!(
            nearest_existing(r"C:\work\\gone", exact),
            Some(r"C:\work".into())
        );
    }

    #[test]
    fn a_two_letter_folder_is_not_taken_for_a_drive() {
        // `ab` has two characters but is no drive (`C:`), so the walk stops
        // there and does not jump to the first three characters of the path.
        let exact = |path: &Path| path.to_str() == Some("ab");
        assert_eq!(nearest_existing(r"ab\cd\e.txt", exact), Some("ab".into()));
    }

    #[test]
    fn a_unc_share_is_tried_up_to_its_server() {
        let share = |path: &Path| path == Path::new(r"\\server\share");
        assert_eq!(
            nearest_existing(r"\\server\share\gone\a.txt", share),
            Some(r"\\server\share".into())
        );
    }
}
