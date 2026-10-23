//! Thin wrappers over `windows-sys` for PeekBuddy.
//!
//! This is the only crate allowed to contain `unsafe` code. Every `unsafe`
//! block carries a `// SAFETY:` comment. OS-independent record parsers
//! (FILE_NOTIFY and USN record views) live in modules that also compile on
//! Linux, so Miri can run them; Win32 calls are gated on `cfg(windows)` and
//! exposed through traits with in-memory fakes.
#![deny(clippy::indexing_slicing)]

pub mod perf;

#[cfg(windows)]
pub mod process;
#[cfg(windows)]
pub mod session;
#[cfg(windows)]
mod sid;
#[cfg(windows)]
pub mod token;

#[cfg(test)]
mod tests {
    #[test]
    fn builds_on_every_host() {
        assert_eq!(env!("CARGO_PKG_NAME"), "peekbuddy-win");
    }
}
