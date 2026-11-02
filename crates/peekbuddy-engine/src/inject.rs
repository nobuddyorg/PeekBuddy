//! Deliberate regressions for the perf harness's self-check (#269). Compiled
//! only with the `inject` feature, which no release has; `PEEKBUDDY_INJECT`
//! picks one. The harness must fail each of them, so a green harness is a
//! harness that can fail.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// What goes wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// A thread wakes once a second for nothing: the idle wake gate fails.
    Timer,
    /// Every pause or resume leaks a handle: handle stability fails.
    HandleLeak,
    /// Every 1,000th event is dropped without a gap: undetected loss fails.
    DropRow,
}

/// The mode named by `PEEKBUDDY_INJECT` (`timer`, `handle-leak`, `drop-row`).
#[must_use]
pub fn mode() -> Option<Mode> {
    static MODE: OnceLock<Option<Mode>> = OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("PEEKBUDDY_INJECT").ok().as_deref() {
        Some("timer") => Some(Mode::Timer),
        Some("handle-leak") => Some(Mode::HandleLeak),
        Some("drop-row") => Some(Mode::DropRow),
        _ => None,
    })
}

/// Starts the periodic wakeup when the mode is [`Mode::Timer`].
pub fn start_timer() {
    if mode() == Some(Mode::Timer) {
        let _ = std::thread::Builder::new()
            .name(String::from("inject-timer"))
            .spawn(|| {
                loop {
                    std::thread::sleep(Duration::from_secs(1));
                }
            });
    }
}

/// Leaks one handle when the mode is [`Mode::HandleLeak`].
pub fn leak_handle() {
    if mode() == Some(Mode::HandleLeak)
        && let Ok(file) = std::fs::File::open("NUL")
    {
        std::mem::forget(file);
    }
}

/// Whether this event is to be dropped silently (mode [`Mode::DropRow`]).
#[must_use]
pub fn drop_this_event() -> bool {
    static SEEN: AtomicU64 = AtomicU64::new(0);
    mode() == Some(Mode::DropRow) && SEEN.fetch_add(1, Ordering::Relaxed) % 1_000 == 999
}
