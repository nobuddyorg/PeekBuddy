//! Streaming an export to a file (#56).
//!
//! The rows come from the query layer ([`Reader`]) page by page, oldest
//! first, and go through [`RowWriter`] into `<name>.partial` behind a 1 MiB
//! buffer. On success the file is flushed, synced and renamed to its final
//! name. On cancel or any error the partial file is deleted: a file under the
//! final name is always complete. Memory does not grow with the number of
//! rows: one page, one line buffer and the path cache of the reader.

use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use peekbuddy_core::export::{Format, OffsetFn, Options, RowWriter};
use peekbuddy_core::query::{CancelFlag, Direction, Filter, PageRequest};
use rusqlite::{Connection, OpenFlags};

use crate::catalog::{Catalog, CatalogError};
use crate::read::Reader;

/// Bytes buffered before a write reaches the file.
pub const BUFFER_BYTES: usize = 1024 * 1024;
/// Rows fetched per page.
pub const PAGE_ROWS: usize = 1_000;

/// How far an export has come; shared with the thread that shows it.
#[derive(Debug, Default)]
pub struct Progress {
    written: AtomicU64,
    estimated: AtomicU64,
}

impl Progress {
    /// Rows written so far.
    #[must_use]
    pub fn written(&self) -> u64 {
        self.written.load(Ordering::Relaxed)
    }

    /// An upper bound on the rows to write: every row of every segment.
    #[must_use]
    pub fn estimated(&self) -> u64 {
        self.estimated.load(Ordering::Relaxed)
    }
}

/// How an export ended.
#[derive(Debug)]
pub enum Outcome {
    /// The file is complete under its final name.
    Done {
        /// Rows written.
        rows: u64,
    },
    /// The export was cancelled; nothing is left behind.
    Cancelled,
}

/// Why an export failed. The partial file is gone.
#[derive(Debug)]
pub enum ExportError {
    /// Writing, syncing or renaming failed (a full disk, for one).
    Io(io::Error),
    /// The catalog or a segment could not be read.
    Read(CatalogError),
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "export write failed: {error}"),
            Self::Read(error) => write!(f, "export read failed: {error}"),
        }
    }
}

impl std::error::Error for ExportError {}

impl From<io::Error> for ExportError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<CatalogError> for ExportError {
    fn from(error: CatalogError) -> Self {
        Self::Read(error)
    }
}

/// The partial file of an export to `target`.
#[must_use]
pub fn partial_path(target: &Path) -> PathBuf {
    let mut name = target.as_os_str().to_owned();
    name.push(".partial");
    PathBuf::from(name)
}

/// What an export reads and how it writes.
#[derive(Debug, Clone)]
pub struct Job {
    /// The rows to export.
    pub filter: Filter,
    /// The format.
    pub format: Format,
    /// The format options.
    pub options: Options,
}

/// Exports the rows of `job` to `target` (see the module comment).
///
/// # Errors
///
/// [`ExportError`]; the partial file is removed first.
pub fn export_file(
    reader: &Reader,
    job: &Job,
    target: &Path,
    offset: OffsetFn<'_>,
    cancel: &CancelFlag,
    progress: &Progress,
) -> Result<Outcome, ExportError> {
    export_file_with(reader, job, target, offset, cancel, progress, |file| {
        BufWriter::with_capacity(BUFFER_BYTES, file)
    })
}

/// Like [`export_file`], with the writer around the file chosen by the
/// caller (tests wrap it to fail like a full disk).
///
/// # Errors
///
/// [`ExportError`]; the partial file is removed first.
pub fn export_file_with<W: Write>(
    reader: &Reader,
    job: &Job,
    target: &Path,
    offset: OffsetFn<'_>,
    cancel: &CancelFlag,
    progress: &Progress,
    wrap: impl FnOnce(File) -> W,
) -> Result<Outcome, ExportError> {
    let partial = partial_path(target);
    let result = write_partial(reader, job, &partial, offset, cancel, progress, wrap);
    let finished = match result {
        Ok(Some(rows)) => fs::rename(&partial, target)
            .map(|()| Outcome::Done { rows })
            .map_err(ExportError::Io),
        Ok(None) => Ok(Outcome::Cancelled),
        Err(error) => Err(error),
    };
    if !matches!(finished, Ok(Outcome::Done { .. })) {
        let _ = fs::remove_file(&partial);
    }
    finished
}

/// Writes the partial file; `None` when cancelled.
fn write_partial<W: Write>(
    reader: &Reader,
    job: &Job,
    partial: &Path,
    offset: OffsetFn<'_>,
    cancel: &CancelFlag,
    progress: &Progress,
    wrap: impl FnOnce(File) -> W,
) -> Result<Option<u64>, ExportError> {
    progress.written.store(0, Ordering::Relaxed);
    progress
        .estimated
        .store(estimated_rows(reader)?, Ordering::Relaxed);
    let file = File::create(partial)?;
    let sync = file.try_clone()?;
    let mut writer = RowWriter::new(wrap(file), job.format, job.options)?;
    let mut cursor = None;
    loop {
        if cancel.is_cancelled() {
            return Ok(None);
        }
        let request = PageRequest {
            generation: 0,
            filter: job.filter.clone(),
            cursor,
            direction: Direction::Newer,
            limit: PAGE_ROWS,
        };
        let page = reader.page(&request, cancel)?;
        if page.cancelled {
            return Ok(None);
        }
        // A page is newest first; the export is oldest first.
        for row in page.rows.iter().rev() {
            writer.write_row(row, offset)?;
        }
        progress.written.store(writer.rows(), Ordering::Relaxed);
        if let Some(resume) = page.resume {
            cursor = Some(resume);
            continue;
        }
        cursor = page.newest_id();
        if !page.more || cursor.is_none() {
            break;
        }
    }
    let rows = writer.rows();
    writer.finish()?;
    sync.sync_all()?;
    Ok(Some(rows))
}

/// An upper bound on the rows in the log: sealed segments from the catalog,
/// the segment being written from its hourly statistics.
fn estimated_rows(reader: &Reader) -> Result<u64, ExportError> {
    let catalog = Catalog::open(&reader.dir().join("catalog.db"))?;
    let mut total: u64 = 0;
    for segment in catalog.list().map_err(CatalogError::from)? {
        let rows = if segment.sealed {
            segment.rows
        } else {
            Connection::open_with_flags(
                reader.dir().join(&segment.file),
                OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .and_then(|conn| {
                conn.query_row("SELECT coalesce(sum(n), 0) FROM stats", [], |row| {
                    row.get::<_, i64>(0)
                })
            })
            .unwrap_or(0)
        };
        total = total.saturating_add(u64::try_from(rows).unwrap_or(0));
    }
    Ok(total)
}
