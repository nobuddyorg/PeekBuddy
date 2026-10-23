//! SQLite day-segment event store for PeekBuddy.
//!
//! One writer thread with group commit, one file per day, one global
//! contiguous event id sequence (ADR 0004).
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    #[test]
    fn links_core() {
        assert!(peekbuddy_core::VERSION.contains('.'));
    }
}
