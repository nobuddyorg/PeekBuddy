//! Win32 integration tests (Windows only).
#![cfg(windows)]

use peekbuddy_win::session::current_session_id;
use peekbuddy_win::token::{HIGH_INTEGRITY, integrity_rid};

#[test]
fn session_id_matches_get_process() {
    let id = current_session_id().unwrap();
    let pid = std::process::id();
    let output = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-Command",
            &format!("(Get-Process -Id {pid}).SessionId"),
        ])
        .output()
        .unwrap();
    let reported = String::from_utf8_lossy(&output.stdout);
    assert_eq!(reported.trim(), id.to_string());
}

#[test]
fn hosted_runner_tests_run_elevated() {
    // F12: hosted runners are admin with UAC off. Unelevated behaviour needs a
    // child with a lowered token; how to start one is open in #34.
    assert_eq!(integrity_rid().unwrap(), HIGH_INTEGRITY);
}
