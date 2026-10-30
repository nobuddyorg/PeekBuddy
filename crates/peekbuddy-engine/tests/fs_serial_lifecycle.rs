//! The capture host on a real NTFS tree (#48): lost and restored roots,
//! injected faults, and signals that never block.
#![cfg(windows)]
#![expect(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns may unwrap"
)]

use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use peekbuddy_core::capture::EventSink;
use peekbuddy_core::event::{Event, Kind, RootIdx, VolIdx, WatchState};
use peekbuddy_core::lifecycle::Signal;
use peekbuddy_core::pause::Pause;
use peekbuddy_core::perf::Milestone;
use peekbuddy_core::reconcile::RowOut;
use peekbuddy_engine::capture_host::{
    Host, HostParts, Services, Signaller, WatchSpec, notifier_factory, watch_system,
};
use peekbuddy_engine::pause_ctl::RowSink;
use peekbuddy_engine::reconciler::{Reconciler, RootSpec};
use peekbuddy_store::snapshot::SnapshotDb;
use peekbuddy_win::io::path::to_long_path;
use peekbuddy_win::probe::{Probe, probe_root};

const ROOT: RootIdx = RootIdx(0);

fn clock() -> (u64, u64) {
    (1, 1)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    kind: Kind,
    name: String,
    raw_mask: u32,
}

#[derive(Clone, Default)]
struct Log {
    rows: Arc<Mutex<Vec<Row>>>,
    gate: Option<Arc<Gate>>,
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

    fn wait(&self) {
        let mut open = self.open.lock().unwrap();
        while !*open {
            open = self
                .wake
                .wait_timeout(open, Duration::from_secs(30))
                .unwrap()
                .0;
        }
    }
}

impl Log {
    fn add(&self, event: &Event, name: &[u16]) {
        self.rows.lock().unwrap().push(Row {
            kind: event.kind,
            name: String::from_utf16_lossy(name),
            raw_mask: event.raw_mask,
        });
    }

    /// `(state, reason code)` of every state row.
    fn states(&self) -> Vec<(WatchState, u16)> {
        self.rows
            .lock()
            .unwrap()
            .iter()
            .filter_map(|row| match row.kind {
                Kind::WatchStateChanged(state) => {
                    Some((state, u16::try_from(row.raw_mask & 0xFFFF).unwrap()))
                }
                _ => None,
            })
            .collect()
    }

    fn has_state(&self, state: WatchState, reason: u16) -> bool {
        self.states().contains(&(state, reason))
    }

    fn names(&self, kind: Kind) -> Vec<String> {
        self.rows
            .lock()
            .unwrap()
            .iter()
            .filter(|row| row.kind == kind)
            .map(|row| row.name.clone())
            .collect()
    }
}

struct LogSink(Log);

impl EventSink for LogSink {
    fn push(&mut self, event: &Event, name: &[u16]) -> bool {
        self.0.add(event, name);
        true
    }
}

struct LogRows(Log);

impl RowSink for LogRows {
    fn row(&mut self, event: Event) {
        if let Some(gate) = &self.0.gate {
            gate.wait();
        }
        self.0.add(&event, &[]);
    }
}

