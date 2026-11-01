//! The USN source on a disk of its own, where the journal can be deleted,
//! recreated and wrapped (#214, #212).
//!
//! Destructive: it creates a virtual disk with diskpart, mounts it on a
//! folder and changes the journal of that disk with `fsutil`. It needs an
//! elevated token, so it only runs when `PEEKBUDDY_VHD_TESTS=1` is set (the
//! nightly `vhd` job). The journals of the system volumes are never touched.
#![cfg(all(windows, feature = "usn"))]
#![expect(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns may unwrap"
)]

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use peekbuddy_core::capture::EventSink;
use peekbuddy_core::event::{Event, GapReason, Kind, RootIdx, VolIdx};
use peekbuddy_core::usn::checkpoint::Checkpoint;
use peekbuddy_engine::pause_ctl::now_ticks;
use peekbuddy_engine::usn_source::{Checkpoints, OnGap, UsnRunner, UsnSpec};

fn enabled() -> bool {
    std::env::var_os("PEEKBUDDY_VHD_TESTS").is_some() && peekbuddy_win::token::is_admin().unwrap()
}

/// Runs a program and returns its output, failure included (with the exit
/// status) so that a nightly failure says why.
fn run(program: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if output.status.success() {
        Ok(text)
    } else {
        Err(format!("{program} {args:?}: {}: {text}", output.status))
    }
}

fn diskpart(script: &str) -> Result<String, String> {
    let path = std::env::temp_dir().join(format!("pbusn-{}.txt", std::process::id()));
    std::fs::File::create(&path)
        .unwrap()
        .write_all(script.as_bytes())
        .unwrap();
    let result = run("diskpart.exe", &["/s", path.to_str().unwrap()]);
    let _ = std::fs::remove_file(&path);
    result
}

/// A virtual disk mounted on a folder, detached and removed on drop.
struct Disk {
    vhd: PathBuf,
    mount: PathBuf,
    base: PathBuf,
    /// `fsutil usn` wants a drive letter, not a mount folder.
    letter: char,
}

