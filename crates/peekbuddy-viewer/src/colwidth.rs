//! Saved column widths (#347): what the grid restores at start and when it
//! writes the widths back.
//!
//! The rule is pure so it is a unit test on Linux. Only a width the user
//! changed counts: `egui_table` also adjusts widths to the window, and a
//! window that is resized or a log that is scrolled must not write the
//! settings. A change waits for a pause ([`SETTLE`]) so a drag is one write,
//! never one per frame.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use peekbuddy_core::settings::COLUMN_WIDTH;

use crate::debounce::Debounce;

/// How long the widths have to stay still before they are written.
pub const SETTLE: Duration = Duration::from_millis(600);

/// The widths of the columns by name, and the write that waits for a pause.
#[derive(Debug, Clone)]
pub struct Widths {
    /// Widths as last restored or reported, in pixels.
    known: BTreeMap<String, u32>,
    settle: Debounce,
}

impl Default for Widths {
    fn default() -> Self {
        Self::new(&BTreeMap::new())
    }
}

impl Widths {
    /// The widths of `[viewer] column_widths`; entries outside the valid
    /// range or of unknown columns are dropped (validation refuses them in
    /// a file, this keeps a hand-built map honest too).
    #[must_use]
    pub fn new(saved: &BTreeMap<String, u32>) -> Self {
        Self {
            known: saved
                .iter()
                .filter(|(name, width)| {
                    peekbuddy_core::settings::COLUMNS.contains(&name.as_str())
                        && COLUMN_WIDTH.contains(width)
                })
                .map(|(name, width)| (name.clone(), *width))
                .collect(),
            settle: Debounce::new(SETTLE),
        }
    }

    /// The saved width of the column `name`, in pixels.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<f32> {
        // Widths are at most 4096: exact in `f32`.
        #[expect(clippy::cast_precision_loss, reason = "at most 4096, exact")]
        self.known.get(name).map(|width| *width as f32)
    }

    /// Whether a width is saved at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.known.is_empty()
    }

    /// Looks at the current widths of the visible columns. `user` says the
    /// pointer is down or was just released, i.e. the user may be dragging a
    /// column edge. A change is only taken over then.
    pub fn observe<'a>(
        &mut self,
        current: impl IntoIterator<Item = (&'a str, f32)>,
        user: bool,
        now: Instant,
    ) {
        if !user {
            return;
        }
        let mut changed = false;
        for (name, width) in current {
            let Some(width) = clamp(width) else { continue };
            if self.known.get(name) != Some(&width) {
                self.known.insert(name.to_owned(), width);
                changed = true;
            }
        }
        if changed {
            self.settle.touch(now);
        }
    }

    /// The widths to write once they have been still for [`SETTLE`]; `Some`
    /// once per change.
    pub fn take_due(&mut self, now: Instant) -> Option<BTreeMap<String, u32>> {
        self.settle.due(now).then(|| self.known.clone())
    }

    /// How long until a waiting change is due, to ask for one repaint.
    #[must_use]
    pub fn remaining(&self, now: Instant) -> Option<Duration> {
        self.settle.remaining(now)
    }
}

/// A width as whole pixels, if it is one the settings accept.
fn clamp(width: f32) -> Option<u32> {
    if !width.is_finite() {
        return None;
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "rounded, then checked against the valid range"
    )]
    let pixels = width.round().max(0.0) as u32;
    COLUMN_WIDTH.contains(&pixels).then_some(pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ms: u64, base: Instant) -> Instant {
        base + Duration::from_millis(ms)
    }

    #[test]
    fn saved_widths_come_back_and_bad_entries_are_dropped() {
        let saved = BTreeMap::from([
            ("path".to_owned(), 640),
            ("time".to_owned(), 5),
            ("nope".to_owned(), 100),
        ]);
        let widths = Widths::new(&saved);
        assert_eq!(widths.get("path"), Some(640.0));
        assert_eq!(widths.get("time"), None);
        assert_eq!(widths.get("nope"), None);
        assert!(Widths::default().is_empty());
    }

    #[test]
    fn a_change_without_the_user_is_not_a_change() {
        let base = Instant::now();
        let mut widths = Widths::new(&BTreeMap::new());
        widths.observe([("path", 700.0)], false, base);
        assert!(widths.remaining(base).is_none());
        assert!(widths.take_due(at(5000, base)).is_none());
    }

    #[test]
    fn a_drag_is_written_once_after_the_pause() {
        let base = Instant::now();
        let mut widths = Widths::new(&BTreeMap::new());
        // Sixty frames of a drag, each a bit wider.
        for frame in 0..60u64 {
            #[expect(clippy::cast_precision_loss, reason = "a small test number")]
            let width = 500.0 + frame as f32;
            widths.observe(
                [("path", width), ("time", 180.0)],
                true,
                at(frame * 16, base),
            );
            assert!(
                widths.take_due(at(frame * 16, base)).is_none(),
                "no write mid-drag"
            );
        }
        let end = at(59 * 16, base);
        assert!(widths.remaining(end).is_some());
        assert!(widths.take_due(at(59 * 16 + 599, base)).is_none());
        let out = widths.take_due(at(59 * 16 + 600, base)).unwrap();
        assert_eq!(out.get("path"), Some(&559));
        assert_eq!(out.get("time"), Some(&180));
        assert!(
            widths.take_due(at(10_000, base)).is_none(),
            "once per change"
        );
    }

    #[test]
    fn the_same_width_again_writes_nothing() {
        let base = Instant::now();
        let mut widths = Widths::new(&BTreeMap::from([("path".to_owned(), 640)]));
        widths.observe([("path", 640.4)], true, base);
        assert!(widths.take_due(at(5000, base)).is_none());
    }

    #[test]
    fn widths_outside_the_range_are_never_taken_over() {
        let base = Instant::now();
        let mut widths = Widths::new(&BTreeMap::new());
        widths.observe(
            [
                ("a", f32::NAN),
                ("b", f32::INFINITY),
                ("c", 5.0),
                ("d", 9000.0),
            ],
            true,
            base,
        );
        assert!(widths.take_due(at(5000, base)).is_none());
    }

    #[test]
    fn an_unseen_column_keeps_the_saved_ones_in_the_output() {
        let base = Instant::now();
        let mut widths = Widths::new(&BTreeMap::from([("size".to_owned(), 120)]));
        widths.observe([("path", 600.0)], true, base);
        let out = widths.take_due(at(5000, base)).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out.get("size"), Some(&120));
    }
}
