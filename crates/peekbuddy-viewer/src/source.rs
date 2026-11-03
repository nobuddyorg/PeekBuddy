//! Where the rows come from (#66).
//!
//! [`PageSource`] is what the query worker needs from the store: a page by
//! keyset, the extent of the log, and the count of hidden rows. The real
//! source reads the segments with [`Reader`] (read-only connections, one
//! segment at a time); tests use an in-memory log, which is also how the
//! "no query runs on the UI thread" rule is checked.

use std::path::{Path, PathBuf};

use peekbuddy_core::query::{CancelFlag, Filter, Page, PageRequest};
use peekbuddy_store::catalog::Catalog;
use peekbuddy_store::read::Reader;

use crate::timeline::Extent;

/// What the worker reads.
pub trait PageSource: Send + Sync + 'static {
    /// Answers one request.
    ///
    /// # Errors
    ///
    /// The reason, in words for the status line.
    fn page(&self, request: &PageRequest, cancel: &CancelFlag) -> Result<Page, String>;

    /// The newest and the oldest event id, from the catalog (never by
    /// counting rows). `None` for a log that has no row.
    ///
    /// # Errors
    ///
    /// The reason, in words.
    fn extent(&self) -> Result<Option<Extent>, String>;

    /// How many rows the filter hides (`filter.hide`) that pass the rest of
    /// it. `None` when cancelled.
    ///
    /// # Errors
    ///
    /// The reason, in words.
    fn hidden(&self, filter: &Filter, cancel: &CancelFlag) -> Result<Option<u64>, String>;
}

/// The segments of a data directory.
#[derive(Debug)]
pub struct StoreSource {
    reader: Reader,
    catalog: PathBuf,
}

impl StoreSource {
    /// A source over the segments in `dir` (where `catalog.db` lives).
    #[must_use]
    pub fn new(dir: &Path) -> Self {
        Self {
            reader: Reader::new(dir),
            catalog: dir.join("catalog.db"),
        }
    }

    /// Whether any engine ever wrote here. Without a catalog there is
    /// nothing to read, and opening one for reading would create it.
    fn exists(&self) -> bool {
        self.catalog.is_file()
    }
}

impl PageSource for StoreSource {
    fn page(&self, request: &PageRequest, cancel: &CancelFlag) -> Result<Page, String> {
        if !self.exists() {
            return Ok(Page {
                generation: request.generation,
                ..Page::default()
            });
        }
        self.reader
            .page(request, cancel)
            .map_err(|error| error.to_string())
    }

    fn extent(&self) -> Result<Option<Extent>, String> {
        if !self.exists() {
            return Ok(None);
        }
        let catalog =
            Catalog::open_for_reading(&self.catalog).map_err(|error| error.to_string())?;
        let segments = catalog.list().map_err(|error| error.to_string())?;
        let mut extent: Option<Extent> = None;
        for segment in segments.iter().filter(|segment| segment.rows > 0) {
            extent = Some(match extent {
                None => Extent {
                    newest: segment.last_id,
                    oldest: segment.first_id,
                },
                Some(seen) => Extent {
                    newest: seen.newest.max(segment.last_id),
                    oldest: seen.oldest.min(segment.first_id),
                },
            });
        }
        Ok(extent)
    }

