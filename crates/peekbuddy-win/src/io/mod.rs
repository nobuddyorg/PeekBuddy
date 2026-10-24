//! Safe Win32 primitives for capture (#43).
//!
//! Every Win32 call the capture engine makes goes through this crate, so
//! unsafe code is concentrated and every crate above can forbid it. The pure
//! parts (the error table, path building) build and test on every host; the
//! rest is Windows only.
//!
//! - [`error`]: [`WinError`](error::WinError), Win32 codes as meaning.
//! - [`path`]: long (`\\?\`) paths and path classes.
//! - [`open`]: the flags for opening a directory.
//! - `dir`: the directory-open helper and `DirHandle`.
//! - `iocp`: an I/O completion port with batch dequeue.
//! - `slot`: `IoSlot`, an OVERLAPPED and its buffer, pinned while I/O is
//!   pending.
//! - `volume`: volume identity and final path names.

pub mod error;
pub mod open;
pub mod path;

#[cfg(windows)]
pub mod dir;
#[cfg(windows)]
pub mod iocp;
#[cfg(windows)]
pub mod slot;
#[cfg(windows)]
mod sys;
#[cfg(windows)]
pub mod volume;
