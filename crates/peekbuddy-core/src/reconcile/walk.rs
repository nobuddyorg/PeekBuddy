//! The walk itself (#46); see the [module docs](super) for the rules.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

use super::{DirSource, Item, ListError, Listing, Meta, RowOut, Snapshot};
use crate::event::{Event, Flags, GapReason, Kind, NameKind, NameRef, RootIdx, Source, VolIdx};

/// The separator of the relative paths in rows and listings.
const SEPARATOR: u16 = b'\\' as u16;

/// What a walk needs to know.
#[derive(Debug, Clone, Copy)]
pub struct Config<'a> {
    /// The root being reconciled.
    pub root: RootIdx,
    /// Its volume.
    pub vol: VolIdx,
    /// The gap that asked for the walk, if any.
    pub reason: Option<GapReason>,
    /// Checked once per directory; the walk stops and rolls back when set.
    pub cancel: &'a AtomicBool,
    /// Wall time (FILETIME ticks) and a monotonic count, now.
    pub clock: fn() -> (u64, u64),
}

/// What a walk found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Summary {
    /// Entries seen on disk.
    pub scanned: u64,
    /// Files among them.
    pub files: u64,
    /// Directories among them.
    pub dirs: u64,
    /// Difference rows written (not counting the state rows).
    pub changed: u64,
    /// Directories that could not be listed.
    pub denied: u64,
    /// This was the first walk of the root: only the snapshot was written.
    pub baseline: bool,
    /// The walk was cancelled; the snapshot is as it was before.
    pub cancelled: bool,
    /// The rows could not be made durable; the snapshot is as it was before.
    pub not_durable: bool,
}

/// A directory still to list.
struct Frame {
    /// The snapshot id of the directory (0 for the root).
    id: i64,
    /// The path relative to the root.
    path: Vec<u16>,
}

/// An entry that is only on one side, waiting for a rename partner.
struct Pending {
    path: Vec<u16>,
    meta: Meta,
    /// The snapshot id (0 for an entry that is only on disk).
    id: i64,
}

fn join(dir: &[u16], name: &[u16]) -> Vec<u16> {
    let mut path = Vec::with_capacity(dir.len() + 1 + name.len());
    path.extend_from_slice(dir);
    if !dir.is_empty() {
        path.push(SEPARATOR);
    }
    path.extend_from_slice(name);
    path
}

struct Walk<'a, R> {
    cfg: &'a Config<'a>,
    out: &'a mut R,
    summary: Summary,
}

impl<R: RowOut> Walk<'_, R> {
    fn event(&self, kind: Kind, meta: Option<&Meta>, name_len: usize) -> Event {
        let (ts, qpc) = (self.cfg.clock)();
        let meta = meta.copied().unwrap_or_default();
        Event {
            ts,
            qpc,
            src_ts: 0,
            source: Source::Reconcile,
            kind,
            raw_mask: 0,
            source_info: 0,
            is_dir: meta.is_dir(),
            size: meta.size,
            attrs: meta.attributes,
            reparse_tag: meta.reparse_tag,
            file_id: meta.file_id,
            parent_file_id: 0,
            usn: 0,
            vol: self.cfg.vol,
            root: self.cfg.root,
            name: NameRef {
                offset: 0,
                len: u16::try_from(name_len).unwrap_or(0),
            },
            name_kind: NameKind::Unknown,
            flags: Flags::NONE,
        }
    }

    fn row(&mut self, kind: Kind, path: &[u16], meta: &Meta) {
        let event = self.event(kind, Some(meta), path.len());
        self.out.row(&event, path);
        self.summary.changed += 1;
    }

    fn state_row(&mut self, kind: Kind, raw_mask: u32, counts: (u64, u64, u64)) {
        let mut event = self.event(kind, None, 0);
        event.raw_mask = raw_mask;
        event.size = counts.0;
        event.file_id = u128::from(counts.1);
        event.src_ts = counts.2;
        self.out.row(&event, &[]);
    }
}

