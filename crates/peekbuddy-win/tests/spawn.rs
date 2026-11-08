//! Starting a program for a rule (`spawn`, #81): the exit code comes back, a
//! program that outlives its timeout is killed with the job, and a program
//! that is not there is an error and not a search of `PATH`.
#![cfg(windows)]

use std::time::{Duration, Instant};

use peekbuddy_win::spawn::spawn;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn cmd() -> String {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| String::from(r"C:\Windows"));
    format!(r"{root}\System32\cmd.exe")
}

#[test]
fn the_exit_code_comes_back() {
    let program = cmd();
    let line = format!("\"{program}\" /c exit 7");
    let child = spawn(&wide(&program), &wide(&line)).unwrap();
    assert_eq!(child.wait(10_000).unwrap(), Some(7));
}

#[test]
fn a_program_that_runs_on_is_still_running_and_dies_with_the_job() {
    let program = cmd();
    let line = format!("\"{program}\" /c ping -n 60 127.0.0.1 >nul");
    let child = spawn(&wide(&program), &wide(&line)).unwrap();
    let started = Instant::now();
    assert_eq!(child.wait(300).unwrap(), None);
    child.kill();
    assert!(started.elapsed() < Duration::from_secs(20));
}

#[test]
fn a_missing_program_is_an_error_and_a_bare_name_is_not_searched() {
    assert!(spawn(&wide(r"C:\no\such\dir\x.exe"), &wide("x")).is_err());
    assert!(spawn(&wide("cmd.exe"), &wide("cmd.exe /c exit 0")).is_err());
}

#[test]
fn a_nul_is_refused() {
    let err = spawn(&[0x63, 0, 0x64], &wide("x")).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}
