//! The reconcile worker on a real NTFS tree (#46): offline changes found at
//! the next start, cancel, links, and coalesced triggers.
#![cfg(windows)]
#![expect(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns may unwrap"
)]

use std::collections::BTreeMap;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use peekbuddy_core::event::{Event, GapReason, Kind, RootIdx, VolIdx};
use peekbuddy_core::reconcile::RowOut;
use peekbuddy_engine::reconciler::{NoObserver, Reconciler, RootSpec};
use peekbuddy_store::snapshot::SnapshotDb;
use peekbuddy_win::io::path::to_long_path;

const ROOT: RootIdx = RootIdx(5);

fn clock() -> (u64, u64) {
    (1, 1)
}

struct Gate {
    open: Mutex<bool>,
    wake: Condvar,
}

impl Gate {
    fn new(open: bool) -> Arc<Self> {
        Arc::new(Self {
            open: Mutex::new(open),
            wake: Condvar::new(),
        })
    }

    fn set(&self, open: bool) {
        *self.open.lock().unwrap() = open;
        self.wake.notify_all();
    }
}

#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<(Kind, String)>>>);

impl Log {
    fn take(&self) -> Vec<(Kind, String)> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }

    fn differences(&self) -> Vec<(Kind, String)> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|(kind, _)| {
                matches!(
                    kind,
                    Kind::ReconciledCreated
                        | Kind::ReconciledModified
                        | Kind::ReconciledDeleted
                        | Kind::ReconciledRenamed
                        | Kind::AccessDenied
                )
            })
            .cloned()
            .collect()
    }

    fn kinds(&self) -> Vec<Kind> {
        self.0.lock().unwrap().iter().map(|(k, _)| *k).collect()
    }
}

/// Rows into a log; the first row of a walk waits for the gate.
struct Rows {
    log: Log,
    gate: Arc<Gate>,
}

impl RowOut for Rows {
    fn row(&mut self, event: &Event, name: &[u16]) {
        if event.kind == Kind::ReconcileStarted {
            let mut open = self.gate.open.lock().unwrap();
            while !*open {
                open = self
                    .gate
                    .wake
                    .wait_timeout(open, Duration::from_secs(30))
                    .unwrap()
                    .0;
            }
        }
        self.log
            .0
            .lock()
            .unwrap()
            .push((event.kind, String::from_utf16_lossy(name)));
    }