/// Walks the root and reports what differs from the snapshot.
///
/// # Errors
///
/// The snapshot's storage error; the transaction is rolled back.
pub fn run<S, N, R>(
    cfg: &Config<'_>,
    source: &mut S,
    snapshot: &mut N,
    out: &mut R,
) -> Result<Summary, N::Error>
where
    S: DirSource,
    N: Snapshot,
    R: RowOut,
{
    let root = cfg.root.0;
    let baseline = !snapshot.is_baselined(root)?;
    snapshot.begin()?;
    match walk(cfg, source, snapshot, out, baseline) {
        Ok(summary) => Ok(summary),
        Err(error) => {
            snapshot.rollback();
            Err(error)
        }
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "one pass over the tree; the steps read top to bottom"
)]
fn walk<S, N, R>(
    cfg: &Config<'_>,
    source: &mut S,
    snapshot: &mut N,
    out: &mut R,
    baseline: bool,
) -> Result<Summary, N::Error>
where
    S: DirSource,
    N: Snapshot,
    R: RowOut,
{
    let root = cfg.root.0;
    let started = (cfg.clock)().1;
    let mut walk = Walk {
        cfg,
        out,
        summary: Summary {
            baseline,
            ..Summary::default()
        },
    };
    if !baseline {
        let reason = cfg.reason.map_or(0, |reason| {
            u32::from(Kind::Overflow(reason).to_wire().1) + 1
        });
        if cfg.reason == Some(GapReason::EngineNotRunning) {
            // Nothing watched the tree while the engine was down: the log
            // shows the gap, and the walk that follows closes it. The
            // decoder writes the gap rows of every other reason itself.
            walk.state_row(Kind::Overflow(GapReason::EngineNotRunning), 0, (0, 0, 0));
        }
        walk.state_row(Kind::ReconcileStarted, reason, (0, 0, 0));
    }
    let mut listing = Listing::new();
    let mut known = Listing::new();
    let mut created: Vec<Pending> = Vec::new();
    let mut deleted: Vec<Pending> = Vec::new();
    let mut root_listed = false;
    let mut stack = vec![Frame {
        id: 0,
        path: Vec::new(),
    }];
    while let Some(frame) = stack.pop() {
        if cfg.cancel.load(Ordering::Acquire) {
            walk.summary.cancelled = true;
            break;
        }
        listing.clear();
        let listed = source.list(&frame.path, &mut listing);
        match listed {
            Ok(()) => root_listed |= frame.path.is_empty(),
            Err(ListError::Gone) => listing.clear(),
            Err(ListError::AccessDenied | ListError::Other) => {
                walk.summary.denied += 1;
                if !baseline {
                    let meta = Meta::default();
                    walk.row(Kind::AccessDenied, &frame.path, &meta);
                }
                continue;
            }
        }
        known.clear();
        if !baseline {
            snapshot.children(root, frame.id, &mut known)?;
        }
        listing.sort();
        known.sort();
        walk.summary.scanned += u64::try_from(listing.len()).unwrap_or(u64::MAX);
        let (mut i, mut j) = (0, 0);
        loop {
            let new = listing.items().get(i).copied();
            let old = known.items().get(j).copied();
            let order = match (&new, &old) {
                (None, None) => break,
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (Some(n), Some(o)) => listing.cmp_names(n, &known, o),
            };
            match (order, new, old) {
                (std::cmp::Ordering::Equal, Some(n), Some(o)) => {
                    i += 1;
                    j += 1;
                    let name = listing.name(&n);
                    let path = join(&frame.path, name);
                    if n.meta.is_dir() == o.meta.is_dir() {
                        count(&mut walk.summary, &n);
                        if n.meta.differs_from(&o.meta) {
                            walk.row(Kind::ReconciledModified, &path, &n.meta);
                        }
                        let id = snapshot.upsert(root, frame.id, name, &n.meta, Some(o.id))?;
                        descend(&mut stack, &n, id, path);
                    } else {
                        // A file became a directory or the reverse.
                        snapshot.remove(root, o.id)?;
                        deleted.push(Pending {
                            path: path.clone(),
                            meta: o.meta,
                            id: o.id,
                        });
                        count(&mut walk.summary, &n);
                        let id = snapshot.upsert(root, frame.id, name, &n.meta, None)?;
                        created.push(Pending {
                            path: path.clone(),
                            meta: n.meta,
                            id,
                        });
                        descend(&mut stack, &n, id, path);
                    }
                }
                (std::cmp::Ordering::Less, Some(n), _) => {
                    i += 1;
                    let name = listing.name(&n);
                    let path = join(&frame.path, name);
                    count(&mut walk.summary, &n);
                    let id = snapshot.upsert(root, frame.id, name, &n.meta, None)?;
                    if !baseline {
                        created.push(Pending {
                            path: path.clone(),
                            meta: n.meta,
                            id,
                        });
                    }
                    descend(&mut stack, &n, id, path);
                }
                (_, _, Some(o)) => {
                    j += 1;
                    deleted.push(Pending {
                        path: join(&frame.path, known.name(&o)),
                        meta: o.meta,
                        id: o.id,
                    });
                }
                _ => break,
            }
        }
    }
    if walk.summary.cancelled {
        snapshot.rollback();
        let counts = (walk.summary.scanned, walk.summary.changed, 0);
        if !baseline {
            walk.state_row(Kind::ReconcileFinished, 1, counts);
        }
        walk.out.flush();
        return Ok(walk.summary);
    }
    if !baseline {
        settle(&mut walk, snapshot, root, &created, deleted)?;
    }
    if baseline {
        let counts = (walk.summary.files, walk.summary.dirs, 0);
        if root_listed {
            walk.state_row(Kind::BaselineEstablished, 0, counts);
        }
    } else {
        let elapsed = (cfg.clock)().1.saturating_sub(started) / 100;
        let counts = (walk.summary.scanned, walk.summary.changed, elapsed);
        walk.state_row(Kind::ReconcileFinished, 0, counts);
    }
    if !walk.out.flush() {
        snapshot.rollback();
        walk.summary.not_durable = true;
        return Ok(walk.summary);
    }
    if baseline && root_listed {
        snapshot.set_baselined(root)?;
    }
    snapshot.commit()?;
    Ok(walk.summary)
}

