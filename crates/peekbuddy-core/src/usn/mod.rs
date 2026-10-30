//! The USN change journal as a capture source (#47), OS-independent part.
//!
//! - [`record`] decodes a buffer of journal output (versions 2 and 3, version
//!   4 skipped) without allocation per record.
//! - [`scope`] keeps the read subtree-scoped: the set of watched directories
//!   and the accept decision per record.
//! - [`checkpoint`] decides where a read resumes and when a gap is explicit.
//!
//! Reading the journal itself (handles, `FSCTL_*`, the IOCP reader) lives in
//! `peekbuddy-win` behind the `usn` feature, which stays off in v1 (ADR 0018).

pub mod checkpoint;
pub mod record;
pub mod scope;
