//! Thin wrappers over `windows-sys` for PeekBuddy.
//!
//! This is the only crate allowed to contain `unsafe` code. Every `unsafe`
//! block carries a `// SAFETY:` comment. OS-independent record parsers
//! (FILE_NOTIFY and USN record views) live in modules that also compile on
//! Linux, so Miri can run them; Win32 calls are gated on `cfg(windows)` and
//! exposed through traits with in-memory fakes.
#![deny(clippy::indexing_slicing)]

pub mod fs;
pub mod io;
pub mod metrics;
pub mod perf;

#[cfg(windows)]
pub mod atomic;
#[cfg(windows)]
pub mod attrs;
#[cfg(windows)]
pub mod autostart;
#[cfg(windows)]
pub mod console;
#[cfg(windows)]
pub mod job;
#[cfg(windows)]
pub mod localtime;
#[cfg(windows)]
pub mod osinfo;
#[cfg(windows)]
pub mod pipe;
#[cfg(windows)]
pub mod pnp;
#[cfg(windows)]
pub mod probe;
#[cfg(windows)]
pub mod process;
#[cfg(windows)]
pub mod rdcw;
#[cfg(windows)]
pub mod reconcile;
#[cfg(windows)]
pub mod resources;
#[cfg(windows)]
pub mod session;
#[cfg(windows)]
mod sid;
#[cfg(windows)]
pub mod single_instance;
#[cfg(windows)]
pub mod timer;
#[cfg(windows)]
pub mod token;
#[cfg(windows)]
pub mod tray;
#[cfg(windows)]
pub mod window;

#[cfg(test)]
mod tests {
    #[test]
    fn builds_on_every_host() {
        assert_eq!(env!("CARGO_PKG_NAME"), "peekbuddy-win");
    }
}
