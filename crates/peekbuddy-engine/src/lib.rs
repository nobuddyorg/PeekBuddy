//! The PeekBuddy engine role: tray, watches, reconciliation and the writer.
//!
//! The engine never depends on a GPU or UI stack (ADR 0001); `cargo xtask
//! check-deps` enforces that.
#![forbid(unsafe_code)]

/// Runs the engine role until it is asked to exit.
///
/// The message loop, tray and watches arrive with #59 and #60.
#[must_use]
pub fn run() -> std::process::ExitCode {
    std::process::ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    #[test]
    fn run_succeeds() {
        assert_eq!(super::run(), std::process::ExitCode::SUCCESS);
    }
}
