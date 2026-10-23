//! Process metrics against real work (Windows only, #24).
//!
//! nextest runs every test in its own process, so the deltas below are this
//! test's own work.
#![cfg(windows)]

use std::hint::black_box;
use std::os::windows::io::AsHandle as _;
use std::time::{Duration, Instant};

use peekbuddy_win::metrics::{
    ContextSwitches, arch, cpu_times, current_process, cycle_time, gui_objects, handle_count, io,
    memory, open_process,
};

const MB: u64 = 1024 * 1024;

#[test]
fn allocating_50_mb_raises_private_bytes() {
    let before = memory(current_process()).unwrap();
    // Touch every page so the commit is real; black_box keeps it alive.
    let block = black_box(vec![1u8; 50 * 1024 * 1024]);
    let after = memory(current_process()).unwrap();
    assert!(
        after.private_bytes >= before.private_bytes + 50 * MB,
        "private bytes {} -> {}",
        before.private_bytes,
        after.private_bytes
    );
    assert!(after.working_set > before.working_set);
    assert!(after.peak_working_set >= after.working_set);
    assert!(after.page_faults > before.page_faults);
    drop(block);
}

#[test]
fn spinning_200_ms_raises_user_time() {
    let before = cpu_times(current_process()).unwrap();
    let cycles_before = cycle_time(current_process()).unwrap();
    std::thread::spawn(|| {
        let start = Instant::now();
        let mut x = 0u64;
        while start.elapsed() < Duration::from_millis(200) {
            x = black_box(x.wrapping_add(1));
        }
    })
    .join()
    .unwrap();
    let after = cpu_times(current_process()).unwrap();
    // 150 ms in 100 ns ticks.
    assert!(
        after.user >= before.user + 1_500_000,
        "user time {} -> {}",
        before.user,
        after.user
    );
    assert!(after.kernel >= before.kernel);
    assert_eq!(after.creation, before.creation);
    assert!(cycle_time(current_process()).unwrap() > cycles_before);
}

#[test]
fn opening_100_handles_raises_the_count_by_100() {
    let exe = std::env::current_exe().unwrap();
    let before = handle_count(current_process()).unwrap();
    let files: Vec<_> = (0..100)
        .map(|_| std::fs::File::open(&exe).unwrap())
        .collect();
    assert_eq!(handle_count(current_process()).unwrap(), before + 100);
    drop(files);
    assert_eq!(handle_count(current_process()).unwrap(), before);
}

#[test]
fn reading_a_file_raises_read_counters() {
    let exe = std::env::current_exe().unwrap();
    let before = io(current_process()).unwrap();
    let data = std::fs::read(&exe).unwrap();
    let after = io(current_process()).unwrap();
    let len = u64::try_from(data.len()).unwrap();
    assert!(after.read_bytes >= before.read_bytes + len);
    assert!(after.read_ops > before.read_ops);
    assert!(after.write_bytes >= before.write_bytes);
}

#[test]
fn context_switches_grow_with_sleeps() {
    let mut sampler = ContextSwitches::new();
    let pid = std::process::id();
    let before = sampler.sample(pid).unwrap();
    for _ in 0..20 {
        std::thread::sleep(Duration::from_millis(2));
    }
    let after = sampler.sample(pid).unwrap();
    // Each sleep blocks the thread, so it is switched in again afterwards.
    assert!(after >= before + 20, "context switches {before} -> {after}");
    assert_eq!(sampler.sample(u32::MAX - 3), None);
}

#[test]
fn another_process_can_be_sampled_from_outside() {
    let mut child = std::process::Command::new("cmd.exe")
        .args(["/d", "/c", "ping -n 3 127.0.0.1 >nul"])
        .spawn()
        .unwrap();
    let handle = open_process(child.id()).unwrap();
    let process = handle.as_handle();
    assert!(memory(process).unwrap().private_bytes > 0);
    assert!(cpu_times(process).unwrap().creation > 0);
    assert!(handle_count(process).unwrap() > 0);
    assert!(io(process).is_ok());
    assert!(arch(process).is_ok());
    let _ = gui_objects(process);
    assert!(ContextSwitches::new().sample(child.id()).is_some());
    child.wait().unwrap();
    assert!(open_process(u32::MAX - 3).is_err());
}

#[test]
fn the_test_binary_runs_natively() {
    let arch = arch(current_process()).unwrap();
    assert!(!arch.emulated(), "{arch:?}");
}

#[test]
fn a_gdi_brush_is_counted() {
    use windows_sys::Win32::Graphics::Gdi::{CreateSolidBrush, DeleteObject};
    let before = gui_objects(current_process()).gdi;
    // SAFETY: plain call; the brush is deleted below.
    let brush = unsafe { CreateSolidBrush(0x00FF_8000) };
    assert!(!brush.is_null());
    assert_eq!(gui_objects(current_process()).gdi, before + 1);
    // SAFETY: `brush` is a brush this test created and no longer uses.
    assert_ne!(unsafe { DeleteObject(brush) }, 0);
    assert_eq!(gui_objects(current_process()).gdi, before);
}

#[test]
fn a_handle_that_is_not_a_process_is_an_error() {
    let file = std::fs::File::open(std::env::current_exe().unwrap()).unwrap();
    let not_a_process = file.as_handle();
    assert!(memory(not_a_process).is_err());
    assert!(cpu_times(not_a_process).is_err());
    assert!(cycle_time(not_a_process).is_err());
    assert!(handle_count(not_a_process).is_err());
    assert!(io(not_a_process).is_err());
    assert!(arch(not_a_process).is_err());
}