    fn hidden(&self, filter: &Filter, cancel: &CancelFlag) -> Result<Option<u64>, String> {
        if !self.exists() {
            return Ok(Some(0));
        }
        self.reader
            .hidden(filter, cancel)
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
pub(crate) mod fake {
    //! An in-memory log with the reader's keyset semantics.

    use std::sync::{Arc, Mutex, PoisonError};
    use std::thread::ThreadId;

    use peekbuddy_core::event::{Kind, Source};
    use peekbuddy_core::query::{CancelFlag, Direction, Filter, Page, PageRequest, Row};

    use super::PageSource;
    use crate::timeline::Extent;

    /// A log with the ids `first..=last`.
    #[derive(Debug)]
    pub struct FakeLog {
        state: Mutex<State>,
    }

    #[derive(Debug)]
    struct State {
        first: i64,
        last: i64,
        /// At most this many rows per call, with a `resume` cursor, like a
        /// scan that ends at its time budget.
        step: Option<usize>,
        /// The threads that asked.
        threads: Vec<ThreadId>,
        calls: usize,
        extents: usize,
        hidden: u64,
    }

    impl FakeLog {
        pub fn new(rows: i64) -> Arc<Self> {
            Arc::new(Self {
                state: Mutex::new(State {
                    first: 1,
                    last: rows,
                    step: None,
                    threads: Vec::new(),
                    calls: 0,
                    extents: 0,
                    hidden: 0,
                }),
            })
        }

        fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
            f(&mut self.state.lock().unwrap_or_else(PoisonError::into_inner))
        }

        pub fn grow(&self, by: i64) {
            self.with(|s| s.last += by);
        }

        pub fn step(&self, rows: usize) {
            self.with(|s| s.step = Some(rows));
        }

        pub fn set_hidden(&self, hidden: u64) {
            self.with(|s| s.hidden = hidden);
        }

        pub fn threads(&self) -> Vec<ThreadId> {
            self.with(|s| s.threads.clone())
        }

        pub fn calls(&self) -> usize {
            self.with(|s| s.calls)
        }

        pub fn extents(&self) -> usize {
            self.with(|s| s.extents)
        }

        pub fn row(id: i64) -> Row {
            Row {
                id,
                ts: 134_374_896_000_000_000 + u64::try_from(id).unwrap_or(0) * 10_000,
                kind: Kind::Modified,
                source: Source::Rdcw,
                raw_mask: 3,
                is_dir: false,
                size: Some(100),
                attrs: None,
                reparse_tag: 0,
                file_id: 0,
                name: Vec::new(),
                path: format!(r"C:\w\f{id}.txt").encode_utf16().collect(),
                old_path: Vec::new(),
                root: Vec::new(),
                flags: 0,
            }
        }
    }

    impl PageSource for FakeLog {
        fn page(&self, request: &PageRequest, cancel: &CancelFlag) -> Result<Page, String> {
            self.with(|s| {
                s.threads.push(std::thread::current().id());
                s.calls += 1;
                if cancel.is_cancelled() {
                    return Ok(Page {
                        generation: request.generation,
                        cancelled: true,
                        ..Page::default()
                    });
                }
                let limit = request.limit.min(s.step.unwrap_or(usize::MAX));
                let mut rows = Vec::new();
                let mut resume = None;
                let more;
                match request.direction {
                    Direction::Older => {
                        let mut id = request.cursor.map_or(s.last, |c| (c - 1).min(s.last));
                        while id >= s.first && rows.len() < limit {
                            rows.push(Self::row(id));
                            id -= 1;
                        }
                        more = id >= s.first;
                        if more && s.step.is_some() {
                            resume = Some(id + 1);
                        }
                    }
                    Direction::Newer => {
                        let mut id = request.cursor.map_or(s.first, |c| c + 1).max(s.first);
                        while id <= s.last && rows.len() < limit {
                            rows.push(Self::row(id));
                            id += 1;
                        }
                        more = id <= s.last;
                        if more && s.step.is_some() {
                            resume = Some(id - 1);
                        }
                        rows.reverse();
                    }
                }
                Ok(Page {
                    generation: request.generation,
                    scanned: u64::try_from(rows.len()).unwrap_or(0),
                    rows,
                    more,
                    resume,
                    ..Page::default()
                })
            })
        }

        fn extent(&self) -> Result<Option<Extent>, String> {
            self.with(|s| {
                s.extents += 1;
                Ok((s.last >= s.first).then_some(Extent {
                    newest: s.last,
                    oldest: s.first,
                }))
            })
        }

        fn hidden(&self, _: &Filter, _: &CancelFlag) -> Result<Option<u64>, String> {
            self.with(|s| Ok(Some(s.hidden)))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use peekbuddy_core::query::{CancelFlag, Filter, PageRequest};

    use super::{PageSource, StoreSource};

    fn missing() -> PathBuf {
        std::env::temp_dir().join(format!("pb-viewer-missing-{}", std::process::id()))
    }

    #[test]
    fn a_directory_without_a_catalog_is_an_empty_log_and_stays_that_way() {
        let dir = missing();
        let source = StoreSource::new(&dir);
        assert_eq!(source.extent().unwrap(), None);
        let page = source
            .page(
                &PageRequest::newest(4, Filter::default()),
                &CancelFlag::new(),
            )
            .unwrap();
        assert!(page.rows.is_empty() && !page.more);
        assert_eq!(page.generation, 4);
        assert_eq!(
            source
                .hidden(&Filter::default_view(), &CancelFlag::new())
                .unwrap(),
            Some(0)
        );
        // Reading did not create anything.
        assert!(!dir.exists());
    }
}
