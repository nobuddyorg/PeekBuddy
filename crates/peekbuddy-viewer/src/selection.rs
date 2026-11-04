//! Which rows are selected (#66), by virtual index.
//!
//! One anchor and one cursor, like every list: a click sets both, Shift moves
//! only the cursor, and the selection is everything between them. The
//! keyboard moves the cursor by a row, a page, to the first or to the last
//! row. Pure, so the keys are unit tests.

use std::ops::RangeInclusive;

/// A keyboard move of the cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Arrow up: one row towards the newest.
    Up,
    /// Arrow down: one row towards the oldest.
    Down,
    /// Page up: this many rows towards the newest.
    PageUp(u64),
    /// Page down: this many rows towards the oldest.
    PageDown(u64),
    /// Home: the newest row.
    Home,
    /// End: the oldest row.
    End,
}

/// The selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Selection {
    anchor: Option<u64>,
    cursor: Option<u64>,
}

impl Selection {
    /// Nothing selected.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// A click on row `index`; with `shift` the anchor stays.
    pub fn click(&mut self, index: u64, shift: bool) {
        if !shift || self.anchor.is_none() {
            self.anchor = Some(index);
        }
        self.cursor = Some(index);
    }

    /// Moves the cursor in a view of `total` rows; with `shift` the anchor
    /// stays. Without a selection the first move lands on the first row (for
    /// moves towards the newest on the last one). Returns the cursor.
    pub fn step(&mut self, step: Step, total: u64, shift: bool) -> Option<u64> {
        let last = total.checked_sub(1)?;
        let target = match (self.cursor, step) {
            (_, Step::Home) | (None, Step::Down | Step::PageDown(_)) => 0,
            (_, Step::End) | (None, Step::Up | Step::PageUp(_)) => last,
            (Some(at), Step::Up) => at.saturating_sub(1),
            (Some(at), Step::Down) => at.saturating_add(1).min(last),
            (Some(at), Step::PageUp(rows)) => at.saturating_sub(rows.max(1)),
            (Some(at), Step::PageDown(rows)) => at.saturating_add(rows.max(1)).min(last),
        };
        self.click(target, shift);
        Some(target)
    }

    /// The cursor, the row the keyboard acts on.
    #[must_use]
    pub const fn cursor(&self) -> Option<u64> {
        self.cursor
    }

    /// The selected rows, lowest index first.
    #[must_use]
    pub fn range(&self) -> Option<RangeInclusive<u64>> {
        let (a, b) = (self.anchor?, self.cursor?);
        Some(a.min(b)..=a.max(b))
    }

    /// Whether row `index` is selected.
    #[must_use]
    pub fn contains(&self, index: u64) -> bool {
        self.range().is_some_and(|range| range.contains(&index))
    }

    /// `rows` new rows were put in front of the selection.
    pub fn shift_down(&mut self, rows: u64) {
        self.anchor = self.anchor.map(|at| at.saturating_add(rows));
        self.cursor = self.cursor.map(|at| at.saturating_add(rows));
    }
}

#[cfg(test)]
mod tests {
    use super::{Selection, Step};

    #[test]
    fn a_click_selects_one_row_and_shift_extends_from_the_anchor() {
        let mut selection = Selection::default();
        assert_eq!(selection.range(), None);
        selection.click(10, false);
        assert_eq!(selection.range(), Some(10..=10));
        selection.click(14, true);
        assert_eq!(selection.range(), Some(10..=14));
        selection.click(5, true);
        assert_eq!(selection.range(), Some(5..=10), "the anchor stays at 10");
        assert!(selection.contains(7) && !selection.contains(11));
        selection.click(20, false);
        assert_eq!(selection.range(), Some(20..=20));
    }

    #[test]
    fn shift_without_an_anchor_starts_one() {
        let mut selection = Selection::default();
        selection.click(3, true);
        assert_eq!(selection.range(), Some(3..=3));
    }

    #[test]
    fn the_arrows_move_the_cursor_and_stop_at_the_ends() {
        let mut selection = Selection::default();
        assert_eq!(selection.step(Step::Down, 100, false), Some(0));
        assert_eq!(selection.step(Step::Down, 100, false), Some(1));
        assert_eq!(selection.step(Step::Up, 100, false), Some(0));
        assert_eq!(
            selection.step(Step::Up, 100, false),
            Some(0),
            "stops at the newest"
        );
        assert_eq!(selection.step(Step::End, 100, false), Some(99));
        assert_eq!(
            selection.step(Step::Down, 100, false),
            Some(99),
            "stops at the oldest"
        );
        assert_eq!(selection.step(Step::Home, 100, false), Some(0));
    }

    #[test]
    fn pages_move_by_the_rows_in_view_and_clamp() {
        let mut selection = Selection::default();
        selection.click(50, false);
        assert_eq!(selection.step(Step::PageDown(30), 100, false), Some(80));
        assert_eq!(selection.step(Step::PageDown(30), 100, false), Some(99));
        assert_eq!(selection.step(Step::PageUp(30), 100, false), Some(69));
        assert_eq!(
            selection.step(Step::PageUp(0), 100, false),
            Some(68),
            "at least one row"
        );
    }

    #[test]
    fn shift_with_a_key_extends_the_selection() {
        let mut selection = Selection::default();
        selection.click(10, false);
        selection.step(Step::Down, 100, true);
        selection.step(Step::Down, 100, true);
        assert_eq!(selection.range(), Some(10..=12));
        selection.step(Step::End, 100, true);
        assert_eq!(selection.range(), Some(10..=99));
    }

    #[test]
    fn a_first_move_up_starts_at_the_end_and_an_empty_view_moves_nowhere() {
        let mut selection = Selection::default();
        assert_eq!(selection.step(Step::Up, 10, false), Some(9));
        let mut selection = Selection::default();
        assert_eq!(selection.step(Step::Down, 0, false), None);
        assert_eq!(selection.cursor(), None);
    }

    #[test]
    fn rows_put_in_front_move_the_selection_with_its_rows() {
        let mut selection = Selection::default();
        selection.click(5, false);
        selection.click(8, true);
        selection.shift_down(3);
        assert_eq!(selection.range(), Some(8..=11));
        selection.clear();
        selection.shift_down(3);
        assert_eq!(selection.range(), None);
    }
}