fn count(summary: &mut Summary, item: &Item) {
    if item.meta.is_dir() {
        summary.dirs += 1;
    } else {
        summary.files += 1;
    }
}

/// Queues a directory for listing; links, mount points and placeholders are
/// entries, not trees.
fn descend(stack: &mut Vec<Frame>, item: &Item, id: i64, path: Vec<u16>) {
    if item.meta.is_dir() && !item.meta.is_reparse() {
        stack.push(Frame { id, path });
    }
}

/// Pairs renames, writes the creates and deletes, and drops what is gone
/// from the snapshot.
fn settle<R, N>(
    walk: &mut Walk<'_, R>,
    snapshot: &mut N,
    root: u16,
    created: &[Pending],
    mut deleted: Vec<Pending>,
) -> Result<(), N::Error>
where
    R: RowOut,
    N: Snapshot,
{
    // A deleted directory takes its known subtree with it.
    let mut subtree = Listing::new();
    let mut at = 0;
    while let Some(entry) = deleted.get(at) {
        at += 1;
        if !entry.meta.is_dir() || entry.meta.is_reparse() {
            continue;
        }
        let (parent_id, parent_path) = (entry.id, entry.path.clone());
        subtree.clear();
        snapshot.children(root, parent_id, &mut subtree)?;
        for item in subtree.items() {
            deleted.push(Pending {
                path: join(&parent_path, subtree.name(item)),
                meta: item.meta,
                id: item.id,
            });
        }
    }
    let mut by_id: HashMap<u128, usize> = HashMap::with_capacity(deleted.len());
    for (index, entry) in deleted.iter().enumerate() {
        if entry.meta.file_id != 0 {
            by_id.insert(entry.meta.file_id, index);
        }
    }
    let mut paired = vec![false; deleted.len()];
    for entry in created {
        let partner = by_id.get(&entry.meta.file_id).copied().filter(|&index| {
            deleted
                .get(index)
                .is_some_and(|d| d.meta.is_dir() == entry.meta.is_dir())
        });
        match partner {
            Some(index) if entry.meta.file_id != 0 => {
                by_id.remove(&entry.meta.file_id);
                if let Some(flag) = paired.get_mut(index) {
                    *flag = true;
                }
                walk.row(Kind::ReconciledRenamed, &entry.path, &entry.meta);
            }
            _ => walk.row(Kind::ReconciledCreated, &entry.path, &entry.meta),
        }
    }
    for (entry, was_paired) in deleted.iter().zip(&paired) {
        if !was_paired {
            walk.row(Kind::ReconciledDeleted, &entry.path, &entry.meta);
        }
        snapshot.remove(root, entry.id)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
