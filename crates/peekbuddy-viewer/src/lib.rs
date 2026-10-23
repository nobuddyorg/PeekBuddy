//! The PeekBuddy viewer role: the on-demand egui window (`--viewer`).
#![forbid(unsafe_code)]

/// Runs the viewer role until its window is closed.
///
/// The window arrives with #65.
pub fn run() {}

#[cfg(test)]
mod tests {
    #[test]
    fn run_returns() {
        super::run();
    }
}