    fn flush(&mut self) -> bool {
        true
    }
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().collect()
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let mut path = std::env::var_os("RUNNER_TEMP").map_or_else(std::env::temp_dir, Into::into);
        path.push(format!("peekbuddy-reconcile-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn tree(&self) -> PathBuf {
        let tree = self.0.join("tree");
        std::fs::create_dir_all(&tree).unwrap();
        tree
    }

    fn db(&self) -> PathBuf {
        self.0.join("snapshot.db")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn spec(tree: &Path) -> RootSpec {
    RootSpec {
        root: ROOT,
        vol: VolIdx(0),
        path: to_long_path(&wide(tree)).unwrap(),
    }
}

fn start(scratch: &Scratch, gate: &Arc<Gate>) -> (Reconciler, Log) {
    let log = Log::default();
    let reconciler = Reconciler::start(
        SnapshotDb::open(&scratch.db()).unwrap(),
        Rows {
            log: log.clone(),
            gate: Arc::clone(gate),
        },
        vec![spec(&scratch.tree())],
        clock,
        Arc::new(NoObserver),
    )
    .unwrap();
    (reconciler, log)
}

/// The real tree as `relative path -> (is dir, size)`.
fn real_tree(root: &Path) -> BTreeMap<Vec<u16>, (bool, u64)> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<Vec<u16>, (bool, u64)>) {
        let Ok(read) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in read.flatten() {
            let path = entry.path();
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            let rel = path.strip_prefix(base).unwrap();
            out.insert(
                wide(rel),
                (meta.is_dir(), if meta.is_dir() { 0 } else { meta.len() }),
            );
            if meta.is_dir() && !meta.file_type().is_symlink() {
                walk(base, &path, out);
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

fn snapshot_tree(db: &Path) -> BTreeMap<Vec<u16>, (bool, u64)> {
    SnapshotDb::open(db)
        .unwrap()
        .paths(ROOT.0)
        .unwrap()
        .into_iter()
        .map(|(path, meta)| {
            (
                path,
                (meta.is_dir(), if meta.is_dir() { 0 } else { meta.size }),
            )
        })
        .collect()
}

/// A small deterministic generator.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % u64::try_from(n.max(1)).unwrap()).unwrap()
    }
}

fn scan(tree: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let (mut dirs, mut files) = (vec![tree.to_path_buf()], Vec::new());
    let mut at = 0;
    while let Some(dir) = dirs.get(at).cloned() {
        at += 1;
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                dirs.push(path);
            } else {
                files.push(path);
            }
        }
    }
    (dirs, files)
}

fn random_ops(tree: &Path, count: usize, seed: u64) {
    let mut rng = Rng(seed);
    let (mut dirs, mut files) = scan(tree);
    for step in 0..count {
        if step % 25 == 0 {
            (dirs, files) = scan(tree);
        }
        let unique = format!("n{step}");
        match rng.below(8) {
            0 | 1 => {
                let dir = dirs[rng.below(dirs.len())].clone();
                let _ = std::fs::write(dir.join(&unique), vec![b'x'; rng.below(50) + 1]);
            }
            2 if !files.is_empty() => {
                let _ = std::fs::remove_file(&files[rng.below(files.len())]);
            }
            3 | 4 if !files.is_empty() => {
                let file = &files[rng.below(files.len())];
                let _ = std::fs::OpenOptions::new()
                    .append(true)
                    .open(file)
                    .and_then(|mut f| std::io::Write::write_all(&mut f, b"more"));
            }
            5 if !files.is_empty() => {
                let file = files[rng.below(files.len())].clone();
                let _ = std::fs::rename(&file, file.with_file_name(&unique));
            }
            6 if !files.is_empty() => {
                let file = files[rng.below(files.len())].clone();
                let dir = dirs[rng.below(dirs.len())].clone();
                let _ = std::fs::rename(&file, dir.join(&unique));
            }
            7 => {
                let dir = dirs[rng.below(dirs.len())].clone();
                if rng.below(3) == 0 && dir != tree {
                    let _ = std::fs::remove_dir_all(&dir);
                } else {
                    let _ = std::fs::create_dir(dir.join(&unique));
                }
            }
            _ => {}
        }
    }
}

#[test]
fn offline_changes_are_found_at_the_next_start() {
    let scratch = Scratch::new("offline");
    let tree = scratch.tree();
    for d in 0..20 {
        let dir = tree.join(format!("dir{d}"));
        std::fs::create_dir_all(dir.join("inner")).unwrap();
        for f in 0..15 {
            std::fs::write(dir.join(format!("file{f}.txt")), vec![b'a'; f + 1]).unwrap();
            std::fs::write(dir.join("inner").join(format!("deep{f}")), b"deep").unwrap();
        }
    }
    let open = Gate::new(true);

    // First start: the baseline.
    let (mut reconciler, log) = start(&scratch, &open);
    reconciler.trigger(ROOT, None);
    assert!(reconciler.wait_idle(Duration::from_secs(60)));
    assert_eq!(log.kinds(), [Kind::BaselineEstablished]);
    reconciler.stop();
    assert_eq!(snapshot_tree(&scratch.db()), real_tree(&tree));

    // The engine is stopped while 5,000 things happen.
    random_ops(&tree, 5000, 0x9E37_79B9_7F4A_7C15);
    assert_ne!(snapshot_tree(&scratch.db()), real_tree(&tree));

    // Restart: a reconcile explains the difference and the snapshot catches up.
    let (mut reconciler, log) = start(&scratch, &open);
    reconciler.trigger(ROOT, None);
    assert!(reconciler.wait_idle(Duration::from_secs(120)));
    let stats = reconciler.stats();
    let last = stats.last.unwrap();
    assert!(!last.cancelled && !last.baseline, "{last:?}");
    assert!(last.changed > 100, "{last:?}");
    assert_eq!(log.kinds().first(), Some(&Kind::ReconcileStarted));
    assert_eq!(log.kinds().last(), Some(&Kind::ReconcileFinished));
    reconciler.stop();
    assert_eq!(snapshot_tree(&scratch.db()), real_tree(&tree));

    // And a third start finds nothing left.
    let (mut reconciler, log) = start(&scratch, &open);
    reconciler.trigger(ROOT, None);
    assert!(reconciler.wait_idle(Duration::from_secs(60)));
    assert_eq!(log.differences(), []);
    reconciler.stop();
}

#[test]
fn a_cancel_leaves_the_previous_snapshot_intact() {
    let scratch = Scratch::new("cancel");
    let tree = scratch.tree();
    for d in 0..400 {
        let dir = tree.join(format!("d{d}"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("f"), b"x").unwrap();
    }
    let open = Gate::new(true);
    let (mut reconciler, _) = start(&scratch, &open);
    reconciler.trigger(ROOT, None);
    assert!(reconciler.wait_idle(Duration::from_secs(60)));
    reconciler.stop();
    let before = snapshot_tree(&scratch.db());

    for d in 0..400 {
        std::fs::write(tree.join(format!("d{d}")).join("new"), b"y").unwrap();
    }
    let gate = Gate::new(false);
    let (mut reconciler, log) = start(&scratch, &gate);
    reconciler.trigger(ROOT, Some(GapReason::BufferOverflow));
    // The walk is parked on its first row; cancel, then let it go.
    std::thread::sleep(Duration::from_millis(300));
    reconciler.cancel_current();
    gate.set(true);
    assert!(reconciler.wait_idle(Duration::from_secs(60)));
    let last = reconciler.stats().last.unwrap();
    assert!(last.cancelled, "{last:?}");
    assert!(last.scanned < 400, "it stopped early: {last:?}");
    assert_eq!(log.kinds().last(), Some(&Kind::ReconcileFinished));
    reconciler.stop();
    assert_eq!(snapshot_tree(&scratch.db()), before);

    // The next run, not cancelled, catches up.
    let (mut reconciler, _) = start(&scratch, &Gate::new(true));
    reconciler.trigger(ROOT, None);
    assert!(reconciler.wait_idle(Duration::from_secs(60)));
    reconciler.stop();
    assert_eq!(snapshot_tree(&scratch.db()), real_tree(&tree));
}

#[test]
fn a_junction_is_logged_but_not_traversed() {
    let scratch = Scratch::new("junction");
    let tree = scratch.tree();
    let outside = scratch.0.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.txt"), b"x").unwrap();
    std::fs::write(tree.join("plain.txt"), b"x").unwrap();
    let open = Gate::new(true);

    let (mut reconciler, _) = start(&scratch, &open);
    reconciler.trigger(ROOT, None);
    assert!(reconciler.wait_idle(Duration::from_secs(30)));
    reconciler.stop();

    let status = std::process::Command::new("cmd.exe")
        .args(["/C", "mklink", "/J"])
        .arg(tree.join("link"))
        .arg(&outside)
        .output()
        .unwrap();
    assert!(status.status.success(), "{status:?}");

    let (mut reconciler, log) = start(&scratch, &open);
    reconciler.trigger(ROOT, None);
    assert!(reconciler.wait_idle(Duration::from_secs(30)));
    reconciler.stop();
    assert_eq!(
        log.differences(),
        [(Kind::ReconciledCreated, String::from("link"))]
    );
    let known = snapshot_tree(&scratch.db());
    assert!(
        known
            .keys()
            .all(|path| { !String::from_utf16_lossy(path).contains("secret.txt") })
    );
    assert!(known.contains_key(&wide(Path::new("link"))));
    // Remove the junction itself, not its target.
    std::fs::remove_dir(tree.join("link")).unwrap();
}

#[test]
fn triggers_during_a_walk_become_one_follow_up() {
    let scratch = Scratch::new("coalesce");
    let tree = scratch.tree();
    std::fs::write(tree.join("a.txt"), b"x").unwrap();
    let open = Gate::new(true);
    let (mut reconciler, _) = start(&scratch, &open);
    reconciler.trigger(ROOT, None);
    assert!(reconciler.wait_idle(Duration::from_secs(30)));
    reconciler.stop();

    let gate = Gate::new(false);
    let (mut reconciler, log) = start(&scratch, &gate);
    reconciler.trigger(ROOT, Some(GapReason::BufferOverflow));
    std::thread::sleep(Duration::from_millis(200));
    for _ in 0..4 {
        reconciler.trigger(ROOT, Some(GapReason::RingFull));
    }
    std::fs::write(tree.join("b.txt"), b"x").unwrap();
    gate.set(true);
    assert!(reconciler.wait_idle(Duration::from_secs(30)));
    let stats = reconciler.stats();
    assert_eq!((stats.runs, stats.coalesced), (2, 3), "{stats:?}");
    // The follow-up is where b.txt shows up.
    assert!(
        log.differences()
            .contains(&(Kind::ReconciledCreated, String::from("b.txt")))
            || log
                .take()
                .iter()
                .any(|(k, n)| *k == Kind::ReconciledCreated && n == "b.txt")
    );
    reconciler.stop();
}
