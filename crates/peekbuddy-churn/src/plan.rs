//! Deterministic operation streams.
//!
//! Each thread works in its own subdirectory `t<n>` and keeps a model of
//! that tree, so every planned operation is valid when run in order. The
//! seed fixes each thread's sequence; the ledger, not the seed, is the
//! ground truth for how threads interleave.

use crate::ops::{Op, OpKind, WidePath};
use crate::profile::{Names, Profile};
use crate::rng::Rng;

/// `\` in UTF-16.
const SEP: u16 = b'\\' as u16;

/// Initial tree per thread: `depth` levels of `width` directories, `files`
/// files in each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shape {
    /// Directory levels below the thread root.
    pub depth: u32,
    /// Directories per level and parent.
    pub width: u32,
    /// Files per directory.
    pub files: u32,
}

impl Default for Shape {
    fn default() -> Self {
        Self {
            depth: 2,
            width: 3,
            files: 4,
        }
    }
}

/// Joins `parent` and `name` with `\`.
#[must_use]
pub fn join(parent: &[u16], name: &[u16]) -> WidePath {
    let mut path = parent.to_vec();
    if !path.is_empty() {
        path.push(SEP);
    }
    path.extend_from_slice(name);
    path
}

/// `true` when `path` is `dir` or below it.
#[must_use]
pub fn is_within(path: &[u16], dir: &[u16]) -> bool {
    dir.is_empty() || path == dir || (path.starts_with(dir) && path.get(dir.len()) == Some(&SEP))
}

fn parent(path: &[u16]) -> &[u16] {
    match path.iter().rposition(|&unit| unit == SEP) {
        Some(at) => path.get(..at).unwrap_or_default(),
        None => &[],
    }
}

fn file_name(path: &[u16]) -> &[u16] {
    match path.iter().rposition(|&unit| unit == SEP) {
        Some(at) => path.get(at + 1..).unwrap_or_default(),
        None => path,
    }
}

/// Swaps the case of every ASCII letter in the last component.
fn case_swapped(path: &[u16]) -> WidePath {
    let mut name: WidePath = file_name(path).to_vec();
    for unit in &mut name {
        if let Ok(byte) = u8::try_from(*unit) {
            if byte.is_ascii_lowercase() {
                *unit = u16::from(byte.to_ascii_uppercase());
            } else if byte.is_ascii_uppercase() {
                *unit = u16::from(byte.to_ascii_lowercase());
            }
        }
    }
    join(parent(path), &name)
}

/// Plans the operations of one thread.
#[derive(Debug, Clone)]
pub struct Planner {
    rng: Rng,
    profile: &'static Profile,
    total_weight: u64,
    root: WidePath,
    dirs: Vec<WidePath>,
    files: Vec<WidePath>,
    links: Vec<WidePath>,
    junctions: Vec<WidePath>,
    next_name: u64,
}

impl Planner {
    /// A planner for thread `thread` of a run seeded with `seed`.
    #[must_use]
    pub fn new(seed: u64, thread: u32, profile: &'static Profile) -> Self {
        let root: WidePath = format!("t{thread}").encode_utf16().collect();
        Self {
            rng: Rng::new(seed, u64::from(thread)),
            profile,
            total_weight: profile.mix.iter().map(|(_, w)| u64::from(*w)).sum(),
            root,
            dirs: Vec::new(),
            files: Vec::new(),
            links: Vec::new(),
            junctions: Vec::new(),
            next_name: 0,
        }
    }

    /// The operations that build the initial tree: the thread root, then
    /// `shape`. Run before [`Planner::next_op`].
    pub fn setup(&mut self, shape: Shape) -> Vec<Op> {
        let mut ops = vec![self.create_dir_at(self.root.clone())];
        let mut level = vec![self.root.clone()];
        for _ in 0..shape.depth {
            let mut next = Vec::new();
            for dir in &level {
                for _ in 0..shape.width {
                    let name = self.ascii_name("d");
                    let path = join(dir, &name);
                    ops.push(self.create_dir_at(path.clone()));
                    next.push(path);
                }
            }
            level = next;
        }
        let dirs = self.dirs.clone();
        for dir in &dirs {
            for _ in 0..shape.files {
                let name = self.ascii_name("f");
                ops.push(self.create_file_at(join(dir, &name)));
            }
        }
        ops
    }