impl Disk {
    /// `None` (after saying why on stdout) when the file system cannot be
    /// put on a disk of this runner; every other failure fails the test.
    fn new(tag: &str, letter: char, fs: &str, megabytes: u32) -> Option<Self> {
        let base = std::env::temp_dir().join(format!("pbusn-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(base.join("mount")).unwrap();
        // The runner's temp directory is an 8.3 path: diskpart wants long
        // names.
        let base = long_path(&base);
        let disk = Self {
            vhd: base.join("disk.vhdx"),
            mount: base.join("mount"),
            base,
            letter,
        };
        let script = format!(
            "create vdisk file=\"{}\" maximum={megabytes} type=expandable\nattach vdisk\ncreate partition primary\nformat fs={fs} quick label=pbusn\nassign mount=\"{}\"\nassign letter={}\n",
            disk.vhd.display(),
            disk.mount.display(),
            disk.letter
        );
        if let Err(why) = diskpart(&script) {
            assert_ne!(fs, "ntfs", "diskpart failed: {why}");
            // A passing test shows no output, so the reason goes to a file
            // the workflow turns into a warning annotation (never a silent
            // pass).
            if let Some(notes) = std::env::var_os("PEEKBUDDY_VHD_NOTES") {
                let mut file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(notes)
                    .unwrap();
                let why = why.replace(['\r', '\n'], " ");
                writeln!(file, "{fs} could not be put on a vdisk: {why}").unwrap();
            }
            return None;
        }
        assert_ne!(
            volume_of(&disk.mount),
            volume_of(&disk.base),
            "the mount folder is not on the virtual disk"
        );
        // #228: one volume reached by drive letter and by mount folder is the
        // same volume.
        let by_letter = identity_of(Path::new(&format!("{letter}:\\")));
        let by_mount = identity_of(&disk.mount);
        assert_eq!(by_letter.guid_path, by_mount.guid_path);
        assert_eq!(by_letter.serial, by_mount.serial);
        Some(disk)
    }

    /// The root of the disk by its drive letter. The mount folder cannot be
    /// the watched path: opened as a link it is a folder of the host volume,
    /// and its file id means nothing on the disk.
    fn root(&self) -> PathBuf {
        PathBuf::from(format!("{}:\\", self.letter))
    }

    fn mount_arg(&self) -> String {
        format!("{}:", self.letter)
    }

    fn create_journal(&self, max: &str, delta: &str) {
        let (m, a) = (format!("m={max}"), format!("a={delta}"));
        run(
            "fsutil.exe",
            &["usn", "createjournal", &m, &a, &self.mount_arg()],
        )
        .unwrap();
    }

    fn delete_journal(&self) {
        run(
            "fsutil.exe",
            &["usn", "deletejournal", "/d", &self.mount_arg()],
        )
        .unwrap();
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

fn long_path(path: &Path) -> PathBuf {
    let canonical = std::fs::canonicalize(path).unwrap();
    let text = canonical.to_string_lossy();
    PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(&text))
}

/// The identity of the volume a directory lives on.
fn identity_of(path: &Path) -> peekbuddy_win::io::volume::VolumeIdentity {
    use std::os::windows::ffi::OsStrExt as _;

    use peekbuddy_win::io::dir::DirHandle;
    use peekbuddy_win::io::open::OpenOptions;
    use peekbuddy_win::io::path::to_long_path;
    use peekbuddy_win::io::volume::volume_identity;

    let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    let long = to_long_path(&wide).unwrap();
    let dir = DirHandle::open(&long, OpenOptions::default()).unwrap();
    volume_identity(&dir, &long).unwrap()
}

/// The volume GUID path a directory lives on.
fn volume_of(path: &Path) -> Vec<u16> {
    identity_of(path).guid_path
}

type Rows = Arc<Mutex<Vec<(Event, String)>>>;

struct Collect(Rows);

impl EventSink for Collect {
    fn push(&mut self, event: &Event, name: &[u16]) -> bool {
        self.0
            .lock()
            .unwrap()
            .push((*event, String::from_utf16_lossy(name)));
        true
    }
}

#[derive(Clone, Default)]
struct Saved(Arc<Mutex<Option<Checkpoint>>>);

impl Checkpoints for Saved {
    fn load(&self) -> Option<Checkpoint> {
        *self.0.lock().unwrap()
    }

    fn save(&self, checkpoint: Checkpoint) {
        *self.0.lock().unwrap() = Some(checkpoint);
    }
}

struct Started {
    runner: UsnRunner,
    rows: Rows,
    gaps: Arc<Mutex<Vec<GapReason>>>,
}

fn start(dir: &Path, saved: &Saved) -> Started {
    let rows: Rows = Arc::default();
    let gaps: Arc<Mutex<Vec<GapReason>>> = Arc::default();
    let on_gap: OnGap = {
        let gaps = Arc::clone(&gaps);
        Box::new(move |reason| gaps.lock().unwrap().push(reason))
    };
    let spec = UsnSpec {
        vol: VolIdx(1),
        root: RootIdx(2),
        path: dir,
    };
    let started = UsnRunner::start(
        &spec,
        Box::new(saved.clone()),
        Box::new(Collect(Arc::clone(&rows))),
        on_gap,
        now_ticks,
    );
    let runner = started
        .map_err(|error| format!("start failed: {error}; {}", diagnose(dir)))
        .unwrap();
    Started { runner, rows, gaps }
}

/// Replays the setup of the runner step by step and says which one fails.
fn diagnose(dir: &Path) -> String {
    use std::os::windows::ffi::OsStrExt as _;

    use peekbuddy_win::io::dir::DirHandle;
    use peekbuddy_win::io::open::OpenOptions;
    use peekbuddy_win::io::path::to_long_path;
    use peekbuddy_win::io::volume::volume_identity;
    use peekbuddy_win::usn::{DEFAULT_BUFFER, Journal};

    let wide: Vec<u16> = dir.as_os_str().encode_wide().collect();
    let long = match to_long_path(&wide) {
        Ok(long) => long,
        Err(error) => return format!("to_long_path: {error:?}"),
    };
    let handle = match DirHandle::open(&long, OpenOptions::default()) {
        Ok(handle) => handle,
        Err(error) => return format!("open the root: {error}"),
    };
    let identity = match volume_identity(&handle, &long) {
        Ok(identity) => identity,
        Err(error) => return format!("volume_identity: {error}"),
    };
    let frn = match peekbuddy_win::fs::file_id(dir) {
        Ok(frn) => frn,
        Err(error) => return format!("file_id: {error}"),
    };
    let mut journal = match Journal::open(&identity.guid_path, DEFAULT_BUFFER) {
        Ok(journal) => journal,
        Err(error) => return format!("Journal::open: {error}"),
    };
    let query = match journal.query() {
        Ok(data) => format!("{data:?}"),
        Err(error) => return format!("query: {error}"),
    };
    let mut out = vec![0_u16; 4096];
    match journal.resolve_path(frn, &mut out) {
        Ok(len) => format!("all steps work: {query}; root path {len} units"),
        Err(error) => format!("resolve_path of frn {frn:#x}: {error}; {query}"),
    }
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn has_row(rows: &Rows, name: &str, kind: Kind) -> bool {
    rows.lock()
        .unwrap()
        .iter()
        .any(|(event, n)| n == name && event.kind == kind)
}

fn gap_rows(rows: &Rows) -> Vec<GapReason> {
    rows.lock()
        .unwrap()
        .iter()
        .filter_map(|(event, _)| match event.kind {
            Kind::Overflow(reason) => Some(reason),
            _ => None,
        })
        .collect()
}

#[test]
fn a_deleted_and_recreated_journal_is_one_gap_and_reading_goes_on() {
    if !enabled() {
        return;
    }
    let Some(disk) = Disk::new("recreate", 'U', "ntfs", 128) else {
        return;
    };
    disk.create_journal("0x100000", "0x40000");
    let saved = Saved::default();
    let mut started = start(&disk.root(), &saved);
    std::fs::write(disk.root().join("before.txt"), b"x").unwrap();
    wait_until("the first file", || {
        has_row(&started.rows, "before.txt", Kind::Created)
    });
    assert_eq!(gap_rows(&started.rows), []);

    // The journal goes away under a pending read and comes back with a new
    // id.
    let old_id = saved.load().unwrap().journal_id;
    disk.delete_journal();
    disk.create_journal("0x100000", "0x40000");
    wait_until("the gap", || !gap_rows(&started.rows).is_empty());
    // What happens between the gap and the runner reading the new journal is
    // the gap's to cover (a reconcile); only a file written after the runner
    // saved the new journal id must arrive.
    wait_until("the new journal id", || {
        saved.load().is_some_and(|c| c.journal_id != old_id)
    });
    std::fs::write(disk.root().join("after.txt"), b"x").unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while !has_row(&started.rows, "after.txt", Kind::Created) {
        if Instant::now() >= deadline {
            let rows: Vec<String> = started
                .rows
                .lock()
                .unwrap()
                .iter()
                .map(|(event, name)| format!("{:?} {name}", event.kind))
                .collect();
            let ended = started.runner.stop();
            let report = format!(
                "no row after the gap; the runner ended with {ended:?}; gaps {:?}; counters {:?}; saved {:?}; rows {rows:?}",
                started.gaps.lock().unwrap(),
                started.runner.counters(),
                saved.load()
            );
            assert_eq!(report, "", "the file after the gap");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(gap_rows(&started.rows), [GapReason::JournalIdChanged]);
    assert_eq!(*started.gaps.lock().unwrap(), [GapReason::JournalIdChanged]);
    assert!(started.runner.stop().is_none());
}

#[test]
fn a_journal_recreated_while_stopped_is_one_gap_at_the_next_start() {
    if !enabled() {
        return;
    }
    let Some(disk) = Disk::new("idchange", 'V', "ntfs", 128) else {
        return;
    };
    disk.create_journal("0x100000", "0x40000");
    let saved = Saved::default();
    let mut first = start(&disk.root(), &saved);
    std::fs::write(disk.root().join("one.txt"), b"x").unwrap();
    wait_until("the first file", || {
        has_row(&first.rows, "one.txt", Kind::Created)
    });
    assert!(first.runner.stop().is_none());

    disk.delete_journal();
    disk.create_journal("0x100000", "0x40000");

    let mut second = start(&disk.root(), &saved);
    assert_eq!(gap_rows(&second.rows), [GapReason::JournalIdChanged]);
    std::fs::write(disk.root().join("two.txt"), b"x").unwrap();
    wait_until("the file after the gap", || {
        has_row(&second.rows, "two.txt", Kind::Created)
    });
    assert_eq!(gap_rows(&second.rows).len(), 1);
    assert!(second.runner.stop().is_none());
}

#[test]
fn a_journal_that_wrapped_while_stopped_is_one_entry_deleted_gap() {
    if !enabled() {
        return;
    }
    let Some(disk) = Disk::new("wrap", 'W', "ntfs", 256) else {
        return;
    };
    // The smallest journal NTFS accepts, so a few thousand files push the
    // saved position out of it.
    disk.create_journal("0x100000", "0x40000");
    let saved = Saved::default();
    let mut first = start(&disk.root(), &saved);
    std::fs::write(disk.root().join("old.txt"), b"x").unwrap();
    wait_until("the first file", || {
        has_row(&first.rows, "old.txt", Kind::Created)
    });
    assert!(first.runner.stop().is_none());

    for index in 0..30_000 {
        std::fs::write(disk.root().join(format!("wrap-{index}.txt")), b"x").unwrap();
    }

    let mut second = start(&disk.root(), &saved);
    assert_eq!(gap_rows(&second.rows), [GapReason::JournalEntryDeleted]);
    assert_eq!(
        *second.gaps.lock().unwrap(),
        [GapReason::JournalEntryDeleted]
    );
    std::fs::write(disk.root().join("fresh.txt"), b"x").unwrap();
    wait_until("the file after the gap", || {
        has_row(&second.rows, "fresh.txt", Kind::Created)
    });
    assert!(second.runner.stop().is_none());
}

#[test]
fn refs_reads_fall_back_to_the_synchronous_reader_and_still_deliver() {
    if !enabled() {
        return;
    }
    // ReFS wants a larger disk than NTFS; a runner without ReFS says so and
    // skips (the warning is the record that this ran nothing).
    let Some(disk) = Disk::new("refs", 'X', "refs", 4096) else {
        return;
    };
    // ReFS creates its journal when asked; ignore "already there".
    let _ = run(
        "fsutil.exe",
        &[
            "usn",
            "createjournal",
            "m=0x1000000",
            "a=0x400000",
            &disk.mount_arg(),
        ],
    );
    let saved = Saved::default();
    let mut started = start(&disk.root(), &saved);
    std::fs::create_dir_all(disk.root().join("dir")).unwrap();
    std::fs::write(disk.root().join("dir").join("refs.txt"), b"x").unwrap();
    wait_until("a file on ReFS", || {
        has_row(&started.rows, r"dir\refs.txt", Kind::Created)
    });
    assert_eq!(gap_rows(&started.rows), []);
    assert!(started.runner.stop().is_none());
}
