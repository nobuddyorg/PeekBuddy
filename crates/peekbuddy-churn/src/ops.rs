//! Operations the generator performs and the raw records each one must
//! produce.

use serde::{Deserialize, Serialize};

/// A path relative to the churn root, as exact UTF-16 (it may hold lone
/// surrogates, F17). Components are separated by `\`.
pub type WidePath = Vec<u16>;

/// Raw record kinds, one per `FILE_ACTION_*` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RawKind {
    /// `FILE_ACTION_ADDED`.
    Added,
    /// `FILE_ACTION_REMOVED`.
    Removed,
    /// `FILE_ACTION_MODIFIED`.
    Modified,
    /// `FILE_ACTION_RENAMED_OLD_NAME`.
    RenamedOld,
    /// `FILE_ACTION_RENAMED_NEW_NAME`.
    RenamedNew,
}

/// Operation classes of the weighted mix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpKind {
    /// Create a new empty file.
    CreateFile,
    /// Overwrite a file's start.
    Write,
    /// Append to a file.
    Append,
    /// Truncate a file to zero.
    Truncate,
    /// Set a file's size (`SetEndOfFile`).
    SetSize,
    /// Rename within the same directory.
    Rename,
    /// Rename a->b->c in one go (two renames).
    RenameChain,
    /// Move to another directory.
    Move,
    /// Delete a file.
    Delete,
    /// Create a directory.
    CreateDir,
    /// Delete a directory and everything below it.
    DeleteTree,
    /// Change attributes (toggle `FILE_ATTRIBUTE_ARCHIVE`).
    SetAttributes,
    /// Change the last-write time.
    SetTimes,
    /// Create a hard link to a file.
    HardLink,
    /// Delete a hard link (an extra name of a file).
    DeleteLink,
    /// Rename that changes only letter case.
    CaseRename,
    /// Create a junction; it is logged as the link object, never followed
    /// (F18).
    Junction,
}

impl OpKind {
    /// Every class, for coverage checks.
    pub const ALL: [Self; 17] = [
        Self::CreateFile,
        Self::Write,
        Self::Append,
        Self::Truncate,
        Self::SetSize,
        Self::Rename,
        Self::RenameChain,
        Self::Move,
        Self::Delete,
        Self::CreateDir,
        Self::DeleteTree,
        Self::SetAttributes,
        Self::SetTimes,
        Self::HardLink,
        Self::DeleteLink,
        Self::CaseRename,
        Self::Junction,
    ];
}

/// One planned operation. `paths` holds the operand paths in order: the
/// target for single-path operations, `[old, new]` for renames and moves,
/// `[a, b, c]` for a rename chain, `[existing, link]` for a hard link,
/// `[junction, target]` for a junction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Op {
    /// Class of the operation.
    pub kind: OpKind,
    /// Operand paths.
    pub paths: Vec<WidePath>,
    /// For a tree delete: every path below the directory, deepest first,
    /// which the delete removes too.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed_below: Vec<WidePath>,
}

/// One record the watcher must log for an operation: a path and a kind.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Expected {
    /// Path the record is for.
    pub path: WidePath,
    /// Kind of the record.
    pub kind: RawKind,
}

fn expect(path: &WidePath, kind: RawKind) -> Expected {
    Expected {
        path: path.clone(),
        kind,
    }
}

