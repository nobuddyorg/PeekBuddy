//! SQLite day-segment event store for PeekBuddy.
//!
//! One writer thread with group commit, one file per day, one global
//! contiguous event id sequence (ADR 0004). This crate holds the schema
//! (#53); the writer (#52), segments (#54) and queries (#55) build on it.
#![forbid(unsafe_code)]

pub mod batch;
pub mod catalog;
pub mod ids;
pub mod interner;
pub mod names;
pub mod pipeline;
pub mod policy;
pub mod read;
pub mod retention;
pub mod schema;
pub mod segment;
pub mod segments;
pub mod snapshot;
pub mod spill;
pub mod writer;
