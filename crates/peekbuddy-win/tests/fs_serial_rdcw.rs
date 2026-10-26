//! The `ReadDirectoryChangesExW` backend against NTFS (#45).
//!
//! Every test stops and joins the watcher before its directory goes away.
#![cfg(windows)]
#![expect(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns may unwrap"
)]

use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use peekbuddy_core::capture::EventSink;
use peekbuddy_core::event::{Event, GapReason, Kind, RootIdx, VolIdx, WatchState};
use peekbuddy_core::notify::Class;
use peekbuddy_win::io::dir::DirHandle;
use peekbuddy_win::io::open::OpenOptions;
use peekbuddy_win::io::path::to_long_path;
use peekbuddy_win::io::volume::final_path;
use peekbuddy_win::rdcw::{Detail, LifecycleSink, RootInfo, RootSpec, Watcher};

const ROOT: RootIdx = RootIdx(7);

/// What the sink stores and how much room it has.
struct Ring {
    room: usize,
    events: Vec<(Event, String)>,
    /// While closed, the capture thread blocks inside the sink.
    open: bool,
}

#[derive(Clone)]
struct Probe(Arc<(Mutex<Ring>, Condvar)>);

impl Probe {
    fn new(room: usize) -> Self {
        Self(Arc::new((
            Mutex::new(Ring {
                room,
                events: Vec::new(),
                open: true,
            }),
            Condvar::new(),
        )))
    }

    fn with<R>(&self, f: impl FnOnce(&mut Ring) -> R) -> R {
        f(&mut self.0.0.lock().unwrap())
    }

    fn set_open(&self, open: bool) {
        self.with(|ring| ring.open = open);
        self.0.1.notify_all();
    }

    fn kinds(&self) -> Vec<(Kind, String)> {
        self.with(|ring| {
            ring.events
                .iter()
                .map(|(e, n)| (e.kind, n.clone()))
                .collect()
        })
    }

    fn names(&self, kind: Kind) -> Vec<String> {
        self.kinds()
            .into_iter()
            .filter(|(k, _)| *k == kind)
            .map(|(_, n)| n)
            .collect()
    }

    fn gaps(&self) -> usize {
        self.kinds()
            .iter()
            .filter(|(k, _)| matches!(k, Kind::Overflow(_)))
            .count()
    }
}

struct Sink(Probe);

impl EventSink for Sink {
    fn push(&mut self, event: &Event, name: &[u16]) -> bool {
        let (lock, wake) = &*(self.0).0;
        let mut ring = lock.lock().unwrap();
        while !ring.open {
            ring = wake.wait_timeout(ring, Duration::from_secs(30)).unwrap().0;
        }
        if ring.events.len() >= ring.room {
            return false;
        }
        ring.events.push((*event, String::from_utf16_lossy(name)));
        true
    }
}

#[derive(Default)]
struct Log {
    states: Vec<(WatchState, Detail)>,
    reconciles: Vec<GapReason>,
}

#[derive(Clone, Default)]
struct Life(Arc<Mutex<Log>>);

impl LifecycleSink for Life {
    fn state(&mut self, root: RootIdx, state: WatchState, detail: Detail) {
        assert_eq!(root, ROOT);
        self.0.lock().unwrap().states.push((state, detail));
    }

