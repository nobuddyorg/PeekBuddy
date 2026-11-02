//! A child under a restricted copy of the token (#277): it is not an
//! administrator (the group is deny-only and the privileges are gone),
//! whatever the parent is.
#![cfg(windows)]

use std::time::Duration;

use peekbuddy_win::token::{is_admin, run_restricted};

/// The child: ends with 0 when its token is the restricted one.
#[test]
#[ignore = "run as the child of `a_restricted_child_is_not_elevated`"]
fn child_checks_its_token() {
    assert!(!is_admin().unwrap(), "the child is an administrator");
}

#[test]
fn a_restricted_child_is_not_elevated() {
    let exe = std::env::current_exe().unwrap();
    let code = run_restricted(
        &exe,
        &["--ignored", "--exact", "child_checks_its_token"],
        Duration::from_secs(120),
    )
    .unwrap();
    assert_eq!(code, 0, "the child found an elevated token");
}

/// A child that ends with a code of its own.
#[test]
#[ignore = "run as the child of `the_exit_code_of_the_child_comes_back`"]
fn child_exits_with_seven() {
    std::process::exit(7);
}

/// A child that does not end.
#[test]
#[ignore = "run as the child of `a_child_that_does_not_end_is_terminated`"]
fn child_sleeps() {
    std::thread::sleep(Duration::from_secs(120));
}

#[test]
fn the_exit_code_of_the_child_comes_back() {
    let exe = std::env::current_exe().unwrap();
    let code = run_restricted(
        &exe,
        &["--ignored", "--exact", "child_exits_with_seven"],
        Duration::from_secs(60),
    )
    .unwrap();
    assert_eq!(code, 7);
}

#[test]
fn a_child_that_does_not_end_is_terminated() {
    let exe = std::env::current_exe().unwrap();
    let error = run_restricted(
        &exe,
        &["--ignored", "--exact", "child_sleeps"],
        Duration::from_millis(1_500),
    )
    .unwrap_err();
    assert_eq!(error.raw_os_error(), Some(1460));
}
