//! The replayed log against the disk (#277): the final tree after a gap is the
//! tree on disk.
#![cfg(windows)]

use peekbuddy_core::event::Kind;

use crate::engine::all_rows;
use crate::judge::{Run, relative};

/// Paths below the root, lower case, as the rows of the log say they are:
/// the log replayed. Created and reconciled-created add a path, deleted and
/// renamed-from remove it (and everything below it), renamed-to adds the new
/// path and moves what was below the old one of a directory.
///
/// The two rows of a rename are not always adjacent: with writers in many
/// directories other events come between the old name and the new name, so a
/// renamed-to takes the oldest pending renamed-from of its own directory (a
/// move across directories, which has none there, takes the oldest of all).
pub fn logged_tree(run: &Run) -> std::collections::BTreeSet<String> {
    use std::collections::BTreeSet;

    let root: Vec<u16> = run
        .root
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .encode_utf16()
        .collect();
    let lower = |units: &[u16]| String::from_utf16_lossy(units).to_lowercase();
    let mut tree: BTreeSet<String> = BTreeSet::new();
    let remove_below = |tree: &mut BTreeSet<String>, path: &str| {
        let prefix = format!("{path}\\");
        tree.retain(|entry| entry != path && !entry.starts_with(&prefix));
    };
    let mut pending_old: Vec<String> = Vec::new();
    for row in all_rows(&run.data) {
        let Some(path) = relative(&row.path, &root).map(|units| lower(&units)) else {
            continue;
        };
        match row.kind {
            Kind::Created | Kind::ReconciledCreated => {
                tree.insert(path);
            }
            Kind::Deleted | Kind::ReconciledDeleted => remove_below(&mut tree, &path),
            Kind::RenamedFrom => {
                pending_old.push(path);
            }
            Kind::RenamedTo | Kind::ReconciledRenamed => {
                let old = if row.kind == Kind::RenamedTo {
                    let parent = |entry: &str| entry.rsplit_once('\\').map(|(head, _)| head.to_owned());
                    let own = pending_old
                        .iter()
                        .position(|candidate| parent(candidate) == parent(&path));
                    own.or(if pending_old.is_empty() { None } else { Some(0) })
                        .map(|at| pending_old.remove(at))
                } else {
                    relative(&row.old_path, &root).map(|units| lower(&units))
                };
                if let Some(old) = old {
                    let prefix = format!("{old}\\");
                    let moved: Vec<String> = tree
                        .iter()
                        .filter(|entry| entry.starts_with(&prefix))
                        .cloned()
                        .collect();
                    for entry in moved {
                        tree.remove(&entry);
                        tree.insert(format!("{path}\\{}", &entry[prefix.len()..]));
                    }
                    tree.remove(&old);
                }
                tree.insert(path);
            }
            _ => {}
        }
    }
    tree
}

/// Paths below the root as they are on disk now, lower case; links are
/// entries and are not entered.
pub fn real_tree(root: &std::path::Path) -> std::collections::BTreeSet<String> {
    fn walk(dir: &std::path::Path, prefix: &str, out: &mut std::collections::BTreeSet<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            let path = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}\\{name}")
            };
            let is_dir = entry.file_type().is_ok_and(|kind| kind.is_dir());
            out.insert(path.clone());
            if is_dir {
                walk(&entry.path(), &path, out);
            }
        }
    }
    let mut out = std::collections::BTreeSet::new();
    walk(root, "", &mut out);
    out
}

/// What differs between the replayed log and the disk: up to `limit` lines,
/// `-` for a path only the log has, `+` for one only the disk has. Empty when
/// the trees are equal.
pub fn tree_diff(run: &Run, limit: usize) -> Vec<String> {
    let logged = logged_tree(run);
    let real = real_tree(&run.root);
    let mut diff: Vec<String> = logged
        .difference(&real)
        .map(|path| format!("- {path}"))
        .chain(real.difference(&logged).map(|path| format!("+ {path}")))
        .collect();
    let total = diff.len();
    diff.truncate(limit);
    if total > limit {
        diff.push(format!("... and {} more", total - limit));
    }
    diff
}