    /// The next operation of the weighted mix. A class that cannot run on
    /// the current tree (nothing to rename, say) becomes a file create.
    pub fn next_op(&mut self) -> Op {
        let kind = self.pick_kind();
        self.plan(kind).unwrap_or_else(|| {
            let dir = self.pick_dir();
            let name = self.name("f");
            self.create_file_at(join(&dir, &name))
        })
    }

    fn pick_kind(&mut self) -> OpKind {
        let mut ticket = self.rng.below(self.total_weight);
        for (kind, weight) in self.profile.mix {
            let weight = u64::from(*weight);
            if ticket < weight {
                return *kind;
            }
            ticket -= weight;
        }
        OpKind::CreateFile
    }

    fn plan(&mut self, kind: OpKind) -> Option<Op> {
        match kind {
            OpKind::CreateFile => {
                let dir = self.pick_dir();
                let name = self.name("f");
                Some(self.create_file_at(join(&dir, &name)))
            }
            OpKind::CreateDir => {
                let dir = self.pick_dir();
                let name = self.name("d");
                Some(self.create_dir_at(join(&dir, &name)))
            }
            OpKind::Write
            | OpKind::Append
            | OpKind::Truncate
            | OpKind::SetSize
            | OpKind::SetAttributes
            | OpKind::SetTimes => {
                let file = self.pick(&self.files.clone())?;
                Some(single(kind, file))
            }
            OpKind::Delete => {
                let index = self.pick_index(self.files.len())?;
                let file = self.files.swap_remove(index);
                Some(single(kind, file))
            }
            OpKind::Rename => {
                let index = self.pick_index(self.files.len())?;
                let old = self.files.get(index)?.clone();
                let name = self.name("r");
                let new = join(parent(&old), &name);
                self.files.get_mut(index)?.clone_from(&new);
                Some(pair(kind, old, new))
            }
            OpKind::CaseRename => {
                let index = self.pick_index(self.files.len())?;
                let old = self.files.get(index)?.clone();
                let new = case_swapped(&old);
                if new == old {
                    return None;
                }
                self.files.get_mut(index)?.clone_from(&new);
                Some(pair(kind, old, new))
            }
            OpKind::RenameChain => {
                let index = self.pick_index(self.files.len())?;
                let a = self.files.get(index)?.clone();
                let b = join(parent(&a), &self.name("b"));
                let c = join(parent(&a), &self.name("c"));
                self.files.get_mut(index)?.clone_from(&c);
                Some(Op {
                    kind,
                    paths: vec![a, b, c],
                    removed_below: Vec::new(),
                })
            }
            OpKind::Move => {
                let index = self.pick_index(self.files.len())?;
                let old = self.files.get(index)?.clone();
                let from = parent(&old).to_vec();
                let targets: Vec<WidePath> =
                    self.dirs.iter().filter(|d| **d != from).cloned().collect();
                let dir = self.pick(&targets)?;
                let new = join(&dir, file_name(&old));
                self.files.get_mut(index)?.clone_from(&new);
                Some(pair(kind, old, new))
            }
            OpKind::DeleteTree => {
                let candidates: Vec<WidePath> = self
                    .dirs
                    .iter()
                    .filter(|d| **d != self.root)
                    .cloned()
                    .collect();
                let dir = self.pick(&candidates)?;
                Some(self.delete_tree(dir))
            }
            OpKind::HardLink => {
                let file = self.pick(&self.files.clone())?;
                let link = join(parent(&file), &self.name("l"));
                self.links.push(link.clone());
                Some(pair(kind, file, link))
            }
            OpKind::DeleteLink => {
                let index = self.pick_index(self.links.len())?;
                let link = self.links.swap_remove(index);
                Some(single(kind, link))
            }
            OpKind::Junction => {
                let target = self.pick(&self.dirs.clone())?;
                let dir = self.pick_dir();
                let junction = join(&dir, &self.name("j"));
                self.junctions.push(junction.clone());
                Some(pair(kind, junction, target))
            }
        }
    }

