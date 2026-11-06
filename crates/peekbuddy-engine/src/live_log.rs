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
use peekbuddy_core::query::{CancelFlag, Direction, Filter, PageRequest};
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

    /// The id of the newest row that ends a walk that was not cancelled: the
    /// snapshot is as of that row.
    fn last_finished_walk(&self, root: &[u16], cancel: &CancelFlag) -> Option<i64> {
        let request = PageRequest {
            generation: 0,
            filter: Filter {
                kinds: Some(KindSet::of(&[
                    Kind::ReconcileFinished,
                    Kind::BaselineEstablished,
                ])),
                root: Some(root.to_vec()),
                ..Filter::default()
            },
            cursor: None,
            direction: Direction::Older,
            limit: WALKS_LOOKED_AT,
        };
        let page = self.reader.page(&request, cancel).ok()?;
        // `raw_mask` 1 on a finish row: the walk was cancelled and rolled
        // back, so the snapshot is older than that row.
        page.rows
            .iter()
            .find(|row| !(row.kind == Kind::ReconcileFinished && row.raw_mask == 1))
            .map(|row| row.id)
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
            kinds: Some(KindSet::of(&[Kind::Created, Kind::RenamedTo])),
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
