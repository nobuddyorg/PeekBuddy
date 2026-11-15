//! The live rows of the event store, read back for the reconcile walk (#290).
//!
//! A walk after a gap asks which entries the live stream logged as created or
//! renamed to since the last finished walk, and looks whether they are still
//! there. Entries that are gone were created and deleted inside the gap: the
//! snapshot never knew them and the disk no longer has them, so only the log
//! can tell. Nothing here is on the capture path: it runs on the reconcile
//! thread and reads the segments through the same read API as the viewer.

use std::path::Path;

use peekbuddy_core::event::Kind;
use peekbuddy_core::filter::KindSet;
use peekbuddy_core::query::{CancelFlag, Direction, Filter, PageRequest, Row};
use peekbuddy_core::reconcile::LiveLog;
use peekbuddy_store::read::Reader;

use crate::reconciler::RootSpec;

/// Rows per page; a page is read in one go.
const PAGE: usize = 5_000;
/// How many finished walks are looked at to find one that was not cancelled.
const WALKS_LOOKED_AT: usize = 16;

/// The live rows of the segments in one data directory.
#[derive(Debug)]
pub struct StoreLiveLog {
    reader: Reader,
    roots: Vec<(u16, Vec<u16>)>,
}

impl StoreLiveLog {
    /// A live log over the segments in `segments`, for the roots of `specs`.
    #[must_use]
    pub fn new(segments: &Path, specs: &[RootSpec]) -> Self {
        Self {
            reader: Reader::new(segments),
            roots: specs
                .iter()
                .map(|spec| (spec.root.0, spec.path.clone()))
                .collect(),
        }
    }

    /// The newest row of `kinds` older than `before` (the newest of all for
    /// `None`) that `accept` takes.
    ///
    /// A scan that stops at its step limit says where to go on: after a burst
    /// of rows since the last walk, the row looked for is far back.
    fn newest_row(
        &self,
        root: &[u16],
        kinds: &[Kind],
        before: Option<i64>,
        accept: impl Fn(&Row) -> bool,
        cancel: &CancelFlag,
    ) -> Option<Row> {
        let filter = Filter {
            kinds: Some(KindSet::of(kinds)),
            root: Some(root.to_vec()),
            ..Filter::default()
        };
        let mut cursor = before;
        loop {
            let request = PageRequest {
                generation: 0,
                filter: filter.clone(),
                cursor,
                direction: Direction::Older,
                limit: WALKS_LOOKED_AT,
            };
            let page = self.reader.page(&request, cancel).ok()?;
            if let Some(row) = page.rows.iter().find(|row| accept(row)) {
                return Some(row.clone());
            }
            cursor = match page.resume {
                Some(resume) => Some(resume),
                None if page.more => page.oldest_id(),
                None => return None,
            };
        }
    }

    /// The id from which the live rows are read: the row that starts the
    /// newest walk that was not cancelled, or the row that ends it for a
    /// baseline, which has no start row.
    ///
    /// The snapshot is as of the listing of the walk, not of its end: an entry
    /// a live row created while the walk ran may be in no snapshot, and a row
    /// that ends the walk would leave it out of the lookup.
    fn last_finished_walk(&self, root: &[u16], cancel: &CancelFlag) -> Option<i64> {
        // `raw_mask` 1 on a finish row: the walk was cancelled and rolled
        // back, so the snapshot is older than that row.
        let end = self.newest_row(
            root,
            &[Kind::ReconcileFinished, Kind::BaselineEstablished],
            None,
            |row| !(row.kind == Kind::ReconcileFinished && row.raw_mask == 1),
            cancel,
        )?;
        if end.kind != Kind::ReconcileFinished {
            return Some(end.id);
        }
        // Walks of a root do not overlap: the newest start before the finish
        // is its own.
        let start = self.newest_row(
            root,
            &[Kind::ReconcileStarted],
            Some(end.id),
            |_| true,
            cancel,
        );
        Some(start.map_or(end.id, |row| row.id))
    }
}

