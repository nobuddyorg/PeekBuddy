//! A virtual disk is detached and attached while the engine watches it (#48).
//!
//! Needs an elevated token and `diskpart`, so it only runs when
//! `PEEKBUDDY_VHD_TESTS=1` is set (the nightly job on the hosted runner, once
//! #34 confirms diskpart works there). The expectation: `detach` succeeds
//! while the volume is watched (the handles are closed in answer to the
//! volume's query-remove or lock), the watch is logged as suspended or lost,
//! and `attach` brings it back as `Restored` with a reconcile.
#![cfg(windows)]
#![expect(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns may unwrap"
)]

use std::io::Write as _;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use peekbuddy_core::capture::EventSink;
use peekbuddy_core::event::{Event, Kind, RootIdx, VolIdx, WatchState};
use peekbuddy_core::pause::Pause;
use peekbuddy_engine::capture_host::{
    Host, HostParts, Services, WatchSpec, notifier_factory, real_probe, watch_system,
};
use peekbuddy_engine::pause_ctl::RowSink;
use peekbuddy_win::io::path::to_long_path;

fn enabled() -> bool {
    std::env::var_os("PEEKBUDDY_VHD_TESTS").is_some() && peekbuddy_win::token::is_admin().unwrap()
}

fn diskpart(script: &str) -> bool {
    let path = std::env::temp_dir().join(format!("pbvhd-{}.txt", std::process::id()));
    std::fs::File::create(&path)
        .unwrap()
        .write_all(script.as_bytes())
        .unwrap();
    let status = Command::new("diskpart.exe")
        .arg("/s")
        .arg(&path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    let _ = std::fs::remove_file(&path);
    status.success()
}

#[derive(Clone, Default)]
struct States(Arc<Mutex<Vec<WatchState>>>);

impl RowSink for States {
    fn row(&mut self, event: Event) {
        if let Kind::WatchStateChanged(state) = event.kind {
            self.0.lock().unwrap().push(state);
        }
    }
}

struct Quiet;

impl EventSink for Quiet {
    fn push(&mut self, _: &Event, _: &[u16]) -> bool {
        true
    }
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().collect()
}

#[test]
fn a_vhd_can_be_detached_while_watched_and_comes_back_on_attach() {
    if !enabled() {
        return;
    }
    let base: PathBuf = std::env::temp_dir().join(format!("pbvhd-{}", std::process::id()));
    let mount = base.join("mount");
    std::fs::create_dir_all(&mount).unwrap();
    let vhd = base.join("disk.vhd");
    assert!(diskpart(&format!(
        "create vdisk file=\"{}\" maximum=64 type=expandable\nattach vdisk\ncreate partition primary\nformat fs=ntfs quick label=pbtest\nassign mount=\"{}\"\n",
        vhd.display(),
        mount.display()
    )));
    let tree = mount.join("watched");
    std::fs::create_dir_all(&tree).unwrap();
    std::fs::write(tree.join("a.txt"), b"x").unwrap();

    let states = States::default();
    let bind = Arc::new(OnceLock::new());
    let plain = wide(&tree);
    let host = Host::start(
        HostParts {
            watches: vec![WatchSpec {
                root: RootIdx(0),
                vol: VolIdx(0),
                long: to_long_path(&plain).unwrap(),
                plain,
                recursive: true,
                buffer_bytes: 64 * 1024,
                last_access: false,
            }],
            sink: Box::new(Quiet),
            pause: Arc::new(Pause::new()),
            reconciler: None,
            services: Services {
                rows: Box::new(states.clone()),
                flush: Box::new(|| {}),
                probe: real_probe(),
                clock: || (1, 1),
                notifier: Some(notifier_factory(&bind)),
                milestone: Arc::new(|_| {}),
                ready: None,
            },
            exclude: Vec::new(),
        },
        &bind,
    )
    .unwrap();
    watch_system(&host);
    wait_for("live", || {
        states.0.lock().unwrap().contains(&WatchState::Live)
    });

    // Detach while the engine holds the volume.
    assert!(
        diskpart(&format!(
            "select vdisk file=\"{}\"\ndetach vdisk\n",
            vhd.display()
        )),
        "detach failed: a handle of the engine kept the volume"
    );
    wait_for("suspended or lost", || {
        states.0.lock().unwrap().iter().any(|s| {
            matches!(
                s,
                WatchState::SuspendedQueryRemove | WatchState::SuspendedLocked | WatchState::Lost
            )
        })
    });

    assert!(diskpart(&format!(
        "select vdisk file=\"{}\"\nattach vdisk\n",
        vhd.display()
    )));
    wait_for("restored", || {
        states.0.lock().unwrap().contains(&WatchState::Restored)
    });
    drop(host);
    let _ = diskpart(&format!(
        "select vdisk file=\"{}\"\ndetach vdisk\n",
        vhd.display()
    ));
    let _ = std::fs::remove_dir_all(&base);
}
