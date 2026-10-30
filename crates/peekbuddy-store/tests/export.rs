//! Exporting from real segments (#56): streamed output equals the paged
//! rows, cancel and a failed write leave nothing behind.
#![expect(clippy::unwrap_used, reason = "tests unwrap")]

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use peekbuddy_core::event::{Event, Flags, Kind, NameKind, NameRef, RootIdx, Source, VolIdx};
use peekbuddy_core::export::Format;
use peekbuddy_core::export::Options;
use peekbuddy_core::export::read::{ReadOptions, read_csv, read_jsonl};
use peekbuddy_core::filter::KindSet;
use peekbuddy_core::query::{CancelFlag, Direction, Filter, PageRequest, Row};
use peekbuddy_store::batch::Batch;
use peekbuddy_store::export::{
    ExportError, Job, Outcome, Progress, export_file, export_file_with, partial_path,
};
use peekbuddy_store::read::Reader;
use peekbuddy_store::retention::{Caps, NoViewer};
use peekbuddy_store::segment::{DayFn, DayKey, Rotation};
use peekbuddy_store::writer::{RootInfo, SegmentsConfig, VolInfo, Writer};

const HOUR: u64 = 36_000_000_000;
const DAY: u64 = 24 * HOUR;

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("pb-export-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn event(ts: u64, kind: Kind) -> Event {
    Event {
        ts,
        qpc: ts,
        src_ts: 0,
        source: Source::Rdcw,
        kind,
        raw_mask: 1,
        source_info: 0,
        is_dir: false,
        size: 7,
        attrs: 0x20,
        reparse_tag: 0,
        file_id: u128::from(ts),
        parent_file_id: 5,
        usn: 0,
        vol: VolIdx(0),
        root: RootIdx(0),
        name: NameRef::EMPTY,
        name_kind: NameKind::Long,
        flags: Flags::NONE,
    }
}

fn day_of() -> DayFn {
    Arc::new(|ts| DayKey {
        year: 2026,
        month: 10,
        day: u8::try_from(1 + ts / DAY).unwrap_or(28).min(28),
    })
}

/// A store of `count` events over `days` days.
fn build(dir: &Dir, days: u64, count: u64) {
    let config = SegmentsConfig {
        rotation: Rotation {
            dir: dir.path().to_path_buf(),
            day_of: day_of(),
            size_cap: u64::MAX,
        },
        caps: Caps::default(),
        retire: Arc::new(NoViewer),
    };
    let (mut writer, _) = Writer::open_segments(config, 1).unwrap();
    writer
        .register_root(&RootInfo {
            idx: RootIdx(0),
            vol: VolInfo {
                guid: String::from("vol-1"),
                serial: 1,
                letter: Some(String::from("C:")),
                journal_id: None,
            },
            path: units(r"\\?\C:\Watched"),
            recursive: true,
        })
        .unwrap();
    for index in 0..count {
        let ts = 1 + index * (days * DAY / count);
        let kind = if index % 3 == 0 {
            Kind::Created
        } else {
            Kind::Modified
        };
        let mut batch = Batch::with_capacity(2, 1024);
        let name = format!("dir{}\\file {index}, \"q\".txt", index % 4);
        batch.push(event(ts, kind), &units(&name)).unwrap();
        writer.write(&[batch], &[]).unwrap();
    }
    writer.wait_background();
    writer.flush_system_rows().unwrap();
}

fn offset(_ts: u64) -> i32 {
    7200
}

fn job(format: Format) -> Job {
    Job {
        filter: Filter::default(),
        format,
        options: Options::default(),
    }
}

/// The rows the query layer returns, oldest first.
fn paged(reader: &Reader, filter: &Filter) -> Vec<Row> {
    let cancel = CancelFlag::new();
    let mut out: Vec<Row> = Vec::new();
    let mut cursor = None;
    loop {
        let page = reader
            .page(
                &PageRequest {
                    generation: 0,
                    filter: filter.clone(),
                    cursor,
                    direction: Direction::Newer,
                    limit: 7,
                },
                &cancel,
            )
            .unwrap();
        out.extend(page.rows.iter().rev().cloned());
        cursor = page.newest_id();
        if !page.more || cursor.is_none() {
            return out;
        }
    }
}

