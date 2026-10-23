//! Win32 integration tests (Windows only).
#![cfg(windows)]

use peekbuddy_win::process::run_restricted;
use peekbuddy_win::session::current_session_id;

#[test]
fn session_id_matches_tasklist() {
    let id = current_session_id().unwrap();
    let pid = std::process::id();
    let output = std::process::Command::new("tasklist")
        .args(["/fi", &format!("PID eq {pid}"), "/fo", "csv", "/nh"])
        .output()
        .unwrap();
    let line = String::from_utf8_lossy(&output.stdout);
    // "image","pid","session name","session#","mem usage"
    let fields: Vec<&str> = line.trim().split("\",\"").collect();
    assert_eq!(
        fields.get(3).copied(),
        Some(id.to_string().as_str()),
        "{line}"
    );
}

#[test]
fn restricted_child_reports_its_exit_code() {
    assert_eq!(run_restricted("cmd.exe /d /c exit 0").unwrap(), 0);
    assert_eq!(run_restricted("cmd.exe /d /c exit 7").unwrap(), 7);
}

#[test]
fn restricted_child_runs_at_medium_integrity_without_admin() {
    // findstr exits 0 when the pattern is found.
    let medium = run_restricted(
        r#"cmd.exe /d /c whoami /groups | findstr /C:"Mandatory Label\Medium Mandatory Level""#,
    )
    .unwrap();
    assert_eq!(medium, 0, "child is not at Medium integrity");
    let admin = run_restricted(
        r#"cmd.exe /d /c whoami /groups | findstr /R /C:"BUILTIN\\Administrators.*Group used for deny only""#,
    )
    .unwrap();
    assert_eq!(admin, 0, "Administrators is not deny-only in the child");
}

#[test]
fn a_bad_command_is_an_error() {
    assert!(run_restricted("this-program-does-not-exist.exe").is_err());
}
