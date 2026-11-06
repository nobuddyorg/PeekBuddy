//! Filling a [`Scope`] from a directory walk (#212).
//!
//! The scope of a USN read starts from the directories that exist now, and a
//! directory that moves into the tree brings its subtree with it without a
//! record per descendant. Both are the same walk: list a directory, add the
//! directories in it, go on below them. It reads through the reconcile
//! walker's [`DirSource`], so one lister serves both.
//!
//! Reparse points are never entered (junctions, symlinks, mount points,
//! cloud placeholders that are directories): a watch does not follow a link
//! out of its root.

use super::scope::Scope;
use crate::reconcile::{DirSource, ListError, Listing};

const BACKSLASH: u16 = 0x5C;

/// What a walk found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Walked {
    /// Directories added to the scope.
    pub added: usize,
    /// Directories that could not be listed below the first one (their
    /// subdirectories are not in the scope).
    pub skipped: usize,
}

/// Adds every directory below `dir_frn` to `scope`. `rel_path` is the path of
/// that directory relative to the root of the source (empty for the root).
///
/// # Errors
///
/// Why the first directory could not be listed; nothing was added then.
pub fn walk_into(
    source: &mut impl DirSource,
    scope: &mut Scope,
    dir_frn: u128,
    rel_path: &[u16],
) -> Result<Walked, ListError> {
    let mut walked = Walked::default();
    let mut listing = Listing::new();
    let mut pending: Vec<(u128, Vec<u16>)> = vec![(dir_frn, rel_path.to_vec())];
    let mut first = true;
    while let Some((frn, path)) = pending.pop() {
        listing.clear();
        match source.list(&path, &mut listing) {
            Ok(()) => {}
            Err(error) if first => return Err(error),
            Err(_) => {
                walked.skipped += 1;
                continue;
            }
        }
        first = false;
        for item in listing.items() {
            if !item.meta.is_dir() || item.meta.is_reparse() {
                continue;
            }
            let name = listing.name(item);
            scope.insert(item.meta.file_id, frn, name);
            walked.added += 1;
            let mut below = Vec::with_capacity(path.len() + 1 + name.len());
            below.extend_from_slice(&path);
            if !path.is_empty() {
                below.push(BACKSLASH);
            }
            below.extend_from_slice(name);
            pending.push((item.meta.file_id, below));
        }
    }
    Ok(walked)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::reconcile::Meta;
    use crate::reconcile::dirinfo::{ATTR_DIRECTORY, ATTR_REPARSE_POINT};

    fn units(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    fn meta(file_id: u128, attributes: u32) -> Meta {
        Meta {
            file_id,
            attributes,
            reparse_tag: 0,
            size: 0,
            write_time: 0,
            change_time: 0,
        }
    }

    /// Directories by relative path: `(name, id, attributes)` entries.
    #[derive(Default)]
    struct Tree {
        dirs: HashMap<String, Vec<(&'static str, u128, u32)>>,
        denied: Vec<String>,
        asked: Vec<String>,
    }

    impl DirSource for Tree {
        fn list(&mut self, path: &[u16], out: &mut Listing) -> Result<(), ListError> {
            let key = String::from_utf16_lossy(path);
            self.asked.push(key.clone());
            if self.denied.contains(&key) {
                return Err(ListError::AccessDenied);
            }
            let entries = self.dirs.get(&key).ok_or(ListError::Gone)?;
            for (name, id, attributes) in entries {
                out.push(&units(name), meta(*id, *attributes), 0);
            }
            Ok(())
        }

        fn exists(&mut self, path: &[u16]) -> bool {
            self.dirs.contains_key(&String::from_utf16_lossy(path))
        }
    }

    fn tree() -> Tree {
        let mut tree = Tree::default();
        tree.dirs.insert(
            String::new(),
            vec![
                ("a", 11, ATTR_DIRECTORY),
                ("file.txt", 12, 0x20),
                ("link", 13, ATTR_DIRECTORY | ATTR_REPARSE_POINT),
            ],
        );
        tree.dirs
            .insert("a".into(), vec![("b", 21, ATTR_DIRECTORY), ("f", 22, 0x20)]);
        tree.dirs
            .insert(r"a\b".into(), vec![("c", 31, ATTR_DIRECTORY)]);
        tree.dirs.insert(r"a\b\c".into(), Vec::new());
        tree
    }

    #[test]
    fn every_directory_is_added_with_its_parent_and_name() {
        let mut scope = Scope::new(1);
        let walked = walk_into(&mut tree(), &mut scope, 1, &[]).unwrap();
        assert_eq!(
            walked,
            Walked {
                added: 3,
                skipped: 0
            }
        );
        let mut path = Vec::new();
        assert_eq!(scope.path_into(31, &mut path), Some(false));
        assert_eq!(String::from_utf16_lossy(&path), r"a\b\c");
        assert!(!scope.contains(12), "files are not in the scope");
    }

    #[test]
    fn a_link_is_not_entered_and_not_added() {
        let mut tree = tree();
        let mut scope = Scope::new(1);
        walk_into(&mut tree, &mut scope, 1, &[]).unwrap();
        assert!(!scope.contains(13));
        assert!(!tree.asked.contains(&"link".to_owned()));
    }

    #[test]
    fn a_subtree_is_walked_from_its_own_path() {
        let mut tree = tree();
        let mut scope = Scope::new(1);
        scope.insert(11, 1, &units("a"));
        let walked = walk_into(&mut tree, &mut scope, 11, &units("a")).unwrap();
        assert_eq!(walked.added, 2);
        assert!(scope.contains(21) && scope.contains(31));
        // Only the subtree was listed.
        assert!(tree.asked.iter().all(|p| p == "a" || p.starts_with(r"a\")));
    }

    #[test]
    fn a_denied_directory_below_the_first_is_skipped_and_counted() {
        let mut tree = tree();
        tree.denied.push(r"a\b".into());
        let mut scope = Scope::new(1);
        let walked = walk_into(&mut tree, &mut scope, 1, &[]).unwrap();
        assert_eq!(
            walked,
            Walked {
                added: 2,
                skipped: 1
            }
        );
        assert!(scope.contains(21), "the directory itself is a member");
        assert!(!scope.contains(31));
    }

    #[test]
    fn the_first_directory_failing_is_an_error_and_adds_nothing() {
        let mut tree = tree();
        tree.denied.push(String::new());
        let mut scope = Scope::new(1);
        assert_eq!(
            walk_into(&mut tree, &mut scope, 1, &[]),
            Err(ListError::AccessDenied)
        );
        assert_eq!(scope.len(), 1);
    }
}
