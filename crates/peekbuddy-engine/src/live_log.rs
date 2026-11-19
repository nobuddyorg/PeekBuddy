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
    /// newest walk that was not cancelled, or 0 for a baseline, which has no
    /// start row.
    ///
    /// The snapshot is as of the listing of the walk, not of its end: an entry
    /// a live row created while the walk ran may be in no snapshot, and a row
    /// that ends the walk would leave it out of the lookup. A baseline can run
    /// for minutes on a large tree, so every row of the root counts.
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
            return Some(0);
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
    use std::path::PathBuf;
    use std::sync::Arc;

    use peekbuddy_core::event::{Event, Flags, Kind, NameKind, NameRef, RootIdx, Source, VolIdx};
    use peekbuddy_core::reconcile::LiveLog;
    use peekbuddy_store::batch::Batch;
    use peekbuddy_store::retention::{Caps, NoViewer};
    use peekbuddy_store::segment::{DayFn, DayKey, Rotation};
    use peekbuddy_store::writer::{RootInfo, SegmentsConfig, VolInfo, Writer};

    use super::{PAGE, StoreLiveLog, plain, relative};
    use crate::reconciler::RootSpec;

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
        // As long as the root, with another name.
        assert_eq!(relative(&w(r"C:\Demo\Projeqts\a"), &root), None);
    }

    #[test]
    fn the_long_prefix_comes_off_a_root_path() {
        assert_eq!(plain(&w(r"\\?\C:\x")), w(r"C:\x").as_slice());
        assert_eq!(plain(&w(r"C:\x")), w(r"C:\x").as_slice());
    }

    const ROOT_A: &str = r"\\?\C:\Watched";
    const ROOT_B: &str = r"\\?\C:\Watched\Inner";

    struct Dir(PathBuf);

    impl Dir {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("pb-livelog-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn event(ts: u64, root: u16, kind: Kind, raw_mask: u32) -> Event {
        Event {
            ts,
            qpc: ts,
            src_ts: 0,
            source: Source::Rdcw,
            kind,
            raw_mask,
            source_info: 0,
            is_dir: false,
            size: 1,
            attrs: 0x20,
            reparse_tag: 0,
            file_id: u128::from(ts),
            parent_file_id: 5,
            usn: 0,
            vol: VolIdx(0),
            root: RootIdx(root),
            name: NameRef::EMPTY,
            name_kind: NameKind::Long,
            flags: Flags::NONE,
        }
    }

    /// A store with the two roots and the rows of `rows`, in order:
    /// `(root, kind, raw_mask, name below the root)`.
    fn store(dir: &Dir, rows: &[(u16, Kind, u32, &str)]) -> StoreLiveLog {
        let day_of: DayFn = Arc::new(|_| DayKey {
            year: 2026,
            month: 11,
            day: 18,
        });
        let config = SegmentsConfig {
            rotation: Rotation {
                dir: dir.0.clone(),
                day_of,
                size_cap: u64::MAX,
            },
            caps: Caps::default(),
            retire: Arc::new(NoViewer),
        };
        let (mut writer, _) = Writer::open_segments(config, 1).unwrap();
        for (idx, path) in [(0, ROOT_A), (1, ROOT_B)] {
            writer
                .register_root(&RootInfo {
                    idx: RootIdx(idx),
                    vol: VolInfo {
                        guid: String::from("vol-1"),
                        serial: 1,
                        letter: Some(String::from("C:")),
                        journal_id: None,
                    },
                    path: w(path),
                    recursive: true,
                })
                .unwrap();
        }
        for (chunk, group) in rows.chunks(200).enumerate() {
            let mut batch = Batch::with_capacity(201, 1 << 16);
            for (offset, (root, kind, raw_mask, name)) in group.iter().enumerate() {
                let ts = 1 + (chunk * 200 + offset) as u64;
                batch
                    .push(event(ts, *root, *kind, *raw_mask), &w(name))
                    .unwrap();
            }
            writer.write(&[batch], &[]).unwrap();
        }
        writer.wait_background();
        let specs: Vec<RootSpec> = [(0, ROOT_A), (1, ROOT_B)]
            .iter()
            .map(|(idx, path)| RootSpec {
                root: RootIdx(*idx),
                vol: VolIdx(0),
                path: w(path),
            })
            .collect();
        StoreLiveLog::new(&dir.0, &specs)
    }

    fn created(log: &StoreLiveLog, root: u16) -> Vec<String> {
        let mut names = Vec::new();
        log.created_since_last_walk(root, &mut |below| {
            names.push(String::from_utf16_lossy(below));
            true
        });
        names
    }

    #[test]
    fn the_lookup_starts_at_the_newest_walk_that_was_not_cancelled_of_its_own_root() {
        use Kind::{Created, Modified, ReconcileFinished as Done, ReconcileStarted as Start};
        let dir = Dir::new("walks");
        let mut rows = vec![
            (0, Created, 1, "before"),
            (0, Start, 1, ""),
            (0, Created, 1, "inwalk"),
            (0, Modified, 1, "touched"),
            (0, Done, 0, ""),
            (0, Created, 1, "after"),
            // A walk of the other root, which lies inside the first and is
            // newer than everything of the first.
            (1, Start, 1, ""),
            (1, Created, 1, "other-only"),
            (1, Done, 0, ""),
            // A cancelled walk of the first root does not move the start.
            (0, Start, 1, ""),
            (0, Created, 1, "cancelled"),
        ];
        // Enough cancelled finishes to push the good one past the first page.
        rows.extend((0..40).map(|_| (0, Done, 1, "")));
        let log = store(&dir, &rows);
        assert_eq!(created(&log, 0), ["inwalk", "after", "cancelled"]);
        assert_eq!(created(&log, 1), ["other-only"]);
    }

    #[test]
    fn a_baseline_counts_every_row_of_its_root() {
        use Kind::{BaselineEstablished as Base, Created};
        let dir = Dir::new("baseline");
        let log = store(
            &dir,
            &[
                (0, Created, 1, "early"),
                (0, Base, 1, ""),
                (0, Created, 1, "late"),
                (1, Created, 1, "other"),
            ],
        );
        assert_eq!(created(&log, 0), ["early", "late"]);
    }

    #[test]
    fn a_root_without_a_walk_or_an_unknown_root_gives_nothing() {
        let dir = Dir::new("none");
        let log = store(&dir, &[(0, Kind::Created, 1, "a")]);
        assert_eq!(created(&log, 0), Vec::<String>::new());
        assert_eq!(created(&log, 7), Vec::<String>::new());
    }

    #[test]
    fn only_cancelled_walks_give_nothing_to_look_from() {
        use Kind::{Created, ReconcileFinished as Done};
        let dir = Dir::new("cancelled");
        let log = store(&dir, &[(0, Created, 1, "a"), (0, Done, 1, "")]);
        assert_eq!(created(&log, 0), Vec::<String>::new());
    }

    #[test]
    fn a_callback_that_says_stop_ends_the_lookup() {
        use Kind::Created;
        let dir = Dir::new("stop");
        let log = store(
            &dir,
            &[
                (0, Kind::BaselineEstablished, 1, ""),
                (0, Created, 1, "a"),
                (0, Created, 1, "b"),
                (0, Created, 1, "c"),
            ],
        );
        let mut seen = Vec::new();
        log.created_since_last_walk(0, &mut |below| {
            seen.push(String::from_utf16_lossy(below));
            seen.len() < 2
        });
        assert_eq!(seen, ["a", "b"]);
    }

    #[test]
    fn more_rows_than_a_page_are_all_looked_at_oldest_first() {
        let dir = Dir::new("pages");
        let names: Vec<String> = (0..PAGE * 6).map(|index| format!("f{index}")).collect();
        let mut rows: Vec<(u16, Kind, u32, &str)> = vec![(0, Kind::BaselineEstablished, 1, "")];
        rows.extend(
            names
                .iter()
                .map(|name| (0, Kind::Created, 1, name.as_str())),
        );
        let log = store(&dir, &rows);
        let found = created(&log, 0);
        assert_eq!(found.len(), names.len());
        assert!(found == names, "the rows came back in another order");
    }
}
