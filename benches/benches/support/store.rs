//! The store of millions of rows the query bench (#267) and the private
//! bytes run (#347) read. It takes minutes to build, so callers keep it
//! between runs with `PEEKBUDDY_QUERY_DIR`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use peekbuddy_core::event::{Event, Flags, Kind, NameKind, NameRef, RootIdx, Source, VolIdx};
use peekbuddy_store::batch::Batch;
use peekbuddy_store::retention::{Caps, NoViewer};
use peekbuddy_store::segment::{DayFn, DayKey, Rotation};
use peekbuddy_store::writer::{RootInfo, SegmentsConfig, VolInfo, Writer};

pub const ROOT: &str = r"\\?\C:\Watched";
/// Events in one batch.
pub const BATCH: usize = 8_192;
/// Directories at the top and below each of them.
pub const TOP: u64 = 100;
pub const SUB: u64 = 100;

pub fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

pub fn event(ts: u64, kind: Kind) -> Event {
    Event {
        ts,
        qpc: ts,
        src_ts: 0,
        source: Source::Rdcw,
        kind,
        raw_mask: 1,
        source_info: 0,
        is_dir: false,
        size: 1,
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

/// Files in the tree: a log of 10M rows is about a million files with ten
/// events each, not ten million files.
pub const FILES: u64 = 1_000_000;

/// The path of row `index`: `top/sub/file`, so every directory holds about
/// `FILES / 10,000` files, and one file in a thousand has a rare word in it.
pub fn path_of(index: u64) -> String {
    let file = index % FILES;
    let rare = if file % 1_000 == 7 { "-rareword" } else { "" };
    format!(
        r"top{}\sub{}\file{file}{rare}.dat",
        file % TOP,
        (file / TOP) % SUB
    )
}

pub const KINDS: [Kind; 4] = [
    Kind::Created,
    Kind::Modified,
    Kind::Deleted,
    Kind::RenamedTo,
];

pub fn day_of() -> DayFn {
    Arc::new(|_| DayKey {
        year: 2026,
        month: 11,
        day: 1,
    })
}

pub fn build(dir: &Path, rows: u64) {
    let config = SegmentsConfig {
        rotation: Rotation {
            dir: dir.to_path_buf(),
            day_of: day_of(),
            size_cap: u64::MAX,
        },
        caps: Caps::default(),
        retire: Arc::new(NoViewer),
    };
    let Ok((mut writer, _)) = Writer::open_segments(config, 1) else {
        return;
    };
    let registered = writer.register_root(&RootInfo {
        idx: RootIdx(0),
        vol: VolInfo {
            guid: String::from("vol-1"),
            serial: 1,
            letter: Some(String::from("C:")),
            journal_id: None,
        },
        path: units(ROOT),
        recursive: true,
    });
    if registered.is_err() {
        return;
    }
    let mut index = 0;
    while index < rows {
        let mut batch = Batch::with_capacity(BATCH, BATCH * 96);
        for _ in 0..BATCH {
            if index >= rows {
                break;
            }
            let kind = KINDS[usize::try_from(index % 4).unwrap_or(0)];
            let pushed = batch.push(event(1 + index, kind), &units(&path_of(index)));
            if pushed.is_err() {
                break;
            }
            index += 1;
        }
        if writer.write(&[batch], &[]).is_err() {
            return;
        }
    }
    writer.wait_background();
    let _ = std::fs::write(dir.join("rows"), rows.to_string());
}

/// The store for `rows`, built when the kept directory has none.
pub fn store(rows: u64) -> Option<(PathBuf, Option<tempdir::Temp>)> {
    let kept = std::env::var_os("PEEKBUDDY_QUERY_DIR").map(PathBuf::from);
    let dir = kept.clone().unwrap_or_else(|| {
        std::env::temp_dir().join(format!("pb-query-bench-{}", std::process::id()))
    });
    let have = std::fs::read_to_string(dir.join("rows")).ok();
    if have.as_deref() != Some(&rows.to_string()) {
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).ok()?;
        build(&dir, rows);
    }
    let guard = kept.is_none().then(|| tempdir::Temp(dir.clone()));
    Some((dir, guard))
}

pub mod tempdir {
    /// A directory removed when dropped.
    pub struct Temp(pub std::path::PathBuf);

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
