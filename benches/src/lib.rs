//! Micro-benchmarks for PeekBuddy.
//!
//! The benchmark harness and the same-runner A/B gate arrive with #26.
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder() {
        assert_eq!(env!("CARGO_PKG_NAME"), "benches");
    }
}
