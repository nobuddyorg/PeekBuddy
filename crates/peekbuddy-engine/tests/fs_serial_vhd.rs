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

/// Runs a diskpart script: its output on success, on failure too (with the
/// exit status), so that a nightly failure says why.
fn diskpart(script: &str) -> Result<String, String> {
    let path = std::env::temp_dir().join(format!("pbvhd-{}.txt", std::process::id()));
    std::fs::File::create(&path)
        .unwrap()
        .write_all(script.as_bytes())
        .unwrap();
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

/// The path with its long names, without the verbatim prefix diskpart rejects.
fn long_path(path: &Path) -> PathBuf {
    let canonical = std::fs::canonicalize(path).unwrap();
    let text = canonical.to_string_lossy();
    PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(&text))
}

/// What diskpart says about the virtual disk.
fn detail(vhd: &Path) -> Result<String, String> {
    diskpart(&format!(
        "select vdisk file=\"{}\"
detail vdisk
list vdisk
",
        vhd.display()
    ))
}

/// A diskpart step that must work.
fn must(script: &str) {
    let result = diskpart(script);
    assert!(result.is_ok(), "diskpart failed: {result:?}");
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

fn wait_for(what: &str, states: &States, mut done: impl FnMut() -> bool) {
    wait_for_with(what, states, String::new, &mut done);
}

fn wait_for_with(
    what: &str,
    states: &States,
    context: impl Fn() -> String,
    done: &mut impl FnMut() -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !done() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what}; the watch went through {:?}; {}",
            states.0.lock().unwrap(),
            context()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().collect()
}

/// The volume GUID path a directory lives on.
fn volume_of(path: &Path) -> Vec<u16> {
    use peekbuddy_win::io::dir::DirHandle;
    use peekbuddy_win::io::open::OpenOptions;
    use peekbuddy_win::io::volume::volume_identity;

    let long = to_long_path(&wide(path)).unwrap();
    let dir = DirHandle::open(&long, OpenOptions::default()).unwrap();
    volume_identity(&dir, &long).unwrap().guid_path
}

/// Creates the virtual disk and mounts it on `mount`, which must then be on
/// another volume than `base`.
fn new_disk(vhd: &Path, mount: &Path, base: &Path) {
    must(&format!(
        "create vdisk file=\"{}\" maximum=64 type=expandable\nattach vdisk\ncreate partition primary\nformat fs=ntfs quick label=pbtest\nassign mount=\"{}\"\n",
        vhd.display(),
        mount.display()
    ));
    // Without this a disk that was not (or no longer is) attached would leave
    // the test watching a plain folder of the system volume.
    assert_ne!(
        volume_of(mount),
        volume_of(base),
        "the mount folder is not on the virtual disk; diskpart says: {:?}",
        detail(vhd)
    );
}

#[test]
fn a_vhd_can_be_detached_while_watched_and_comes_back_on_attach() {
    if !enabled() {
        return;
    }
    let short: PathBuf = std::env::temp_dir().join(format!("pbvhd-{}", std::process::id()));
    std::fs::create_dir_all(short.join("mount")).unwrap();
    // The runner's temp directory is an 8.3 path (RUNNER~1): diskpart lists
    // the disk under the long name and `select vdisk` of the short one finds a
    // second, never attached entry, so the detach did nothing.
    let base = long_path(&short);
    let mount = base.join("mount");
    let vhd = base.join("disk.vhd");
    new_disk(&vhd, &mount, &base);
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
    wait_for("live", &states, || {
        states.0.lock().unwrap().contains(&WatchState::Live)
    });

    // Detach while the engine holds the volume.
    let detached = diskpart(&format!(
        "select vdisk file=\"{}\"\ndetach vdisk\n",
        vhd.display()
    ));
    // On the hosted runner diskpart can report the disk as already detached
    // (0x80070057) when the volume went away under the engine's watch: the
    // states below still have to show it.
    let already = detached
        .as_ref()
        .err()
        .is_some_and(|text| text.contains("already detached"));
    assert!(
        detached.is_ok() || already,
        "detach failed (a handle of the engine may have kept the volume): {detached:?}"
    );
    wait_for_with(
        "suspended or lost",
        &states,
        || {
            format!(
                "detach said {detached:?}; the watched file is {:?}; diskpart now: {:?}",
                std::fs::metadata(tree.join("a.txt")).map(|m| m.len()),
                detail(&vhd)
            )
        },
        &mut || {
            states.0.lock().unwrap().iter().any(|s| {
                matches!(
                    s,
                    WatchState::SuspendedQueryRemove
                        | WatchState::SuspendedLocked
                        | WatchState::Lost
                )
            })
        },
    );

    must(&format!(
        "select vdisk file=\"{}\"\nattach vdisk\n",
        vhd.display()
    ));
    wait_for("restored", &states, || {
        states.0.lock().unwrap().contains(&WatchState::Restored)
    });
    drop(host);
    let _ = diskpart(&format!(
        "select vdisk file=\"{}\"\ndetach vdisk\n",
        vhd.display()
    ));
    let _ = std::fs::remove_dir_all(&base);
}
