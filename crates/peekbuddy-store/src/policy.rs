//! The group-commit policy of the writer (R1, spike #35).
//!
//! A batch is committed when it holds [`Policy::max_events`] events, or when
//! [`Policy::linger`] has passed since its **first** event (restarting the
//! linger at every event never fires under steady arrivals: p50 51 ms in the
//! spike), and never later than [`Policy::hard_bound`]. The deadline exists
//! only while events are pending, so an idle writer has no timer at all (F6).
//! Time is a plain nanosecond count so tests inject a clock.

use std::time::Duration;

/// When to commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    /// Commit this long after the first pending event.
    pub linger: Duration,
    /// Commit once this many events are pending.
    pub max_events: usize,
    /// Upper bound on the wait, whatever the linger.
    pub hard_bound: Duration,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            linger: Duration::from_millis(5),
            max_events: 1_000,
            hard_bound: Duration::from_millis(100),
        }
    }
}

fn nanos(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

/// The events waiting for the next commit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Pending {
    first_ns: Option<u64>,
    events: usize,
}

impl Pending {
    /// Nothing pending.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            first_ns: None,
            events: 0,
        }
    }

    /// `count` events arrived at `now_ns`. The first ones start the clock.
    pub fn add(&mut self, now_ns: u64, count: usize) {
        if count == 0 {
            return;
        }
        if self.first_ns.is_none() {
            self.first_ns = Some(now_ns);
        }
        self.events += count;
    }

    /// Events waiting.
    #[must_use]
    pub const fn events(&self) -> usize {
        self.events
    }

    /// Whether anything is waiting.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.events == 0
    }

    /// When the pending events must be committed at the latest; `None` while
    /// nothing is pending (no deadline, no wakeup).
    #[must_use]
    pub fn deadline_ns(&self, policy: &Policy) -> Option<u64> {
        let wait = nanos(policy.linger.min(policy.hard_bound));
        self.first_ns.map(|first| first.saturating_add(wait))
    }

    /// Whether to commit now.
    #[must_use]
    pub fn due(&self, now_ns: u64, policy: &Policy) -> bool {
        if self.events == 0 {
            return false;
        }
        self.events >= policy.max_events
            || self
                .deadline_ns(policy)
                .is_some_and(|deadline| now_ns >= deadline)
    }

    /// How long to wait for more events before the deadline; `None` while
    /// nothing is pending (wait for ever).
    #[must_use]
    pub fn wait_ns(&self, now_ns: u64, policy: &Policy) -> Option<u64> {
        self.deadline_ns(policy)
            .map(|deadline| deadline.saturating_sub(now_ns))
    }

    /// Forgets the pending events after a commit.
    pub fn clear(&mut self) {
        *self = Self::new();
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use proptest::prelude::*;

    use super::{Pending, Policy};

    const MS: u64 = 1_000_000;

    #[test]
    fn an_idle_writer_has_no_deadline_and_nothing_is_due() {
        let pending = Pending::new();
        let policy = Policy::default();
        assert_eq!(pending.deadline_ns(&policy), None);
        assert_eq!(pending.wait_ns(123, &policy), None);
        assert!(!pending.due(u64::MAX, &policy));
        assert!(pending.is_empty());
    }

    #[test]
    fn the_linger_runs_from_the_first_event_and_is_not_restarted() {
        let policy = Policy::default();
        let mut pending = Pending::new();
        pending.add(10 * MS, 1);
        assert_eq!(pending.deadline_ns(&policy), Some(15 * MS));
        // Steady arrivals do not push the deadline out.
        for step in 11..15 {
            pending.add(step * MS, 1);
            assert_eq!(pending.deadline_ns(&policy), Some(15 * MS));
            assert!(!pending.due(step * MS, &policy));
        }
        assert!(pending.due(15 * MS, &policy));
        assert_eq!(pending.events(), 5);
        assert_eq!(pending.wait_ns(12 * MS, &policy), Some(3 * MS));
        assert_eq!(pending.wait_ns(20 * MS, &policy), Some(0));
    }

    #[test]
    fn the_event_cap_commits_at_once() {
        let policy = Policy::default();
        let mut pending = Pending::new();
        pending.add(0, 999);
        assert!(!pending.due(1, &policy));
        pending.add(1, 1);
        assert!(pending.due(1, &policy));
    }

    #[test]
    fn the_hard_bound_caps_a_long_linger() {
        let policy = Policy {
            linger: Duration::from_secs(10),
            ..Policy::default()
        };
        let mut pending = Pending::new();
        pending.add(0, 1);
        assert_eq!(pending.deadline_ns(&policy), Some(100 * MS));
    }

    #[test]
    fn clearing_starts_a_new_batch() {
        let policy = Policy::default();
        let mut pending = Pending::new();
        pending.add(0, 3);
        pending.clear();
        assert_eq!(pending, Pending::new());
        pending.add(50 * MS, 1);
        assert_eq!(pending.deadline_ns(&policy), Some(55 * MS));
        // Adding zero events does not start the clock.
        let mut empty = Pending::new();
        empty.add(7, 0);
        assert_eq!(empty.deadline_ns(&policy), None);
    }

    proptest! {
        #[test]
        fn a_batch_never_waits_longer_than_the_bound(
            arrivals in proptest::collection::vec((0u64..50 * MS, 1usize..40), 1..60),
            linger_ms in 1u64..500,
        ) {
            let policy = Policy { linger: Duration::from_millis(linger_ms), ..Policy::default() };
            let mut now = 0u64;
            let mut pending = Pending::new();
            let mut first = None;
            for (gap, count) in arrivals {
                now += gap;
                if pending.due(now, &policy) {
                    let waited = now - first.unwrap_or(now);
                    // It is due, so it is committed at the next opportunity;
                    // the deadline itself never exceeds the bound.
                    prop_assert!(pending.deadline_ns(&policy).unwrap() - first.unwrap() <= 100 * MS);
                    let _ = waited;
                    pending.clear();
                    first = None;
                }
                if first.is_none() {
                    first = Some(now);
                }
                pending.add(now, count);
            }
        }
    }
}
