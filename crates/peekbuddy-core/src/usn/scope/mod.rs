//! Keeping a journal read subtree-scoped (#47).
//!
//! The journal covers a whole volume; PeekBuddy watches a few directories. A
//! [`Scope`] holds the file reference numbers (FRNs) of the watched root and
//! every directory below it, with each directory's parent and name, and
//! answers one question per record: is it under the root? A record is
//! accepted when its `ParentFileReferenceNumber` is in the set. Memory scales
//! with the number of watched directories, never with the volume.
//!
//! # Keeping the set current
//!
//! Directory records change the set:
//!
//! - `FILE_CREATE` of a directory under a member adds it.
//! - `RENAME_NEW_NAME` of a directory into the tree adds it and asks for a
//!   walk of its subtree ([`Action::Walk`]): its descendants emit no records
//!   when the directory moves, so the scope cannot learn them from the
//!   journal. A rename inside the tree only changes the parent and name.
//! - `RENAME_NEW_NAME` out of the tree, or `FILE_DELETE`, removes the
//!   directory and every member below it.
//!
//! The pre-change path of a record is computed before the record is applied,
//! so the old name of a renamed directory is still in the path of its
//! `RENAME_OLD_NAME` record. FRNs are compared whole, sequence number
//! included, so a reused MFT slot is a different directory.
//!
//! # Names
//!
//! The unprivileged read returns no names (spike 0001). A directory added
//! without a name is reported as [`Action::NeedName`]; the caller resolves it
//! (`OpenFileById`) and calls [`Scope::set_name`]. Until then the path of
//! anything below it is flagged partial.

use std::collections::{HashMap, HashSet};

use super::record::Record;
use crate::event::map::usn_reason as reason;

const BACKSLASH: u16 = 0x5C;
/// Bound on the parent chain walked for one path; a damaged map cannot loop.
const MAX_DEPTH: usize = 4_096;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Dir {
    parent: u128,
    name: Vec<u16>,
}

/// What the caller has to do after a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Nothing.
    None,
    /// A directory moved into the tree: walk its subtree and add what is in
    /// it with [`Scope::insert`].
    Walk(u128),
    /// A directory was added without a name: resolve it and call
    /// [`Scope::set_name`].
    NeedName(u128),
}

/// The answer for one record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    /// The record is under the watched root and becomes an event.
    pub accept: bool,
    /// The path of the record's parent directory relative to the root,
    /// components joined by `\`; empty for the root itself. For a directory
    /// that is renamed or deleted this is the path before the change.
    pub parent_path: Vec<u16>,
    /// Some directory on the chain has no name yet, so `parent_path` is
    /// incomplete.
    pub path_partial: bool,
    /// What to do next.
    pub action: Action,
}

/// [`Verdict`] without the path, which [`Scope::apply_into`] writes to a
/// buffer of the caller's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    /// See [`Verdict::accept`].
    pub accept: bool,
    /// See [`Verdict::path_partial`].
    pub path_partial: bool,
    /// See [`Verdict::action`].
    pub action: Action,
}

impl Decision {
    const fn ignored() -> Self {
        Self {
            accept: false,
            path_partial: false,
            action: Action::None,
        }
    }
}

/// The directories under one watched root.
#[derive(Debug, Clone)]
pub struct Scope {
    root: u128,
    dirs: HashMap<u128, Dir>,
}

impl Scope {
    /// A scope holding just the root directory.
    #[must_use]
    pub fn new(root: u128) -> Self {
        let mut dirs = HashMap::new();
        dirs.insert(
            root,
            Dir {
                parent: 0,
                name: Vec::new(),
            },
        );
        Self { root, dirs }
    }

    /// The root's reference number.
    #[must_use]
    pub const fn root(&self) -> u128 {
        self.root
    }

    /// Directories in the set, the root included.
    #[must_use]
    pub fn len(&self) -> usize {
        self.dirs.len()
    }

