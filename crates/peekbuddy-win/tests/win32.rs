//! Win32 integration tests (Windows only).
//!
//! The unelevated-child tests re-run this test binary as the child, so the
//! child can check its own token. The child entry points are `#[ignore]`d so
//! normal runs skip them.
#![cfg(windows)]

use peekbuddy_win::process::run_restricted;
use peekbuddy_win::session::current_session_id;
use peekbuddy_win::token::{HIGH_INTEGRITY, MEDIUM_INTEGRITY, integrity_rid, is_admin};

/// Runs one ignored test of this binary as an unelevated child.
fn run_child(test: &str) -> std::io::Result<u32> {
    let exe = std::env::current_exe()?;
    let command = format!(
        "\"{}\" {test} --exact --ignored --test-threads=1 --quiet",
        exe.display()
    );
    run_restricted(&command)
}

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
    // F12: hosted runners are admin with UAC off; the unelevated tests below
    // only mean something if the parent really is elevated.
    assert_eq!(integrity_rid().unwrap(), HIGH_INTEGRITY);
    assert!(is_admin().unwrap());
}

#[test]
fn restricted_child_reports_its_exit_code() {
    assert_eq!(run_child("child_exits_with_seven").unwrap(), 7);
    assert_eq!(run_child("child_exits_with_zero").unwrap(), 0);
}

#[test]
fn restricted_cmd_child_runs() {
    assert_eq!(run_restricted("cmd.exe /d /c exit 7").unwrap(), 7);
}

#[test]
fn restricted_child_is_medium_integrity_and_not_admin() {
    assert_eq!(run_child("child_reports_its_token").unwrap(), 0);
}

#[test]
fn a_bad_command_is_an_error() {
    assert!(run_restricted("this-program-does-not-exist.exe").is_err());
}

#[test]
#[ignore = "child entry point for restricted_child_reports_its_exit_code"]
fn child_exits_with_seven() {
    std::process::exit(7);
}

#[test]
#[ignore = "child entry point for restricted_child_reports_its_exit_code"]
fn child_exits_with_zero() {
    std::process::exit(0);
}

#[test]
#[ignore = "child entry point for restricted_child_is_medium_integrity_and_not_admin"]
fn child_reports_its_token() {
    let code = match (integrity_rid(), is_admin()) {
        (Ok(MEDIUM_INTEGRITY), Ok(false)) => 0,
        (Ok(MEDIUM_INTEGRITY), _) => 2,
        (Ok(_), _) => 3,
        (Err(_), _) => 4,
    };
    std::process::exit(code);
}

#[test]
fn a_low_integrity_child_reports_its_exit_code() {
    use peekbuddy_win::process::run_low_integrity;

    assert_eq!(run_low_integrity("cmd.exe /d /c exit 3").unwrap(), 3);
    assert_eq!(run_low_integrity("cmd.exe /d /c exit 0").unwrap(), 0);
    assert!(run_low_integrity("this-program-does-not-exist.exe").is_err());
}

#[test]
fn the_modules_of_a_process_are_listed_by_base_name() {
    use peekbuddy_win::process::module_names;

    let names = module_names(std::process::id()).unwrap();
    let exe = std::env::current_exe().unwrap();
    let exe_name = exe.file_name().unwrap().to_string_lossy().to_lowercase();
    // The exe itself comes first, then the system libraries every process has.
    assert_eq!(names.first(), Some(&exe_name), "{names:?}");
    for system in ["ntdll.dll", "kernel32.dll", "kernelbase.dll"] {
        assert!(
            names.iter().any(|name| name == system),
            "{system} in {names:?}"
        );
    }
    assert!(names.len() >= 4 && names.len() <= 1024, "{}", names.len());
    assert!(
        names
            .iter()
            .all(|name| !name.is_empty() && name == &name.to_lowercase() && !name.contains('\\')),
        "{names:?}"
    );
    // A process that does not exist cannot be opened.
    assert!(module_names(u32::MAX - 1).is_err());
}

#[test]
fn another_user_with_a_wrong_password_is_an_error() {
    use peekbuddy_win::process::run_as_user;

    let result = run_as_user(
        "PeekBuddyNoSuchUser",
        "not a password",
        "cmd.exe /d /c exit 0",
    );
    assert!(result.is_err(), "{result:?}");
}