    fn delete_tree(&mut self, dir: WidePath) -> Op {
        let mut below: Vec<WidePath> = Vec::new();
        for list in [&mut self.files, &mut self.links, &mut self.junctions] {
            below.extend(list.iter().filter(|p| is_within(p, &dir)).cloned());
            list.retain(|p| !is_within(p, &dir));
        }
        let sub_dirs: Vec<WidePath> = self
            .dirs
            .iter()
            .filter(|d| **d != dir && is_within(d, &dir))
            .cloned()
            .collect();
        below.extend(sub_dirs);
        self.dirs.retain(|d| !is_within(d, &dir));
        // Deepest first: the order a recursive delete removes them in.
        below.sort_by(|a, b| {
            let depth = |p: &WidePath| p.iter().filter(|&&u| u == SEP).count();
            depth(b).cmp(&depth(a)).then_with(|| a.cmp(b))
        });
        Op {
            kind: OpKind::DeleteTree,
            paths: vec![dir],
            removed_below: below,
        }
    }

    fn create_dir_at(&mut self, path: WidePath) -> Op {
        self.dirs.push(path.clone());
        single(OpKind::CreateDir, path)
    }

    fn create_file_at(&mut self, path: WidePath) -> Op {
        self.files.push(path.clone());
        single(OpKind::CreateFile, path)
    }

    fn pick_dir(&mut self) -> WidePath {
        self.pick(&self.dirs.clone())
            .unwrap_or_else(|| self.root.clone())
    }

    fn pick_index(&mut self, len: usize) -> Option<usize> {
        (len > 0).then(|| self.rng.index(len))
    }

    fn pick(&mut self, list: &[WidePath]) -> Option<WidePath> {
        let index = self.pick_index(list.len())?;
        list.get(index).cloned()
    }

    fn ascii_name(&mut self, prefix: &str) -> WidePath {
        self.next_name += 1;
        format!("{prefix}{}", self.next_name)
            .encode_utf16()
            .collect()
    }

    /// A unique name. Every style ends in an ASCII prefix and counter, so a
    /// case-only rename always changes something.
    fn name(&mut self, prefix: &str) -> WidePath {
        let mut name: WidePath = match self.profile.names {
            Names::Ascii => Vec::new(),
            Names::Exotic => match self.rng.below(6) {
                0 => Vec::new(),
                1 => "文件".encode_utf16().collect(),
                2 => "e\u{301}a\u{308}".encode_utf16().collect(),
                3 => "\u{1F600}".encode_utf16().collect(),
                // A lone high surrogate: valid on NTFS, not valid Unicode.
                4 => vec![0xD800],
                // Long enough that a few levels pass MAX_PATH (260).
                _ => vec![u16::from(b'L'); 120],
            },
        };
        name.extend(self.ascii_name(prefix));
        name
    }
}

fn single(kind: OpKind, path: WidePath) -> Op {
    Op {
        kind,
        paths: vec![path],
        removed_below: Vec::new(),
    }
}

fn pair(kind: OpKind, first: WidePath, second: WidePath) -> Op {
    Op {
        kind,
        paths: vec![first, second],
        removed_below: Vec::new(),
    }
}

#[cfg(test)]
#[expect(
    clippy::indexing_slicing,
    reason = "tests index plans of known shape; a panic is a test failure"
)]
mod tests {
    use super::{Planner, Shape, case_swapped, is_within, join};
    use crate::ops::{OpKind, WidePath};
    use crate::profile::profile;

    fn w(text: &str) -> WidePath {
        text.encode_utf16().collect()
    }

    fn plan(seed: u64, thread: u32, profile_name: &str, ops: usize) -> Vec<crate::ops::Op> {
        let mut planner = Planner::new(seed, thread, profile(profile_name).unwrap());
        let mut out = planner.setup(Shape::default());
        out.extend((0..ops).map(|_| planner.next_op()));
        out
    }

    #[test]
    fn same_seed_same_plan() {
        assert_eq!(plan(1, 0, "edge-mix", 500), plan(1, 0, "edge-mix", 500));
        assert_ne!(plan(1, 0, "edge-mix", 500), plan(2, 0, "edge-mix", 500));
        assert_ne!(plan(1, 0, "edge-mix", 500), plan(1, 1, "edge-mix", 500));
    }

    #[test]
    fn setup_builds_the_shape() {
        let mut planner = Planner::new(1, 3, profile("latency-10k").unwrap());
        let ops = planner.setup(Shape {
            depth: 2,
            width: 2,
            files: 1,
        });
        let dirs = ops.iter().filter(|op| op.kind == OpKind::CreateDir).count();
        let files = ops
            .iter()
            .filter(|op| op.kind == OpKind::CreateFile)
            .count();
        // Root + 2 + 4 directories, one file in each.
        assert_eq!((dirs, files), (7, 7));
        assert_eq!(ops[0].paths, [w("t3")]);
        assert!(ops.iter().all(|op| is_within(&op.paths[0], &w("t3"))));
    }

