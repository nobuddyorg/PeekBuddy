//! The spill file after an engine quit (#277).
#![cfg(windows)]

use std::path::Path;
use std::time::{Duration, Instant};

use crate::engine::{quit_within, spawn, wait_for_window, wait_until_settled};

/// Rows still in the spill file when the engine quit are durable, and the
/// next start replays them (an engine that is quitting does not drain a
/// backlog first). Starts the engine again until the file is gone, so that
/// the log holds what the run produced before it is judged.
pub fn drain_spill(data: &Path) {
    let spill = data.join("spill").join("spill.bin");
    for _ in 0..5 {
        if !spill.exists() {
            return;
        }
        let mut engine = spawn(data);
        let window = wait_for_window(data, &mut engine.0);
        let deadline = Instant::now() + Duration::from_mins(30);
        while spill.exists() {
            assert!(
                Instant::now() < deadline,
                "the spill file was not replayed in 30 minutes"
            );
            std::thread::sleep(Duration::from_millis(500));
        }
        wait_until_settled(data, Duration::from_secs(5));
        quit_within(window, &mut engine.0, Duration::from_mins(10));
    }
}