/// `path` below `root` (both without a `\\?\` prefix), or `None` when it is
/// not below it. Case is ignored, as the file system does.
fn relative<'a>(path: &'a [u16], root: &[u16]) -> Option<&'a [u16]> {
    let (head, rest) = (path.get(..root.len())?, path.get(root.len()..)?);
    let same = head.len() == root.len()
        && head.iter().zip(root).all(|(a, b)| {
            char::from_u32(u32::from(*a)).map(|c| c.to_ascii_lowercase())
                == char::from_u32(u32::from(*b)).map(|c| c.to_ascii_lowercase())
        });
    if !same {
        return None;
    }
    match rest.split_first() {
        Some((separator, below)) if *separator == u16::from(b'\\') && !below.is_empty() => {
            Some(below)
        }
        _ => None,
    }
}

/// The root path without the `\\?\` of a long path.
fn plain(root: &[u16]) -> &[u16] {
    let prefix: Vec<u16> = r"\\?\".encode_utf16().collect();
    root.strip_prefix(prefix.as_slice()).unwrap_or(root)
}

impl LiveLog for StoreLiveLog {
    fn created_since_last_walk(&self, root: u16, each: &mut dyn FnMut(&[u16]) -> bool) {
        let Some((_, root_path)) = self.roots.iter().find(|(index, _)| *index == root) else {
            return;
        };
        let cancel = CancelFlag::new();
        let Some(after) = self.last_finished_walk(root_path, &cancel) else {
            return;
        };
        let top = plain(root_path);
        let filter = Filter {
            // The rows of a walk that was cancelled count too: they logged
            // entries whose snapshot was rolled back.
            kinds: Some(KindSet::of(&[
                Kind::Created,
                Kind::RenamedTo,
                Kind::ReconciledCreated,
                Kind::ReconciledRenamed,
            ])),
            root: Some(root_path.clone()),
            ..Filter::default()
        };
        let mut cursor = Some(after);
        loop {
            let request = PageRequest {
                generation: 0,
                filter: filter.clone(),
                cursor,
                direction: Direction::Newer,
                limit: PAGE,
            };
            let Ok(page) = self.reader.page(&request, &cancel) else {
                return;
            };
            // A page is newest first; the walk wants oldest first.
            for row in page.rows.iter().rev() {
                if let Some(below) = relative(&row.path, top)
                    && !each(below)
                {
                    return;
                }
            }
            if let Some(resume) = page.resume {
                cursor = Some(resume);
                continue;
            }
            cursor = page.newest_id();
            if !page.more || cursor.is_none() {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{plain, relative};

    fn w(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    #[test]
    fn a_path_below_the_root_is_made_relative_whatever_its_case() {
        let root = w(r"C:\Demo\Projects");
        assert_eq!(
            relative(&w(r"c:\demo\projects\a\b.txt"), &root),
            Some(w(r"a\b.txt").as_slice())
        );
    }

    #[test]
    fn the_root_itself_a_sibling_and_a_longer_name_are_not_below_it() {
        let root = w(r"C:\Demo\Projects");
        assert_eq!(relative(&w(r"C:\Demo\Projects"), &root), None);
        assert_eq!(relative(&w(r"C:\Demo\Projects\"), &root), None);
        assert_eq!(relative(&w(r"C:\Demo\Projects2\a"), &root), None);
        assert_eq!(relative(&w(r"C:\Demo\Other\a"), &root), None);
        assert_eq!(relative(&w(r"C:\Demo"), &root), None);
    }

    #[test]
    fn the_long_prefix_comes_off_a_root_path() {
        assert_eq!(plain(&w(r"\\?\C:\x")), w(r"C:\x").as_slice());
        assert_eq!(plain(&w(r"C:\x")), w(r"C:\x").as_slice());
    }
}
