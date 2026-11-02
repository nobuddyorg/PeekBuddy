//! A really full volume (#266): the store lives on a small virtual disk that
//! is filled to the last byte. The writer must report `failing` and keep
//! retrying, nothing already committed may change, and once space is freed it
//! must recover on its own and take everything that was pushed.
//!
//! Needs an elevated token and `diskpart`, so it only runs when
//! `PEEKBUDDY_VHD_TESTS=1` is set (the nightly `vhd` job). Where it is set,
//! a diskpart failure fails the test: there is no silent skip.
#![cfg(windows)]
#![expect(clippy::unwrap_used, reason = "tests unwrap")]

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use peekbuddy_core::event::{Event, Flags, Kind, NameKind, NameRef, RootIdx, Source, VolIdx};
use peekbuddy_store::pipeline::{Config, Pipeline, Producer};
use peekbuddy_store::writer::{RootInfo, VolInfo, Writer};
use rusqlite::Connection;

/// Events in each batch.
const PER_BATCH: u64 = 20;

fn enabled() -> bool {
    std::env::var_os("PEEKBUDDY_VHD_TESTS").is_some()
}

fn diskpart(script: &str) -> Result<String, String> {
    let path = std::env::temp_dir().join(format!("pbfull-{}.txt", std::process::id()));
    std::fs::write(&path, script).unwrap();
    let output = Command::new("diskpart.exe")
        .arg("/s")
        .arg(&path)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let _ = std::fs::remove_file(&path);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if output.status.success() {
        Ok(text)
    } else {
        Err(format!("{}: {text}", output.status))
    }
}

/// A virtual disk mounted in a folder; detached again on drop.
struct Disk {
    vhd: PathBuf,
    mount: PathBuf,
    base: PathBuf,
}