    fn reconcile(&mut self, root: RootIdx, reason: GapReason) {
        assert_eq!(root, ROOT);
        self.0.lock().unwrap().reconciles.push(reason);
    }
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().collect()
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let mut path = std::env::var_os("RUNNER_TEMP").map_or_else(std::env::temp_dir, Into::into);
        path.push(format!("peekbuddy-rdcw-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn spec(path: &Path, recursive: bool, buffer_bytes: usize) -> RootSpec {
    RootSpec {
        root: ROOT,
        vol: VolIdx(0),
        path: wide(path),
        recursive,
        last_access: false,
        buffer_bytes,
        exclude: Vec::new(),
        reject: Vec::new(),
    }
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn start(room: usize, spec: RootSpec) -> (Watcher, Probe, Life, RootInfo) {
    let probe = Probe::new(room);
    let life = Life::default();
    let watcher = Watcher::start(Sink(probe.clone()), life.clone()).unwrap();
    let info = watcher.add_root(spec).unwrap();
    (watcher, probe, life, info)
}

#[test]
fn files_and_directories_in_a_recursive_watch() {
    let tmp = Scratch::new("recursive");
    let root = &tmp.0;
    let (watcher, probe, life, info) = start(1000, spec(root, true, 256 * 1024));
    assert!(matches!(info.class, Class::Full | Class::Extended));
    assert_eq!(
        life.0.lock().unwrap().states.first().map(|s| s.0),
        Some(WatchState::Arming)
    );

    std::fs::create_dir(root.join("sub")).unwrap();
    std::fs::write(root.join("sub").join("a.txt"), b"one").unwrap();
    std::fs::write(root.join("sub").join("a.txt"), b"two!").unwrap();
    std::fs::rename(
        root.join("sub").join("a.txt"),
        root.join("sub").join("b.txt"),
    )
    .unwrap();
    std::fs::remove_file(root.join("sub").join("b.txt")).unwrap();
    std::fs::remove_dir(root.join("sub")).unwrap();

    wait_for("the removal of sub", || {
        probe.names(Kind::Deleted).iter().any(|n| n == "sub")
    });
    let created = probe.names(Kind::Created);
    assert_eq!(created, ["sub", "sub\\a.txt"]);
    assert_ne!(probe.names(Kind::Modified), Vec::<String>::new());
    assert_eq!(probe.names(Kind::RenamedFrom), ["sub\\a.txt"]);
    assert_eq!(probe.names(Kind::RenamedTo), ["sub\\b.txt"]);
    assert_eq!(probe.names(Kind::Deleted), ["sub\\b.txt", "sub"]);
    // The directory is reported as one.
    let sub_is_dir = probe.with(|ring| {
        ring.events
            .iter()
            .find(|(e, n)| e.kind == Kind::Created && n == "sub")
            .map(|(e, _)| e.is_dir)
    });
    assert_eq!(sub_is_dir, Some(true));
    // Order within a root is preserved.
    let order: Vec<Kind> = probe.kinds().into_iter().map(|(k, _)| k).collect();
    assert_eq!(order.first(), Some(&Kind::Created));
    assert_eq!(order.last(), Some(&Kind::Deleted));

    let metrics = watcher.stop();
    assert_eq!(metrics.counters.overflows, 0);
    assert_eq!(metrics.counters.dropped_full, 0);
    assert_eq!(
        metrics.counters.records,
        probe.with(|r| r.events.len()) as u64
    );
}

#[test]
fn a_non_recursive_watch_ignores_subdirectories() {
    let tmp = Scratch::new("flat");
    let root = &tmp.0;
    std::fs::create_dir(root.join("deep")).unwrap();
    let (watcher, probe, _life, _info) = start(1000, spec(root, false, 64 * 1024));

    std::fs::write(root.join("deep").join("hidden.txt"), b"x").unwrap();
    std::fs::write(root.join("top.txt"), b"x").unwrap();
    wait_for("top.txt", || {
        probe.names(Kind::Created).iter().any(|n| n == "top.txt")
    });
    let _ = watcher.stop();
    let all: Vec<String> = probe.kinds().into_iter().map(|(_, n)| n).collect();
    assert!(all.iter().all(|n| !n.contains("hidden")), "{all:?}");
}

#[test]
fn the_info_class_falls_back_when_the_file_system_refuses() {
    for (reject, expected) in [
        (vec![Class::Full], Class::Extended),
        (vec![Class::Full, Class::Extended], Class::Plain),
    ] {
        let tmp = Scratch::new("fallback");
        let mut s = spec(&tmp.0, true, 64 * 1024);
        s.reject = reject;
        let (watcher, probe, life, info) = start(100, s);
        assert_eq!(info.class, expected);
        assert_eq!(
            life.0.lock().unwrap().states.last().map(|s| s.1.class),
            Some(Some(expected))
        );
        std::fs::write(tmp.0.join("x.txt"), b"x").unwrap();
        wait_for("x.txt", || {
            probe.names(Kind::Created).iter().any(|n| n == "x.txt")
        });
        let _ = watcher.stop();
    }
}

#[test]
fn a_burst_behind_a_stalled_consumer_is_an_overflow_gap_and_a_reconcile() {
    let tmp = Scratch::new("overflow");
    let root = &tmp.0;
    let (watcher, probe, life, info) = start(1_000_000, spec(root, true, 64 * 1024));
    assert_eq!(info.buffer_bytes, 64 * 1024);

    // The first event parks the capture thread inside the sink; the re-armed
    // 64 KiB buffer then fills up while nobody reads it.
    probe.set_open(false);
    std::fs::write(root.join("first.txt"), b"x").unwrap();
    std::thread::sleep(Duration::from_millis(200));
    for i in 0..6000 {
        std::fs::write(root.join(format!("burst-{i:05}.txt")), b"x").unwrap();
    }
    probe.set_open(true);

    wait_for("a gap row", || probe.gaps() >= 1);
    let metrics = watcher.stop();
    let counters = metrics.counters;
    assert!(counters.overflows >= 1, "{counters:?}");
    // Every overflow became one stored gap row, and each one asked for a
    // reconcile.
    assert_eq!(counters.gap_rows, counters.gaps_owed_total);
    assert_eq!(probe.gaps() as u64, counters.gap_rows);
    assert_eq!(counters.overflows, counters.gap_rows);
    let log = life.0.lock().unwrap();
    assert_eq!(log.reconciles.len() as u64, counters.overflows);
    assert!(
        log.reconciles
            .iter()
            .all(|r| matches!(r, GapReason::BufferOverflow | GapReason::NotifyEnumDir))
    );
    assert_eq!(metrics.reconciles, counters.overflows);
}

#[test]
fn a_full_ring_is_a_counted_gap_and_a_reconcile_never_a_silent_drop() {
    let tmp = Scratch::new("ringfull");
    let root = &tmp.0;
    let (watcher, probe, life, _info) = start(3, spec(root, true, 256 * 1024));

    for i in 0..20 {
        std::fs::write(root.join(format!("f{i}.txt")), b"x").unwrap();
    }
    wait_for("the ring-full reconcile", || {
        life.0
            .lock()
            .unwrap()
            .reconciles
            .contains(&GapReason::RingFull)
    });
    let counters = watcher.metrics().counters;
    assert!(counters.dropped_full > 0, "{counters:?}");
    assert!(counters.gaps_owed_total >= 1);
    // The ring has no room even for the gap row yet.
    assert_eq!(probe.gaps(), 0);

    // Space frees up: the next batch pays the gap before newer events.
    probe.with(|ring| ring.room = 1000);
    std::fs::write(root.join("after.txt"), b"x").unwrap();
    wait_for("the gap row", || probe.gaps() >= 1);
    wait_for("after.txt", || {
        probe.names(Kind::Created).iter().any(|n| n == "after.txt")
    });
    let kinds = probe.kinds();
    let gap_at = kinds
        .iter()
        .position(|(k, _)| *k == Kind::Overflow(GapReason::RingFull))
        .unwrap();
    let after_at = kinds.iter().position(|(_, n)| n == "after.txt").unwrap();
    assert!(gap_at < after_at, "{kinds:?}");
    let metrics = watcher.stop();
    assert_eq!(metrics.counters.gap_rows, metrics.counters.gaps_owed_total);
}

#[test]
fn the_data_directory_is_dropped_before_the_ring_and_counted() {
    let tmp = Scratch::new("self");
    let root = &tmp.0;
    let data = root.join("PeekBuddy-data");
    std::fs::create_dir(&data).unwrap();
    let canonical = {
        let long = to_long_path(&wide(&data)).unwrap();
        final_path(&DirHandle::open(&long, OpenOptions::default()).unwrap()).unwrap()
    };
    let mut s = spec(root, true, 64 * 1024);
    s.exclude = vec![canonical];
    let (watcher, probe, _life, _info) = start(1000, s);

    std::fs::write(data.join("segment.db"), b"x").unwrap();
    std::fs::write(root.join("user.txt"), b"x").unwrap();
    wait_for("user.txt", || {
        probe.names(Kind::Created).iter().any(|n| n == "user.txt")
    });
    let metrics = watcher.stop();
    let all: Vec<String> = probe.kinds().into_iter().map(|(_, n)| n).collect();
    assert!(all.iter().all(|n| !n.contains("PeekBuddy-data")), "{all:?}");
    assert!(metrics.counters.excluded >= 1, "{:?}", metrics.counters);
}

#[test]
fn deleting_the_watched_root_is_a_lost_state() {
    let tmp = Scratch::new("lost");
    let root = tmp.0.join("watched");
    std::fs::create_dir(&root).unwrap();
    let (watcher, _probe, life, _info) = start(1000, spec(&root, true, 64 * 1024));

    std::fs::remove_dir_all(&root).unwrap();
    wait_for("Lost", || {
        life.0
            .lock()
            .unwrap()
            .states
            .iter()
            .any(|s| s.0 == WatchState::Lost)
    });
    let lost = life
        .0
        .lock()
        .unwrap()
        .states
        .iter()
        .find(|s| s.0 == WatchState::Lost)
        .copied()
        .unwrap();
    assert!(lost.1.error.is_some());
    let _ = watcher.stop();
}

#[test]
fn stopping_releases_the_directory() {
    let tmp = Scratch::new("stop");
    let root = tmp.0.join("watched");
    std::fs::create_dir(&root).unwrap();
    let (watcher, _probe, _life, _info) = start(1000, spec(&root, true, 64 * 1024));
    let _ = watcher.stop();
    // A directory with a watch handle still open could not be renamed.
    std::fs::rename(&root, tmp.0.join("renamed")).unwrap();
    std::fs::remove_dir_all(tmp.0.join("renamed")).unwrap();
}

#[test]
fn a_root_can_be_added_and_removed_while_running() {
    let tmp = Scratch::new("dynamic");
    let first = tmp.0.join("first");
    std::fs::create_dir(&first).unwrap();
    let (watcher, probe, _life, _info) = start(1000, spec(&first, true, 64 * 1024));
    watcher.remove_root(ROOT).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    std::fs::write(first.join("late.txt"), b"x").unwrap();
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(probe.names(Kind::Created), Vec::<String>::new());
    // The slot of the removed root is free again.
    watcher.add_root(spec(&first, true, 64 * 1024)).unwrap();
    std::fs::write(first.join("again.txt"), b"x").unwrap();
    wait_for("again.txt", || {
        probe.names(Kind::Created).iter().any(|n| n == "again.txt")
    });
    let _ = watcher.stop();
}

#[test]
fn a_paused_watch_stores_nothing_and_counts_every_event() {
    use peekbuddy_core::pause::{PausableSink, Pause};

    let tmp = Scratch::new("paused");
    let root = &tmp.0;
    let probe = Probe::new(1000);
    let pause = Arc::new(Pause::new());
    let watcher = Watcher::start(
        PausableSink::new(Sink(probe.clone()), Arc::clone(&pause)),
        Life::default(),
    )
    .unwrap();
    watcher.add_root(spec(root, true, 256 * 1024)).unwrap();

    let first = pause.set_watch(ROOT, true).unwrap();
    assert_eq!(first.state, WatchState::SuspendedByUser);
    for i in 0..50 {
        std::fs::write(root.join(format!("p{i}.txt")), b"x").unwrap();
    }
    // Every record the backend decoded was counted by the pause, none stored.
    wait_for("the paused records", || {
        let records = watcher.metrics().counters.records;
        records >= 50 && pause.missed(ROOT) == records
    });
    assert_eq!(probe.kinds(), []);

    // Resume: one row carries the count, and capture continues.
    let resumed = pause.set_watch(ROOT, false).unwrap();
    assert_eq!(resumed.state, WatchState::Live);
    assert_eq!(resumed.missed, watcher.metrics().counters.records);
    assert!(!resumed.lower_bound);
    std::fs::write(root.join("live.txt"), b"x").unwrap();
    wait_for("live.txt", || {
        probe.names(Kind::Created).iter().any(|n| n == "live.txt")
    });
    let _ = watcher.stop();
}
