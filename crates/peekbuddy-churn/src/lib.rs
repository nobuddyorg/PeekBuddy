//! Synthetic file-system churn with a ground-truth ledger (#25).
//!
//! Dev-only, never shipped. The pieces that need no Win32 live here and
//! build and test on Linux and under Miri: the PRNG ([`rng`]), the operation
//! model ([`ops`]), the named workloads ([`profile`]), deterministic plans
//! ([`plan`]), the JSONL ledger ([`ledger`]), the comparator against
//! PeekBuddy's log ([`compare`]) and the executor loop ([`exec`]), whose
//! Windows target and clock run the plans for real (`cargo xtask churn`).
//! See `docs/how-to/churn.md`.
#![forbid(unsafe_code)]
#![deny(clippy::indexing_slicing)]

pub mod compare;
pub mod exec;
pub mod ledger;
pub mod ops;
pub mod plan;
pub mod profile;
pub mod rng;