impl RowOut for LogRows {
    fn row(&mut self, event: &Event, name: &[u16]) {
        self.0.add(event, name);
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
        path.push(format!("peekbuddy-lifecycle-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(path.join("tree")).unwrap();
        Self(path)
    }

    fn tree(&self) -> PathBuf {
        self.0.join("tree")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The probe the test controls; `None` means the real one.
type Faults = Arc<Mutex<Option<Probe>>>;

struct Fixture {
    host: Host,
    log: Log,
    milestones: Arc<Mutex<Vec<Milestone>>>,
    faults: Faults,
    signaller: Signaller,
    scratch: Scratch,
}

fn start(tag: &str, gate: Option<Arc<Gate>>) -> Fixture {
    let scratch = Scratch::new(tag);
    let tree = scratch.tree();
    std::fs::write(tree.join("first.txt"), b"x").unwrap();
    let log = Log {
        rows: Arc::default(),
        gate,
    };
    let faults: Faults = Arc::new(Mutex::new(None));
    let plain = wide(&tree);
    let spec = WatchSpec {
        root: ROOT,
        vol: VolIdx(0),
        long: to_long_path(&plain).unwrap(),
        plain,
        recursive: true,
        buffer_bytes: 64 * 1024,
        last_access: false,
    };
    let bind = Arc::new(OnceLock::new());
    let reconciler = Reconciler::start(
        SnapshotDb::open(&scratch.0.join("snapshot.db")).unwrap(),
        LogRows(Log {
            rows: Arc::clone(&log.rows),
            gate: None,
        }),
        vec![RootSpec {
            root: ROOT,
            vol: VolIdx(0),
            path: spec.long.clone(),
        }],
        clock,
        Host::observer(&bind),
    )
    .unwrap();
    let milestones: Arc<Mutex<Vec<Milestone>>> = Arc::default();
    let recorded = Arc::clone(&milestones);
    let injected = Arc::clone(&faults);
    let host = Host::start(
        HostParts {
            watches: vec![spec],
            sink: Box::new(LogSink(log.clone())),
            pause: Arc::new(Pause::new()),
            reconciler: Some(reconciler),
            services: Services {
                rows: Box::new(LogRows(log.clone())),
                flush: Box::new(|| {}),
                probe: Box::new(move |path| {
                    injected.lock().unwrap().unwrap_or_else(|| probe_root(path))
                }),
                clock,
                notifier: Some(notifier_factory(&bind)),
                milestone: Arc::new(move |milestone| recorded.lock().unwrap().push(milestone)),
                ready: None,
            },
            exclude: Vec::new(),
        },
        &bind,
    )
    .unwrap();
    watch_system(&host);
    let signaller = host.signaller();
    Fixture {
        host,
        log,
        milestones,
        faults,
        signaller,
        scratch,
    }
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

// Reason codes of `peekbuddy_core::lifecycle::Reason`.
const ARMED: u16 = 1;
const RECONCILE_DONE: u16 = 7;
const DEVICE_REMOVED: u16 = 13;
const VOLUME_BACK: u16 = 16;
const ROOT_DELETED: u16 = 17;
const ROOT_MOVED: u16 = 18;
const ROOT_RESTORED: u16 = 19;
const DEVICE_NOT_READY: u16 = 20;
const DEVICE_GONE: u16 = 21;

#[test]
fn a_watch_comes_up_live_reconciles_and_captures() {
    let fx = start("live", None);
    wait_for("live", || fx.log.has_state(WatchState::Live, ARMED));
    // The start reconcile of a new root is its baseline.
    wait_for("the baseline", || {
        !fx.log.names(Kind::BaselineEstablished).is_empty()
    });
    std::fs::write(fx.scratch.tree().join("new.txt"), b"y").unwrap();
    wait_for("the event", || {
        fx.log.names(Kind::Created).iter().any(|n| n == "new.txt")
    });
}

#[test]
fn a_deleted_root_is_lost_and_comes_back_restored_and_reconciled() {
    let fx = start("delete", None);
    wait_for("live", || fx.log.has_state(WatchState::Live, ARMED));
    wait_for("the baseline", || {
        !fx.log.names(Kind::BaselineEstablished).is_empty()
    });
    let tree = fx.scratch.tree();

    std::fs::remove_dir_all(&tree).unwrap();
    wait_for("lost", || fx.log.has_state(WatchState::Lost, ROOT_DELETED));

    std::fs::create_dir_all(&tree).unwrap();
    std::fs::write(tree.join("again.txt"), b"z").unwrap();
    wait_for("restored", || {
        fx.log.has_state(WatchState::Restored, ROOT_RESTORED)
    });
    wait_for("live again", || {
        fx.log.has_state(WatchState::Live, RECONCILE_DONE)
    });
    // The reconcile after the restore explains what happened while it was
    // away.
    let created = fx.log.names(Kind::ReconciledCreated);
    let deleted = fx.log.names(Kind::ReconciledDeleted);
    assert!(created.iter().any(|n| n == "again.txt"), "{created:?}");
    assert!(deleted.iter().any(|n| n == "first.txt"), "{deleted:?}");
}

#[test]
fn a_renamed_root_is_lost_as_moved_and_found_again_under_its_name() {
    let fx = start("rename", None);
    wait_for("live", || fx.log.has_state(WatchState::Live, ARMED));
    let tree = fx.scratch.tree();
    let moved = fx.scratch.0.join("elsewhere");

    std::fs::rename(&tree, &moved).unwrap();
    wait_for("moved", || fx.log.has_state(WatchState::Lost, ROOT_MOVED));

    std::fs::rename(&moved, &tree).unwrap();
    wait_for("restored", || {
        fx.log.has_state(WatchState::Restored, ROOT_RESTORED)
    });
    wait_for("live again", || {
        fx.log.has_state(WatchState::Live, RECONCILE_DONE)
    });
}

#[test]
fn an_unexpected_error_is_verified_and_a_device_that_is_not_ready_or_gone_is_lost() {
    for (probe, reason) in [
        (Probe::NotReady, DEVICE_NOT_READY),
        (Probe::VolumeGone, DEVICE_GONE),
    ] {
        let fx = start("fault", None);
        wait_for("live", || fx.log.has_state(WatchState::Live, ARMED));
        *fx.faults.lock().unwrap() = Some(probe);
        fx.signaller.signal(ROOT, Signal::IoError);
        wait_for("lost", || fx.log.has_state(WatchState::Lost, reason));

        // The device comes back; a power resume makes the host look again.
        *fx.faults.lock().unwrap() = None;
        fx.signaller.signal(ROOT, Signal::PowerResume);
        wait_for("restored", || {
            fx.log.has_state(WatchState::Restored, VOLUME_BACK)
        });
        wait_for("live again", || {
            fx.log.has_state(WatchState::Live, RECONCILE_DONE)
        });
        drop(fx);
    }
}

#[test]
fn a_removed_volume_is_lost_and_its_arrival_restores_the_watch() {
    let fx = start("remove", None);
    wait_for("live", || fx.log.has_state(WatchState::Live, ARMED));
    fx.signaller.signal_all(Signal::RemoveComplete);
    wait_for("lost", || {
        fx.log.has_state(WatchState::Lost, DEVICE_REMOVED)
    });
    fx.signaller.signal_volume(VolIdx(0), Signal::VolumeBack);
    wait_for("restored", || {
        fx.log.has_state(WatchState::Restored, VOLUME_BACK)
    });
    wait_for("live again", || {
        fx.log.has_state(WatchState::Live, RECONCILE_DONE)
    });
    // Capture works again after the reopen.
    std::fs::write(fx.scratch.tree().join("after.txt"), b"z").unwrap();
    wait_for("the event", || {
        fx.log.names(Kind::Created).iter().any(|n| n == "after.txt")
    });
}

#[test]
fn signals_never_block_even_when_the_consumer_is_slow() {
    let gate = Gate::new(true);
    let fx = start("slow", Some(Arc::clone(&gate)));
    wait_for("live", || fx.log.has_state(WatchState::Live, ARMED));
    // The next state row parks the lifecycle worker inside the row sink.
    gate.set(false);
    fx.signaller.signal(ROOT, Signal::QueryRemove);
    std::thread::sleep(Duration::from_millis(100));

    let started = Instant::now();
    for _ in 0..2000 {
        fx.signaller.signal_all(Signal::PowerSuspend);
        fx.signaller.signal_volume(VolIdx(0), Signal::VolumeBack);
    }
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "posting signals took {:?}",
        started.elapsed()
    );
    gate.set(true);
    drop(fx.host);
}

#[test]
fn the_milestones_follow_the_watch_arming_gaps_and_reconciles() {
    use peekbuddy_core::event::GapReason;

    let fx = start("milestones", None);
    wait_for("live", || fx.log.has_state(WatchState::Live, ARMED));
    wait_for("the baseline", || {
        !fx.log.names(Kind::BaselineEstablished).is_empty()
    });
    let seen = |m: Milestone| {
        fx.milestones
            .lock()
            .unwrap()
            .iter()
            .filter(|x| **x == m)
            .count()
    };
    assert_eq!(seen(Milestone::WatcherArmed), 1);
    wait_for("the reconcile milestones", || {
        seen(Milestone::ReconcileStart) == 1 && seen(Milestone::ReconcileEnd) == 1
    });

    // An overflow is an Overflow and a Gap; a full ring only a Gap.
    fx.signaller
        .signal(ROOT, Signal::Gap(GapReason::BufferOverflow));
    fx.signaller.signal(ROOT, Signal::Gap(GapReason::RingFull));
    wait_for("the gap milestones", || {
        seen(Milestone::Gap) == 2 && seen(Milestone::Overflow) == 1
    });
    // And the reconciles they asked for are recorded too.
    wait_for("the second reconcile", || {
        seen(Milestone::ReconcileStart) >= 2 && seen(Milestone::ReconcileEnd) >= 2
    });
}