#[test]
fn an_export_equals_the_paged_rows_in_both_formats() {
    let dir = Dir::new("equal");
    build(&dir, 3, 40);
    let reader = Reader::new(dir.path());
    let expected = paged(&reader, &Filter::default());
    assert!(expected.len() >= 40);

    for format in [Format::Csv, Format::JsonLines] {
        let target = dir.path().join("out.export");
        let progress = Progress::default();
        let outcome = export_file(
            &reader,
            &job(format),
            &target,
            &offset,
            &CancelFlag::new(),
            &progress,
        )
        .unwrap();
        let rows = match outcome {
            Outcome::Done { rows } => rows,
            Outcome::Cancelled => 0,
        };
        assert_eq!(rows, expected.len() as u64);
        assert_eq!(progress.written(), rows);
        assert!(progress.estimated() >= rows);
        assert!(!partial_path(&target).exists());
        let text = std::fs::read_to_string(&target).unwrap();
        let back = match format {
            Format::Csv => read_csv(&text, ReadOptions::default()).unwrap(),
            Format::JsonLines => read_jsonl(&text).unwrap(),
        };
        assert_eq!(back, expected);
    }
}

#[test]
fn a_filter_limits_the_export() {
    let dir = Dir::new("filter");
    build(&dir, 2, 30);
    let reader = Reader::new(dir.path());
    let filter = Filter {
        kinds: Some(KindSet::of(&[Kind::Created])),
        ..Filter::default()
    };
    let target = dir.path().join("created.jsonl");
    let job = Job {
        filter: filter.clone(),
        ..job(Format::JsonLines)
    };
    export_file(
        &reader,
        &job,
        &target,
        &offset,
        &CancelFlag::new(),
        &Progress::default(),
    )
    .unwrap();
    let back = read_jsonl(&std::fs::read_to_string(&target).unwrap()).unwrap();
    assert_eq!(back, paged(&reader, &filter));
    assert!(back.iter().all(|row| row.kind == Kind::Created));
}

#[test]
fn a_cancelled_export_leaves_nothing() {
    let dir = Dir::new("cancel");
    build(&dir, 1, 20);
    let reader = Reader::new(dir.path());
    let target = dir.path().join("never.csv");
    let cancel = CancelFlag::new();
    cancel.cancel();
    let outcome = export_file(
        &reader,
        &job(Format::Csv),
        &target,
        &offset,
        &cancel,
        &Progress::default(),
    )
    .unwrap();
    assert!(matches!(outcome, Outcome::Cancelled));
    assert!(!target.exists());
    assert!(!partial_path(&target).exists());
}

/// A writer that fails after `limit` bytes, like a full disk.
struct Full<W> {
    inner: W,
    left: usize,
}

impl<W: Write> Write for Full<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.left == 0 {
            return Err(io::Error::other("disk full"));
        }
        let take = buf.len().min(self.left);
        self.left -= take;
        self.inner.write(buf.get(..take).unwrap_or_default())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[test]
fn a_failed_write_reports_an_error_and_leaves_no_file() {
    let dir = Dir::new("full");
    build(&dir, 1, 50);
    let reader = Reader::new(dir.path());
    let target = dir.path().join("full.csv");
    let error = export_file_with(
        &reader,
        &job(Format::Csv),
        &target,
        &offset,
        &CancelFlag::new(),
        &Progress::default(),
        |file| Full {
            inner: file,
            left: 300,
        },
    )
    .unwrap_err();
    assert!(matches!(error, ExportError::Io(_)), "{error}");
    assert!(!target.exists(), "no file under the final name");
    assert!(!partial_path(&target).exists());
}

#[test]
fn an_existing_file_survives_a_failed_export() {
    let dir = Dir::new("keep");
    build(&dir, 1, 10);
    let reader = Reader::new(dir.path());
    let target = dir.path().join("keep.csv");
    std::fs::write(&target, "old").unwrap();
    export_file_with(
        &reader,
        &job(Format::Csv),
        &target,
        &offset,
        &CancelFlag::new(),
        &Progress::default(),
        |file| Full {
            inner: file,
            left: 10,
        },
    )
    .unwrap_err();
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "old");
}