    /// Always false: the root is a member.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.dirs.is_empty()
    }

    /// Whether `frn` is a member directory.
    #[must_use]
    pub fn contains(&self, frn: u128) -> bool {
        self.dirs.contains_key(&frn)
    }

    /// Bytes the set holds on the heap, estimated: the table slots plus the
    /// names. For the memory gate; not exact.
    #[must_use]
    pub fn memory_usage(&self) -> usize {
        let slot = size_of::<u128>() + size_of::<Dir>() + 1;
        self.dirs.capacity() * slot
            + self
                .dirs
                .values()
                .map(|dir| dir.name.capacity() * size_of::<u16>())
                .sum::<usize>()
    }

    /// Adds a directory found by a walk (or a resolved name). The root is
    /// never replaced.
    pub fn insert(&mut self, frn: u128, parent: u128, name: &[u16]) {
        if frn == self.root {
            return;
        }
        self.dirs.insert(
            frn,
            Dir {
                parent,
                name: name.to_vec(),
            },
        );
    }

    /// Gives a directory added without a name its name.
    pub fn set_name(&mut self, frn: u128, name: &[u16]) {
        if frn == self.root {
            return;
        }
        if let Some(dir) = self.dirs.get_mut(&frn) {
            name.clone_into(&mut dir.name);
        }
    }

    /// Removes a directory and every member below it; returns how many.
    pub fn remove_subtree(&mut self, frn: u128) -> usize {
        if frn == self.root || !self.dirs.contains_key(&frn) {
            return 0;
        }
        let mut children: HashMap<u128, Vec<u128>> = HashMap::new();
        for (&id, dir) in &self.dirs {
            children.entry(dir.parent).or_default().push(id);
        }
        let mut doomed: HashSet<u128> = HashSet::new();
        let mut stack = vec![frn];
        while let Some(id) = stack.pop() {
            if doomed.insert(id)
                && let Some(below) = children.get(&id)
            {
                stack.extend(below.iter().copied());
            }
        }
        for id in &doomed {
            self.dirs.remove(id);
        }
        doomed.len()
    }

    /// The path of directory `frn` relative to the root, and whether a name
    /// on the way is missing. `None` when the chain does not reach the root
    /// (a member whose parent was never learned).
    /// Writes the path of `frn` relative to the root (components joined by
    /// `\`, empty for the root) into `out`, reusing its capacity. Returns
    /// whether some directory on the chain has no name yet, or `None` when
    /// `frn` is not under the root.
    pub fn path_into(&self, frn: u128, out: &mut Vec<u16>) -> Option<bool> {
        out.clear();
        // First pass: the length. One separator is counted per component and
        // the one before the first is taken off below.
        let mut len = 0_usize;
        let mut partial = false;
        let mut at = frn;
        let mut depth = 0_usize;
        while at != self.root {
            if depth >= MAX_DEPTH {
                return None;
            }
            let dir = self.dirs.get(&at)?;
            partial |= dir.name.is_empty();
            len += dir.name.len() + 1;
            at = dir.parent;
            depth += 1;
        }
        let total = len.saturating_sub(1);
        out.resize(total, 0);
        // Second pass: fill from the end, leaf first.
        let mut end = total;
        at = frn;
        while at != self.root {
            let dir = self.dirs.get(&at)?;
            let start = end.saturating_sub(dir.name.len());
            if let Some(slot) = out.get_mut(start..end) {
                slot.copy_from_slice(&dir.name);
            }
            end = start;
            if end > 0 {
                end -= 1;
                if let Some(slot) = out.get_mut(end) {
                    *slot = BACKSLASH;
                }
            }
            at = dir.parent;
        }
        Some(partial)
    }

    /// Decides one record and applies its effect on the set. The path of the
    /// record's parent goes into `path` (see [`Scope::apply_into`]).
    pub fn apply(&mut self, rec: &Record<'_>) -> Verdict {
        let mut parent_path = Vec::new();
        let decision = self.apply_into(rec, &mut parent_path);
        Verdict {
            accept: decision.accept,
            parent_path,
            path_partial: decision.path_partial,
            action: decision.action,
        }
    }

    /// Like [`Scope::apply`], with the parent path written into `path`
    /// instead of a new vector: a file record allocates nothing once `path`
    /// has grown to the longest path seen.
    pub fn apply_into(&mut self, rec: &Record<'_>, path: &mut Vec<u16>) -> Decision {
        let is_dir = rec.is_dir();
        let parent_member = self.contains(rec.parent);
        let frn_member = is_dir && self.contains(rec.frn);

        // The path before the change: a member directory's own parent chain
        // while the map still has its old place, else the record's parent.
        let before = if frn_member && !parent_member {
            self.dirs.get(&rec.frn).map(|dir| dir.parent)
        } else {
            Some(rec.parent)
        };
        let path_partial =
            if let Some(partial) = before.and_then(|parent| self.path_into(parent, path)) {
                partial
            } else {
                path.clear();
                true
            };

        let mut decision = Decision {
            accept: parent_member || frn_member,
            path_partial,
            action: Action::None,
        };
        if !is_dir {
            if !parent_member {
                path.clear();
                return Decision::ignored();
            }
            return decision;
        }

        let name = rec.name_units();
        if rec.reason & reason::FILE_DELETE != 0 {
            self.remove_subtree(rec.frn);
        } else if rec.reason & reason::FILE_CREATE != 0 && parent_member {
            self.insert(rec.frn, rec.parent, &name);
            decision.action = needs(&name, rec.frn);
        } else if rec.reason & reason::RENAME_NEW_NAME != 0 {
            if parent_member {
                let known = self.contains(rec.frn);
                self.insert(rec.frn, rec.parent, &name);
                decision.action = if known {
                    needs(&name, rec.frn)
                } else {
                    Action::Walk(rec.frn)
                };
            } else if frn_member {
                // Moved out of the tree: the new-name record is outside.
                self.remove_subtree(rec.frn);
                decision.accept = false;
            }
        }
        decision
    }
}

/// `NeedName` when the name is empty.
fn needs(name: &[u16], frn: u128) -> Action {
    if name.is_empty() {
        Action::NeedName(frn)
    } else {
        Action::None
    }
}

#[cfg(test)]
mod tests;
