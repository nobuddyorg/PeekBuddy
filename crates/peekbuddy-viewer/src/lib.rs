//! The PeekBuddy viewer role: the on-demand egui window (`--viewer`).
#![forbid(unsafe_code)]

/// Runs the viewer role until its window is closed.
///
/// The window arrives with #65.
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