    #[test]
    fn edge_mix_exercises_every_class() {
        let ops = plan(1, 0, "edge-mix", 2000);
        for kind in OpKind::ALL {
            assert!(ops.iter().any(|op| op.kind == kind), "{kind:?} never ran");
        }
    }

    #[test]
    fn edge_mix_uses_exotic_and_long_names() {
        let ops = plan(1, 0, "edge-mix", 2000);
        let paths: Vec<&WidePath> = ops.iter().flat_map(|op| &op.paths).collect();
        assert!(paths.iter().any(|p| String::from_utf16(p).is_err()));
        assert!(paths.iter().any(|p| p.len() > 260));
        assert!(paths.iter().any(|p| p.contains(&0x6587)));
    }

    #[test]
    fn perf_profiles_use_only_their_mix_and_ascii() {
        let mix = profile("latency-10k").unwrap().mix;
        for op in plan(5, 0, "latency-10k", 2000) {
            assert!(mix.iter().any(|(kind, _)| *kind == op.kind));
            assert!(op.paths.iter().flatten().all(|&unit| unit < 0x80));
        }
    }

    /// Replays a plan on a set model and checks every operation is valid.
    #[test]
    fn plans_are_valid_in_order() {
        use std::collections::BTreeSet;
        for seed in 0..20 {
            let mut existing: BTreeSet<WidePath> = BTreeSet::new();
            for op in plan(seed, 0, "edge-mix", 400) {
                let first = op.paths[0].clone();
                match op.kind {
                    OpKind::CreateFile | OpKind::CreateDir | OpKind::Junction => {
                        assert!(existing.insert(first), "{op:?}");
                    }
                    OpKind::Delete | OpKind::DeleteLink => {
                        assert!(existing.remove(&first), "{op:?}");
                    }
                    OpKind::Rename | OpKind::Move | OpKind::CaseRename => {
                        assert!(existing.remove(&first), "{op:?}");
                        assert!(existing.insert(op.paths[1].clone()), "{op:?}");
                    }
                    OpKind::RenameChain => {
                        assert!(existing.remove(&first), "{op:?}");
                        assert!(existing.insert(op.paths[2].clone()), "{op:?}");
                    }
                    OpKind::HardLink => {
                        assert!(existing.contains(&first), "{op:?}");
                        assert!(existing.insert(op.paths[1].clone()), "{op:?}");
                    }
                    OpKind::DeleteTree => {
                        let below: Vec<WidePath> = existing
                            .iter()
                            .filter(|p| **p != first && is_within(p, &first))
                            .cloned()
                            .collect();
                        let mut planned = op.removed_below.clone();
                        planned.sort();
                        assert_eq!(planned, below, "{op:?}");
                        existing.retain(|p| !is_within(p, &first));
                    }
                    _ => assert!(existing.contains(&first), "{op:?}"),
                }
            }
        }
    }

    #[test]
    fn tree_deletes_list_the_deepest_first() {
        let ops = plan(3, 0, "edge-mix", 2000);
        let tree = ops
            .iter()
            .find(|op| op.kind == OpKind::DeleteTree && op.removed_below.len() > 1)
            .unwrap();
        let depth = |p: &WidePath| p.iter().filter(|&&u| u == u16::from(b'\\')).count();
        let depths: Vec<usize> = tree.removed_below.iter().map(depth).collect();
        assert!(
            depths.windows(2).all(|pair| pair[0] >= pair[1]),
            "{depths:?}"
        );
    }

    #[test]
    fn path_helpers() {
        assert_eq!(join(&w("a"), &w("b")), w("a\\b"));
        assert_eq!(join(&[], &w("b")), w("b"));
        assert!(is_within(&w("a\\b"), &w("a")));
        assert!(is_within(&w("a"), &w("a")));
        assert!(is_within(&w("a"), &[]));
        assert!(!is_within(&w("ab"), &w("a")));
        assert!(!is_within(&w("a"), &w("a\\b")));
        assert_eq!(case_swapped(&w("d1\\aB9")), w("d1\\Ab9"));
        assert_eq!(case_swapped(&w("x")), w("X"));
    }
}