impl Op {
    /// The raw records this operation must produce at least once each.
    ///
    /// One logical change can produce several records (a save is 2-4); only
    /// the kinds that every file system and Windows version report are
    /// required, extra records are fine.
    #[must_use]
    pub fn expected(&self) -> Vec<Expected> {
        let first = self.paths.first();
        let second = self.paths.get(1);
        let mut out = Vec::new();
        match self.kind {
            OpKind::CreateFile | OpKind::CreateDir | OpKind::Junction => {
                out.extend(first.map(|p| expect(p, RawKind::Added)));
            }
            OpKind::Write
            | OpKind::Append
            | OpKind::Truncate
            | OpKind::SetSize
            | OpKind::SetAttributes
            | OpKind::SetTimes => {
                out.extend(first.map(|p| expect(p, RawKind::Modified)));
            }
            OpKind::Rename | OpKind::CaseRename => {
                out.extend(first.map(|p| expect(p, RawKind::RenamedOld)));
                out.extend(second.map(|p| expect(p, RawKind::RenamedNew)));
            }
            // A move to another directory is reported as a removal at the old
            // path and an addition at the new one, not as a rename pair: the
            // end-to-end run of #50 on NTFS showed it for every move.
            OpKind::Move => {
                out.extend(first.map(|p| expect(p, RawKind::Removed)));
                out.extend(second.map(|p| expect(p, RawKind::Added)));
            }
            OpKind::RenameChain => {
                for pair in self.paths.windows(2) {
                    if let [old, new] = pair {
                        out.push(expect(old, RawKind::RenamedOld));
                        out.push(expect(new, RawKind::RenamedNew));
                    }
                }
            }
            OpKind::Delete | OpKind::DeleteLink => {
                out.extend(first.map(|p| expect(p, RawKind::Removed)));
            }
            OpKind::DeleteTree => {
                out.extend(
                    self.removed_below
                        .iter()
                        .map(|p| expect(p, RawKind::Removed)),
                );
                out.extend(first.map(|p| expect(p, RawKind::Removed)));
            }
            OpKind::HardLink => {
                out.extend(second.map(|p| expect(p, RawKind::Added)));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::{Expected, Op, OpKind, RawKind, WidePath};

    fn p(text: &str) -> WidePath {
        text.encode_utf16().collect()
    }

    fn op(kind: OpKind, paths: &[&str]) -> Op {
        Op {
            kind,
            paths: paths.iter().map(|text| p(text)).collect(),
            removed_below: Vec::new(),
        }
    }

    fn kinds(op: &Op) -> Vec<(String, RawKind)> {
        op.expected()
            .into_iter()
            .map(|Expected { path, kind }| (String::from_utf16_lossy(&path), kind))
            .collect()
    }

    #[test]
    fn single_path_operations() {
        for kind in [OpKind::CreateFile, OpKind::CreateDir, OpKind::Junction] {
            assert_eq!(
                kinds(&op(kind, &["a", "t"])),
                [("a".into(), RawKind::Added)]
            );
        }
        for kind in [
            OpKind::Write,
            OpKind::Append,
            OpKind::Truncate,
            OpKind::SetSize,
            OpKind::SetAttributes,
            OpKind::SetTimes,
        ] {
            assert_eq!(kinds(&op(kind, &["f"])), [("f".into(), RawKind::Modified)]);
        }
        for kind in [OpKind::Delete, OpKind::DeleteLink] {
            assert_eq!(kinds(&op(kind, &["f"])), [("f".into(), RawKind::Removed)]);
        }
        assert_eq!(
            kinds(&op(OpKind::HardLink, &["f", "l"])),
            [("l".into(), RawKind::Added)]
        );
    }

    #[test]
    fn renames_expect_both_names() {
        for kind in [OpKind::Rename, OpKind::CaseRename] {
            assert_eq!(
                kinds(&op(kind, &["a", "b"])),
                [
                    ("a".into(), RawKind::RenamedOld),
                    ("b".into(), RawKind::RenamedNew)
                ]
            );
        }
        assert_eq!(
            kinds(&op(OpKind::Move, &["a", "d\\b"])),
            [
                ("a".into(), RawKind::Removed),
                ("d\\b".into(), RawKind::Added)
            ]
        );
        assert_eq!(
            kinds(&op(OpKind::RenameChain, &["a", "b", "c"])),
            [
                ("a".into(), RawKind::RenamedOld),
                ("b".into(), RawKind::RenamedNew),
                ("b".into(), RawKind::RenamedOld),
                ("c".into(), RawKind::RenamedNew),
            ]
        );
    }

    #[test]
    fn a_tree_delete_removes_everything_below() {
        let mut tree = op(OpKind::DeleteTree, &["d"]);
        tree.removed_below = vec![p("d\\x\\f"), p("d\\x")];
        assert_eq!(
            kinds(&tree),
            [
                ("d\\x\\f".into(), RawKind::Removed),
                ("d\\x".into(), RawKind::Removed),
                ("d".into(), RawKind::Removed)
            ]
        );
    }

    #[test]
    fn all_lists_every_class_once() {
        let mut all = OpKind::ALL.to_vec();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), OpKind::ALL.len());
    }
}
