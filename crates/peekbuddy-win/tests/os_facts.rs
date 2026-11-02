//! The facts of `osinfo` against independent queries (`reg.exe`), so a
//! function that returns a constant, or reads the wrong key, is found (#234).
#![cfg(windows)]
#![expect(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns may unwrap"
)]

use std::process::Command;

use peekbuddy_win::osinfo::{current_user_dword, defender_realtime_enabled, update_build_revision};

/// `reg query <key> /v <value>` as a `REG_DWORD`, read by a program that
/// shares no code with the crate; `None` when the value is not there.
fn reg_dword(key: &str, value: &str) -> Option<u32> {
    let output = Command::new("reg")
        .args(["query", key, "/v", value])
        .output()
        .unwrap();
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let hex = text
        .lines()
        .find(|line| line.trim_start().starts_with(value))?
        .split_whitespace()
        .last()?
        .trim_start_matches("0x")
        .to_owned();
    u32::from_str_radix(&hex, 16).ok()
}

#[test]
fn the_update_build_revision_is_the_one_reg_reads() {
    let wanted = reg_dword(r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion", "UBR");
    assert!(wanted.is_some(), "this machine has a UBR");
    assert_eq!(update_build_revision(), wanted);
}

#[test]
fn defender_real_time_is_the_inverse_of_the_disable_switch() {
    let disabled = reg_dword(
        r"HKLM\SOFTWARE\Microsoft\Windows Defender\Real-Time Protection",
        "DisableRealtimeMonitoring",
    );
    assert_eq!(
        defender_realtime_enabled(),
        disabled.map(|value| value == 0)
    );
}

#[test]
fn a_current_user_value_is_read_back_as_it_was_written() {
    let key = format!(r"Software\PeekBuddyTest\osinfo{}", std::process::id());
    let set = |value: &str, data: &str| {
        let status = Command::new("reg")
            .args([
                "add",
                &format!(r"HKCU\{key}"),
                "/v",
                value,
                "/t",
                "REG_DWORD",
                "/d",
                data,
                "/f",
            ])
            .output()
            .unwrap()
            .status;
        assert!(status.success());
    };
    set("Zero", "0");
    set("One", "1");
    set("Seven", "7");
    set("Big", "4000000000");
    assert_eq!(current_user_dword(&key, "Zero"), Some(0));
    assert_eq!(current_user_dword(&key, "One"), Some(1));
    assert_eq!(current_user_dword(&key, "Seven"), Some(7));
    assert_eq!(current_user_dword(&key, "Big"), Some(4_000_000_000));
    assert_eq!(current_user_dword(&key, "Missing"), None);
    assert_eq!(
        current_user_dword(r"Software\PeekBuddyTest\nowhere", "Zero"),
        None
    );
    let _ = Command::new("reg")
        .args(["delete", &format!(r"HKCU\{key}"), "/f"])
        .output();
}
