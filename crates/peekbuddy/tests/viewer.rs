//! The viewer role against the real exe (#65): it opens, connects to the
//! engine, stays quiet while idle, survives no engine and never doubles.
//!
//! The hosted runners have no GPU: the window renders through WARP, and the
//! numbers are not performance numbers (F12). These tests only check what
//! does not depend on the rasteriser.
#![cfg(windows)]
#![expect(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns may unwrap"
)]

use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use peekbuddy_engine::Names;
use peekbuddy_testkit::TestDir;
use peekbuddy_win::window::WindowHandle;

const EXE: &str = env!("CARGO_BIN_EXE_peekbuddy");

/// A running headless engine, killed when dropped.
struct Engine(Child);

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start_engine(data: &Path) -> (Engine, WindowHandle) {
    let mut child = Command::new(EXE)
        .args(["--headless", "--data-dir"])
        .arg(data)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let class = Names::new(Some(data)).window_class;
    let deadline = Instant::now() + Duration::from_secs(60);
    let window = loop {
        if let Some(window) = WindowHandle::find(&class) {
            break window;
        }
        assert!(
            child.try_wait().unwrap().is_none(),
            "the engine exited before its window appeared"
        );
        assert!(Instant::now() < deadline, "no engine window");
        std::thread::sleep(Duration::from_millis(20));
    };
    (Engine(child), window)
}

fn stop_engine(engine: &mut Engine, window: WindowHandle) {
    window.post_close().unwrap();
    let status = wait_exit(&mut engine.0, Duration::from_secs(60), "the engine");
    assert!(status.success(), "{status:?}");
}

fn viewer(data: &Path) -> Command {
    let mut command = Command::new(EXE);
    command
        .args(["--viewer", "--data-dir"])
        .arg(data)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

fn wait_exit(child: &mut Child, limit: Duration, what: &str) -> ExitStatus {
    let deadline = Instant::now() + limit;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        assert!(Instant::now() < deadline, "{what} did not exit in time");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn ping(data: &Path) -> Option<i32> {
    Command::new(EXE)
        .args(["--ipc-ping", "--data-dir"])
        .arg(data)
        .status()
        .unwrap()
        .code()
}

/// A probe run: the exit status and the report line.
fn probe(data: &Path, seconds: u32, vulkan: bool) -> (ExitStatus, String) {
    let out = data.join("viewer-probe.json");
    let mut command = viewer(data);
    command
        .args(["--viewer-probe", &seconds.to_string(), "--perf-probe-out"])
        .arg(&out);
    if vulkan {
        // What a user's environment might say; the viewer must ignore it.
        command
            .env("WGPU_BACKEND", "vulkan")
            .env("WGPU_POWER_PREF", "high");
    }
    let mut child = command.spawn().unwrap();
    // Window creation on a loaded hosted runner is slow; the report is
    // written when the window closes.
    let status = wait_exit(&mut child, Duration::from_secs(180), "the probe viewer");
    (status, std::fs::read_to_string(&out).unwrap_or_default())
}

/// The report as JSON; a line that does not parse fails the test with it.
fn parse(report: &str) -> serde_json::Value {
    let parsed = serde_json::from_str(report);
    assert!(parsed.is_ok(), "not JSON: {report:?}");
    parsed.unwrap()
}

#[test]
fn an_idle_viewer_paints_nothing_and_stays_on_dx12_whatever_the_environment_says() {
    let dir = TestDir::new("viewer-idle").unwrap();
    let (mut running, window) = start_engine(dir.path());

    let (status, report) = probe(dir.path(), 10, true);
    assert!(status.success(), "{status:?} {report}");
    let report = parse(&report);
    // The renderer is DX12 although WGPU_BACKEND said vulkan.
    assert_eq!(report["backend"], "Dx12", "{report}");
    // The first frame happened and carries a time since process creation.
    assert!(report["first_frame_us"].as_u64().unwrap() > 0, "{report}");
    // With the window open and left alone for 10 s, almost nothing is
    // painted. Zero is the goal, but an idle eframe window on Windows gets about
    // one input event a second from the OS (spike 0003 saw 1.0 to 1.2 frames/s
    // on a real GPU too; the report's `causes` says what asked), so the test
    // guards against what matters: a continuous repaint would be 600 frames.
    assert_eq!(report["idle_seconds"], 10, "{report}");
    let idle = report["idle_frames"].as_u64().unwrap();
    assert!(idle <= 12, "{idle} frames while idle: {report}");
    stop_engine(&mut running, window);
}

#[test]
fn a_viewer_without_an_engine_still_opens_and_closes() {
    // Read-only browsing must not depend on the engine.
    let dir = TestDir::new("viewer-no-engine").unwrap();
    let (status, report) = probe(dir.path(), 2, false);
    assert!(status.success(), "{status:?} {report}");
    assert!(parse(&report)["frames"].as_u64().unwrap() > 0, "{report}");
    // The adapter line is in the viewer log.
    let log = std::fs::read_to_string(dir.path().join("logs").join("viewer.log")).unwrap();
    assert!(log.contains("viewer: adapter"), "{log}");
}

#[test]
fn killing_the_viewer_leaves_the_engine_capturing() {
    let dir = TestDir::new("viewer-kill").unwrap();
    let (mut running, window) = start_engine(dir.path());

    let mut shown = viewer(dir.path()).spawn().unwrap();
    // Long enough for the window and the link to exist.
    std::thread::sleep(Duration::from_secs(8));
    assert!(
        shown.try_wait().unwrap().is_none(),
        "the viewer closed itself"
    );
    shown.kill().unwrap();
    shown.wait().unwrap();

    // The engine still runs and still answers on its pipe.
    assert!(running.0.try_wait().unwrap().is_none(), "the engine died");
    assert_eq!(ping(dir.path()), Some(0));
    stop_engine(&mut running, window);
}

#[test]
fn a_second_launch_exits_zero_and_there_is_never_a_second_window() {
    let dir = TestDir::new("viewer-twice").unwrap();
    let mut first = viewer(dir.path()).spawn().unwrap();
    std::thread::sleep(Duration::from_secs(8));
    assert!(
        first.try_wait().unwrap().is_none(),
        "the first viewer closed"
    );

    // A second viewer that opened a window would still run after 30 s; this
    // one hands over and leaves.
    let mut second = viewer(dir.path()).spawn().unwrap();
    let status = wait_exit(&mut second, Duration::from_secs(30), "the second launch");
    assert!(status.success(), "{status:?}");
    assert!(
        first.try_wait().unwrap().is_none(),
        "the first viewer went away"
    );

    first.kill().unwrap();
    first.wait().unwrap();
}
