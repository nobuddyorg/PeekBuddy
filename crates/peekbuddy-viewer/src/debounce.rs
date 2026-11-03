//! A pause after the last change (#67): the filter is applied when the user
//! stops typing, not on every key.
//!
//! The clock is a parameter, so the rule is a unit test. The viewer asks for a
//! repaint after the delay (`request_repaint_after`), so no timer thread is
//! needed and an idle viewer paints nothing.

use std::time::{Duration, Instant};

/// How long the user has to stop typing.
pub const DELAY: Duration = Duration::from_millis(50);

/// The pause after the last change.
#[derive(Debug, Clone, Copy)]
pub struct Debounce {
    delay: Duration,
    last: Option<Instant>,
}

impl Debounce {
    /// A debounce of `delay`.
    #[must_use]
    pub const fn new(delay: Duration) -> Self {
        Self { delay, last: None }
    }

    /// Something changed at `now`: the wait starts over.
    pub fn touch(&mut self, now: Instant) {
        self.last = Some(now);
    }

    /// Whether a change waits for the pause.
    #[must_use]
    pub const fn pending(&self) -> bool {
        self.last.is_some()
    }

    /// Whether the pause is over at `now`; true once per change.
    pub fn due(&mut self, now: Instant) -> bool {
        match self.last {
            Some(last) if now.saturating_duration_since(last) >= self.delay => {
                self.last = None;
                true
            }
            _ => false,
        }
    }

    /// How long until the pause is over at `now`, if a change waits.
    #[must_use]
    pub fn remaining(&self, now: Instant) -> Option<Duration> {
        self.last.map(|last| {
            self.delay
                .saturating_sub(now.saturating_duration_since(last))
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::Debounce;

    #[test]
    fn it_fires_once_after_the_pause_and_not_before() {
        let start = Instant::now();
        let mut debounce = Debounce::new(Duration::from_millis(50));
        assert!(!debounce.due(start), "nothing changed");
        debounce.touch(start);
        assert!(debounce.pending());
        assert!(!debounce.due(start + Duration::from_millis(49)));
        assert!(debounce.due(start + Duration::from_millis(50)));
        assert!(
            !debounce.due(start + Duration::from_millis(500)),
            "once per change"
        );
        assert!(!debounce.pending());
    }

    #[test]
    fn ten_keystrokes_in_200_ms_make_one_query() {
        let start = Instant::now();
        let mut debounce = Debounce::new(Duration::from_millis(50));
        let mut fired = 0;
        for key in 0..10 {
            let now = start + Duration::from_millis(key * 20);
            debounce.touch(now);
            if debounce.due(now) {
                fired += 1;
            }
        }
        // The user stops; the pause passes.
        if debounce.due(start + Duration::from_millis(180 + 50)) {
            fired += 1;
        }
        assert_eq!(fired, 1);
    }

    #[test]
    fn the_wait_counts_down() {
        let start = Instant::now();
        let mut debounce = Debounce::new(Duration::from_millis(50));
        assert_eq!(debounce.remaining(start), None);
        debounce.touch(start);
        assert_eq!(
            debounce.remaining(start + Duration::from_millis(20)),
            Some(Duration::from_millis(30))
        );
        assert_eq!(
            debounce.remaining(start + Duration::from_millis(80)),
            Some(Duration::ZERO)
        );
    }
}
