//! Freezing a process (`suspend_process`, #50, #234): a child that makes
//! progress stops making progress while it is suspended, and goes on when the
//! guard is dropped.
#![cfg(windows)]
#![expect(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns may unwrap"
)]

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use peekbuddy_win::suspend::suspend_process;

/// The child: appends a line to the file named by `PEEKBUDDY_SUSPEND_FILE`
/// every 10 ms, for a minute.
#[test]
#[ignore = "run as the child of `suspending_a_process_stops_its_progress_and_dropping_resumes_it`"]
fn child_makes_progress() {
    let path = PathBuf::from(std::env::var_os("PEEKBUDDY_SUSPEND_FILE").unwrap());
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(60) {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(file, "x").unwrap();
        std::thread::sleep(Duration::from_millis(10));
    }
}

struct Killed(Child);

impl Drop for Killed {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn size(path: &Path) -> u64 {
    std::fs::metadata(path).map_or(0, |meta| meta.len())
}

/// Waits until the file is at least `more` bytes bigger than `before`.
fn grows(path: &Path, before: u64, more: u64) -> bool {
    let until = Instant::now() + Duration::from_secs(10);
    while Instant::now() < until {
        if size(path) >= before + more {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
fn suspending_a_process_stops_its_progress_and_dropping_resumes_it() {
    let dir = std::env::temp_dir().join(format!("pb-suspend-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let progress = dir.join("progress.txt");
    let child = Killed(
        Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "child_makes_progress"])
            .env("PEEKBUDDY_SUSPEND_FILE", &progress)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    assert!(grows(&progress, 0, 20), "the child makes no progress");

    let frozen = suspend_process(child.0.id()).unwrap();
    // A test binary has its main thread and the test's: more than one.
    assert!(frozen.threads() >= 2, "{} threads", frozen.threads());
    // Let a write that was under way finish, then it must stand still.
    std::thread::sleep(Duration::from_millis(300));
    let at_rest = size(&progress);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(
        size(&progress),
        at_rest,
        "the child went on while suspended"
    );

    drop(frozen);
    assert!(
        grows(&progress, at_rest, 20),
        "the child did not go on after the guard was dropped"
    );
    drop(child);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_process_that_does_not_exist_has_no_threads_to_suspend() {
    // Pid 0 is the idle process: no thread can be opened for it.
    let frozen = suspend_process(u32::MAX - 3).unwrap();
    assert_eq!(frozen.threads(), 0);
}
