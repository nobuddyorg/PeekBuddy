//! The viewer of the release exe, asked through UI Automation the way a
//! screen reader asks (#71): its window is found by title, the named
//! controls of its screens are listed and some are invoked. Nothing here
//! knows how the window is drawn, so it holds for glow, wgpu and WARP alike.
//!
//! Needs a desktop session (`assert_interactive_session`); the hosted
//! runners have one.
#![cfg(windows)]
#![expect(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns may unwrap"
)]

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use peekbuddy_testkit::TestDir;
use peekbuddy_win::pipe::stable_hash;
use peekbuddy_win::uia::Client;
use peekbuddy_win::window::WindowHandle;

const EXE: &str = env!("CARGO_BIN_EXE_peekbuddy");

/// The viewer, killed when dropped.
struct Viewer(Child);

impl Drop for Viewer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start(data: &Path) -> Viewer {
    let child = Command::new(EXE)
        .args(["--viewer", "--data-dir"])
        .arg(data)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    Viewer(child)
}

fn window_of(data: &Path, viewer: &mut Viewer) -> WindowHandle {
    let title =
        peekbuddy_viewer::isolated_title(stable_hash(&data.to_string_lossy().to_lowercase()));
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        if let Some(window) = WindowHandle::find_title(&title) {
            return window;
        }
        assert!(
            viewer.0.try_wait().unwrap().is_none(),
            "the viewer exited before its window appeared"
        );
        assert!(Instant::now() < deadline, "no viewer window titled {title}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The names of the controls of the window now; the error says why there are
/// none (a window UI Automation does not know yet, an empty tree).
///
/// Every call connects anew: on the hosted runners the first connection of a
/// session can see an empty tree for a long time while the next one sees the
/// window at once (the first attempt of this test timed out and the retry
/// passed in 4 s, every time), and a fresh connection costs nothing.
fn names(window: WindowHandle) -> Result<Vec<String>, String> {
    let client = Client::new().map_err(|e| e.to_string())?;
    let nodes = client
        .window(window.raw())
        .and_then(|window| window.nodes())
        .map_err(|e| e.to_string())?;
    Ok(nodes.into_iter().map(|node| node.name).collect())
}

/// Waits up to `seconds` until a control named `name` is there (or gone, when
/// `present` is false) and returns every name then, or the last answer.
fn wait_within(
    window: WindowHandle,
    name: &str,
    present: bool,
    seconds: u64,
) -> Result<Vec<String>, String> {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        let now = names(window);
        if let Ok(now) = &now
            && now.iter().any(|n| n == name) == present
        {
            return Ok(now.clone());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "{name:?} did not {} in {seconds} s: {now:#?}",
                if present { "appear" } else { "go" }
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// [`wait_within`] for the 60 s of a screen change that must happen.
fn wait_for(window: WindowHandle, name: &str, present: bool) -> Vec<String> {
    wait_within(window, name, present, 60).unwrap()
}

/// A viewer on a fresh data directory whose window UI Automation can read.
///
/// On the hosted runners the first viewer of a session answers
/// `ElementFromHandle` with 0x80040201 ("an event was unable to invoke any of
/// the subscribers") for as long as it lives, while the next viewer, started
/// seconds later, answers at once (the first attempt of this test failed after
/// 60 s and the retry passed in 4 s, in every run since it exists). So a
/// viewer that is not readable after 20 s is replaced, up to three times.
fn readable_viewer() -> Result<(TestDir, Viewer, WindowHandle), String> {
    let mut last = String::new();
    for _ in 0..3 {
        let dir = TestDir::new("e2e-viewer").unwrap();
        let mut viewer = start(dir.path());
        let window = window_of(dir.path(), &mut viewer);
        match wait_within(window, "Welcome to PeekBuddy", true, 20) {
            Ok(_) => return Ok((dir, viewer, window)),
            Err(why) => last = why,
        }
    }
    Err(format!(
        "no viewer was readable through UI Automation in three tries: {last}"
    ))
}

#[test]
fn the_welcome_screens_work_through_ui_automation() {
    peekbuddy_testkit::assert_interactive_session();
    let (_dir, mut viewer, window) = readable_viewer().unwrap();

    // A fresh data directory opens the welcome screen, with no engine.
    let first = wait_for(window, "Welcome to PeekBuddy", true);
    for expected in ["Step 1 of 5", "Next", "Skip", "Engine not running"] {
        assert!(
            first.iter().any(|n| n == expected),
            "{expected:?} in {first:#?}"
        );
    }

    // The screens work without a mouse or a pixel: invoke by name.
    let client = Client::new().unwrap();
    let ui = client.window(window.raw()).unwrap();
    ui.invoke("Next").unwrap();
    wait_for(window, "Step 2 of 5", true);
    ui.invoke("Skip").unwrap();
    let after = wait_for(window, "Skip", false);
    assert!(!after.iter().any(|n| n.starts_with("Step ")), "{after:#?}");

    // The log screen: its tabs, filters and the empty-log hint.
    for expected in [
        "Log",
        "Watches (0)",
        "Changes",
        "Stats",
        "Settings",
        "Search",
        "Created",
        "Deleted",
        "Compact",
        "Export",
        "No events recorded yet. Changes appear here as they happen.",
    ] {
        assert!(
            after.iter().any(|n| n == expected),
            "{expected:?} in {after:#?}"
        );
    }
    ui.invoke("Compact").unwrap();
    ui.invoke("Settings").unwrap();
    wait_for(window, "Log", true);

    // Closing the window ends the viewer cleanly.
    window.post_close().unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = viewer.0.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "the viewer did not close");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(status.success(), "{status:?}");
}
