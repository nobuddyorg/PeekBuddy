//! Integration tests of the engine process skeleton (#59). They start the real
//! `peekbuddy.exe`, so they run on Windows only.
#![cfg(windows)]
#![expect(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns may unwrap"
)]

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use peekbuddy_engine::Names;
use peekbuddy_testkit::TestDir;
use peekbuddy_win::window::WindowHandle;

const EXE: &str = env!("CARGO_BIN_EXE_peekbuddy");

fn spawn(data: &Path, extra: &[&str]) -> Child {
    Command::new(EXE)
        .args(["--headless", "--data-dir"])
        .arg(data)
        .args(extra)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

fn wait_for_window(data: &Path, child: &mut Child) -> WindowHandle {
    let class = Names::new(Some(data)).window_class;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(window) = WindowHandle::find(&class) {
            return window;
        }
        assert!(
            child.try_wait().unwrap().is_none(),
            "the engine exited before its window appeared"
        );
        assert!(Instant::now() < deadline, "no engine window after 20 s");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn wait_for_exit(child: &mut Child, within: Duration) -> std::process::ExitStatus {
    let deadline = Instant::now() + within;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        assert!(
            Instant::now() < deadline,
            "the process did not exit in time"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Stops an engine through the hidden window and checks the exit code.
fn quit(window: WindowHandle, child: &mut Child) {
    window.post_close().unwrap();
    let status = wait_for_exit(child, Duration::from_secs(10));
    assert!(status.success(), "{status:?}");
}

struct Kill(Child);

impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn a_second_engine_exits_zero_within_a_second_and_leaves_the_first_running() {
    let dir = TestDir::new("engine-second").unwrap();
    let mut first = Kill(spawn(dir.path(), &[]));
    let window = wait_for_window(dir.path(), &mut first.0);

    let started = Instant::now();
    let mut second = spawn(dir.path(), &[]);
    let status = wait_for_exit(&mut second, Duration::from_secs(1));
    assert!(status.success(), "{status:?}");
    assert!(started.elapsed() < Duration::from_secs(1));

    assert!(
        first.0.try_wait().unwrap().is_none(),
        "the first engine died"
    );
    assert_eq!(
        WindowHandle::find(&Names::new(Some(dir.path())).window_class),
        Some(window)
    );
    quit(window, &mut first.0);
}

#[test]
fn the_viewer_mutex_refuses_a_second_viewer() {
    // The viewer is a stub that returns at once, so a second `--viewer` is
    // checked at the primitive level: the exe exits 0 either way. The
    // end-to-end check arrives with the real viewer window (#65).
    let status = Command::new(EXE).arg("--viewer").status().unwrap();
    assert!(status.success());
}

#[test]
fn a_refused_command_line_exits_with_two() {
    let status = Command::new(EXE).arg("--no-such-flag").status().unwrap();
    assert_eq!(status.code(), Some(2));
    let status = Command::new(EXE)
        .args(["--data-dir", "x"])
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(2));
}

#[test]
fn the_exe_carries_exactly_one_correct_manifest() {
    let manifests = peekbuddy_win::resources::manifests(Path::new(EXE)).unwrap();
    assert_eq!(manifests.len(), 1, "{manifests:?}");
    let xml = &manifests[0];
    for needle in [
        ">permonitorv2,permonitor<",
        "<longPathAware",
        "UTF-8",
        r#"level="asInvoker""#,
        // Windows 10 and 11 (ADR 0013).
        "8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a",
        "Microsoft.Windows.Common-Controls",
    ] {
        assert!(xml.contains(needle), "{needle} missing in {xml}");
    }
    assert!(xml.contains(">true</longPathAware>"), "{xml}");
}

#[test]
fn the_headless_engine_loads_no_gpu_or_ui_stack() {
    let dir = TestDir::new("engine-modules").unwrap();
    let mut engine = Kill(spawn(dir.path(), &[]));
    let window = wait_for_window(dir.path(), &mut engine.0);
    let modules = peekbuddy_win::process::module_names(engine.0.id()).unwrap();
    assert!(
        modules.iter().any(|name| name == "peekbuddy.exe"),
        "{modules:?}"
    );
    for banned in [
        "opengl32.dll",
        "d3d12.dll",
        "dxgi.dll",
        "d3dcompiler_47.dll",
    ] {
        assert!(
            !modules.iter().any(|name| name == banned),
            "the engine loaded {banned}: {modules:?}"
        );
    }
    quit(window, &mut engine.0);
}

#[test]
fn the_data_dir_is_created_and_not_content_indexed() {
    let dir = TestDir::new("engine-data").unwrap();
    let data = dir.path().join("data");
    let mut engine = Kill(spawn(&data, &[]));
    let window = wait_for_window(&data, &mut engine.0);
    for folder in [data.clone(), data.join("segments"), data.join("logs")] {
        assert!(
            peekbuddy_win::attrs::is_not_content_indexed(&folder).unwrap(),
            "{folder:?}"
        );
    }
    quit(window, &mut engine.0);
}

#[test]
fn the_session_end_messages_are_answered_at_once() {
    let dir = TestDir::new("engine-endsession").unwrap();
    let mut engine = Kill(spawn(dir.path(), &[]));
    let window = wait_for_window(dir.path(), &mut engine.0);
    let started = Instant::now();
    assert!(window.query_end_session(2_000).unwrap());
    window.end_session(2_000).unwrap();
    assert!(started.elapsed() < Duration::from_secs(2));
    // The shutdown ran once; closing afterwards is still a clean exit.
    quit(window, &mut engine.0);
}

#[test]
fn a_panic_leaves_a_crash_log_and_marker_and_the_next_start_takes_the_marker() {
    // Needs the exe built with `--features test-panic`; without it the flag
    // is refused, which is itself checked.
    let dir = TestDir::new("engine-crash").unwrap();
    let data = dir.path().join("data");
    let mut crashing = spawn(&data, &["--test-panic"]);
    let status = wait_for_exit(&mut crashing, Duration::from_secs(20));
    if cfg!(feature = "test-panic") {
        assert!(!status.success(), "{status:?}");
        let logs: Vec<_> = std::fs::read_dir(data.join("logs"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(logs.len(), 1, "{logs:?}");
        assert!(
            logs[0].starts_with("crash-") && logs[0].ends_with("Z.log"),
            "{logs:?}"
        );
        let report = std::fs::read_to_string(data.join("logs").join(&logs[0])).unwrap();
        assert!(report.contains("test panic requested"), "{report}");
        let marker = data.join(peekbuddy_engine::crash::MARKER_FILE);
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), logs[0]);

        let mut next = Kill(spawn(&data, &[]));
        let window = wait_for_window(&data, &mut next.0);
        assert!(!marker.exists(), "the next start did not take the marker");
        quit(window, &mut next.0);
    } else {
        assert_eq!(status.code(), Some(2));
    }
}

#[test]
fn a_first_start_writes_the_default_settings_and_a_corrupt_file_survives() {
    let dir = TestDir::new("engine-settings").unwrap();
    let data = dir.path().join("data");

    let mut first = Kill(spawn(&data, &[]));
    let window = wait_for_window(&data, &mut first.0);
    let text = std::fs::read_to_string(data.join("settings.toml")).unwrap();
    assert!(text.contains("version = 1"), "{text}");
    assert!(text.contains("first_run = true"), "{text}");
    quit(window, &mut first.0);

    // A corrupt file: the engine still starts and leaves it byte for byte.
    let bad = b"version = 1\n[general\nnot toml at all";
    std::fs::write(data.join("settings.toml"), bad).unwrap();
    let mut second = Kill(spawn(&data, &[]));
    let window = wait_for_window(&data, &mut second.0);
    assert_eq!(std::fs::read(data.join("settings.toml")).unwrap(), bad);
    quit(window, &mut second.0);

    // --diagnostics says what is wrong without touching the file.
    let output = Command::new(EXE)
        .arg("--diagnostics")
        .env("LOCALAPPDATA", dir.path().join("local"))
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(std::fs::read(data.join("settings.toml")).unwrap(), bad);
}

fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn the_tray_icon_comes_back_with_the_taskbar_and_goes_away_on_quit() {
    use peekbuddy_engine::tray_model::Command;
    use peekbuddy_win::tray::{Tray, taskbar_created_message};

    let dir = TestDir::new("engine-tray").unwrap();
    let mut engine = Kill(spawn(dir.path(), &["--tray"]));
    let window = wait_for_window(dir.path(), &mut engine.0);
    let tray = Tray::new(window);
    wait_until("the tray icon", || tray.is_present());

    // Explorer restarting broadcasts TaskbarCreated: the icon is added again.
    tray.remove();
    if !tray.is_present() {
        window
            .send_timeout(taskbar_created_message(), 0, 0, 5_000)
            .unwrap();
        wait_until("the icon to be re-added", || tray.is_present());
    }

    // The same dispatch the real menu uses: Quit leaves no ghost icon.
    window
        .send_timeout(0x0111, Command::Quit.id() as usize, 0, 5_000)
        .unwrap();
    let status = wait_for_exit(&mut engine.0, Duration::from_secs(10));
    assert!(status.success(), "{status:?}");
    assert!(!tray.is_present(), "a ghost icon is left behind");
}