impl Disk {
    fn new(tag: &str) -> Self {
        let short = std::env::temp_dir().join(format!("pbfull-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(short.join("mount")).unwrap();
        // The runner's temp directory is an 8.3 path; diskpart wants the long one.
        let canonical = std::fs::canonicalize(&short).unwrap();
        let text = canonical.to_string_lossy();
        let base = PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(&text));
        let (vhd, mount) = (base.join("disk.vhd"), base.join("mount"));
        let created = diskpart(&format!(
            "create vdisk file=\"{}\" maximum=48 type=expandable\nattach vdisk\ncreate partition primary\nformat fs=ntfs quick label=pbfull\nassign mount=\"{}\"\n",
            vhd.display(),
            mount.display()
        ));
        let disk = Self { vhd, mount, base };
        assert!(created.is_ok(), "diskpart failed: {created:?}");
        disk
    }
}

impl Drop for Disk {
    fn drop(&mut self) {
        let _ = diskpart(&format!(
            "select vdisk file=\"{}\"\ndetach vdisk\n",
            self.vhd.display()
        ));
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

/// Fills the volume of `dir` until a write fails; returns the file, which is
/// removed to free the space again.
fn fill(dir: &Path) -> PathBuf {
    let path = dir.join("filler.bin");
    let mut file = std::fs::File::create(&path).unwrap();
    for chunk in [256 * 1024, 4096, 512] {
        let block = vec![0xAB; chunk];
        while file.write_all(&block).is_ok() {}
    }
    let _ = file.flush();
    path
}

fn event(ts: u64) -> Event {
    Event {
        ts,
        qpc: ts,
        src_ts: 0,
        source: Source::Rdcw,
        kind: Kind::Created,
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

fn push(producer: &Producer, first: u64) {
    let mut batch = producer.new_batch();
    for ts in first..first + PER_BATCH {
        let name: Vec<u16> = format!("d\\f{ts}-{}.txt", "x".repeat(200))
            .encode_utf16()
            .collect();
        batch.push(event(ts), &name).unwrap();
    }
    let _ = producer.push(batch);
}

fn open_writer(db: &Path) -> Writer {
    let mut writer = Writer::open(db).unwrap();
    writer
        .register_root(&RootInfo {
            idx: RootIdx(0),
            vol: VolInfo {
                guid: String::from("vol-1"),
                serial: 1,
                letter: Some(String::from("C:")),
                journal_id: None,
            },
            path: r"\\?\C:\watched".encode_utf16().collect(),
            recursive: true,
        })
        .unwrap();
    writer
}

fn stored(db: &Path) -> Vec<(i64, u64)> {
    let conn = Connection::open(db).unwrap();
    let mut statement = conn
        .prepare("SELECT id, ts FROM event ORDER BY id")
        .unwrap();
    statement
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))
        .unwrap()
        .map(|row| {
            let (id, ts) = row.unwrap();
            (id, u64::try_from(ts).unwrap())
        })
        .collect()
}

fn wait_for(what: &str, mut condition: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(120);
    while !condition() {
        assert!(Instant::now() < until, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Runs the whole story with the spill file in `spill_dir`.
fn full_volume(db: &Path, spill_dir: &Path, free_dir: &Path, spill_on_disk: bool) {
    let pipeline = Pipeline::start(open_writer(db), Config::new(spill_dir.to_path_buf())).unwrap();
    let producer = pipeline.producer();
    let mut next = 1;
    for _ in 0..5 {
        push(&producer, next);
        next += PER_BATCH;
    }
    pipeline.flush(Duration::from_secs(60)).unwrap();
    let committed = stored(db);
    assert_eq!(committed.len() as u64, next - 1);

    // The volume fills up. Every commit now fails.
    let filler = fill(free_dir);
    wait_for("the writer to report it is failing", || {
        push(&producer, next);
        next += PER_BATCH;
        pipeline.stats().failing
    });
    for _ in 0..30 {
        push(&producer, next);
        next += PER_BATCH;
    }
    assert!(pipeline.stats().commit_failures > 0);

    // Space is freed; the writer recovers by itself.
    std::fs::remove_file(&filler).unwrap();
    wait_for("the writer to recover", || !pipeline.stats().failing);
    pipeline.flush(Duration::from_secs(120)).unwrap();
    let stats = pipeline.stats();
    drop(pipeline.close());

    let after = stored(db);
    // What was committed before the volume filled up is untouched.
    assert_eq!(
        after.get(..committed.len()),
        Some(committed.as_slice()),
        "a committed row changed"
    );
    let pushed = next - 1;
    let rows = after.len() as u64;
    if spill_on_disk {
        // The spill file sits on the full volume too, so a batch that found no
        // room anywhere is lost, and counted. Nothing is lost silently.
        assert_eq!(
            rows + stats.lost_batches * PER_BATCH,
            pushed,
            "rows {rows}, lost batches {}, pushed {pushed}",
            stats.lost_batches
        );
    } else {
        assert_eq!(stats.lost_batches, 0);
        assert_eq!(rows, pushed, "everything pushed arrived");
    }
    // In order: ids rise with the push order.
    let ts: Vec<u64> = after.iter().map(|(_, ts)| *ts).collect();
    assert!(ts.windows(2).all(|pair| pair[0] < pair[1]), "out of order");
    let conn = Connection::open(db).unwrap();
    let check: String = conn
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap();
    assert_eq!(check, "ok");
}

#[test]
fn a_full_volume_makes_the_writer_fail_and_it_recovers_when_space_is_freed() {
    if !enabled() {
        return;
    }
    let disk = Disk::new("db");
    let elsewhere = std::env::temp_dir().join(format!("pbfull-spill-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&elsewhere);
    // The spill file is on another volume: nothing can be lost.
    full_volume(
        &disk.mount.join("segment.db"),
        &elsewhere,
        &disk.mount,
        false,
    );
    let _ = std::fs::remove_dir_all(&elsewhere);
}

#[test]
fn a_full_volume_with_the_spill_file_on_it_loses_only_what_it_counts() {
    if !enabled() {
        return;
    }
    let disk = Disk::new("spill");
    // The spill directory is beside the database: the default layout.
    full_volume(
        &disk.mount.join("segment.db"),
        &disk.mount.join("spill"),
        &disk.mount,
        true,
    );
}
