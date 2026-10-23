//! The PeekBuddy engine role: tray, watches, reconciliation and the writer.
//!
//! The engine never depends on a GPU or UI stack (ADR 0001); `cargo xtask
//! check-deps` enforces that.
#![forbid(unsafe_code)]

/// Runs the engine role until it is asked to exit.
///
/// The message loop, tray and watches arrive with #59 and #60.
pub fn run() {}

#[cfg(test)]
mod tests {
    #[test]
    fn run_returns() {
        super::run();
    }
}
