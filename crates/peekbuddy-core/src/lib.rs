//! Source-agnostic event model for PeekBuddy.
//!
//! This crate is pure Rust: no Win32, no `cfg(windows)`, no `unsafe`. It
//! builds and tests on Linux so Miri and fuzzing can run against it
//! (see `docs/architecture.md`).
#![forbid(unsafe_code)]
#![deny(clippy::indexing_slicing)]

pub mod autostart;
pub mod capture;
pub mod derive;
pub mod event;
pub mod filter;
pub mod ipc;
pub mod lifecycle;
pub mod name;
pub mod notify;
pub mod pause;
pub mod perf;
pub mod reconcile;
pub mod reparse;
pub mod settings;

/// Crate version, shared by every PeekBuddy crate.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_set() {
        assert!(super::VERSION.contains('.'));
    }
}
