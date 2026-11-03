//! Follow mode (#66): does the grid stay on the newest rows?
//!
//! Following means the view sits at the newest row and new rows push in
//! above it. The moment the user scrolls away the grid stops following and
//! only counts what arrives (the "N new events" pill); scrolling back to the
//! newest row, or clicking the pill, follows again.

/// Whether the grid follows the newest rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Follow {
    /// At the newest row; new rows appear.
    #[default]
    Following,
    /// Away from it; `new_rows` have arrived since.
    Paused {
        /// Rows that arrived while away.
        new_rows: u64,
    },
}

impl Follow {
    /// The view moved. `at_newest` is whether its first visible row is the
    /// newest one. Returns whether the mode changed.
    pub fn moved(&mut self, at_newest: bool) -> bool {
        match (*self, at_newest) {
            (Self::Following, false) => {
                *self = Self::Paused { new_rows: 0 };
                true
            }
            (Self::Paused { .. }, true) => {
                *self = Self::Following;
                true
            }
            _ => false,
        }
    }

    /// `count` rows arrived. They count only while paused.
    pub fn arrived(&mut self, count: u64) {
        if let Self::Paused { new_rows } = self {
            *new_rows = new_rows.saturating_add(count);
        }
    }

    /// The pill was clicked: follow again. Returns whether it was paused.
    pub fn resume(&mut self) -> bool {
        let was_paused = matches!(self, Self::Paused { .. });
        *self = Self::Following;
        was_paused
    }

    /// Whether the grid follows.
    #[must_use]
    pub const fn is_following(&self) -> bool {
        matches!(self, Self::Following)
    }

    /// Rows that arrived while away (0 while following).
    #[must_use]
    pub const fn new_rows(&self) -> u64 {
        match self {
            Self::Following => 0,
            Self::Paused { new_rows } => *new_rows,
        }
    }

    /// The text of the pill, if it is shown.
    #[must_use]
    pub fn pill(&self) -> Option<String> {
        match self {
            Self::Following => None,
            Self::Paused { new_rows: 0 } => Some(String::from("Jump to the newest")),
            Self::Paused { new_rows: 1 } => Some(String::from("1 new event")),
            Self::Paused { new_rows } => Some(format!("{new_rows} new events")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Follow;

    #[test]
    fn it_starts_following_and_pauses_when_the_user_scrolls_away() {
        let mut follow = Follow::default();
        assert!(follow.is_following());
        assert!(!follow.moved(true), "still at the newest row: no change");
        assert!(follow.moved(false));
        assert_eq!(follow, Follow::Paused { new_rows: 0 });
        assert!(!follow.moved(false), "still away: no change");
    }

    #[test]
    fn new_rows_are_counted_only_while_paused() {
        let mut follow = Follow::default();
        follow.arrived(5);
        assert_eq!(follow.new_rows(), 0);
        follow.moved(false);
        follow.arrived(3);
        follow.arrived(4);
        assert_eq!(follow.new_rows(), 7);
        follow.arrived(u64::MAX);
        assert_eq!(follow.new_rows(), u64::MAX, "saturates");
    }

    #[test]
    fn scrolling_back_to_the_newest_row_or_the_pill_follows_again() {
        let mut follow = Follow::Paused { new_rows: 9 };
        assert!(follow.moved(true));
        assert!(follow.is_following());
        assert_eq!(follow.new_rows(), 0, "the count is gone with the pause");

        let mut follow = Follow::Paused { new_rows: 9 };
        assert!(follow.resume());
        assert!(follow.is_following());
        assert!(!follow.resume(), "resuming a follower changes nothing");
    }

    #[test]
    fn the_pill_says_how_many_rows_are_new() {
        assert_eq!(Follow::Following.pill(), None);
        assert_eq!(
            Follow::Paused { new_rows: 0 }.pill().as_deref(),
            Some("Jump to the newest")
        );
        assert_eq!(
            Follow::Paused { new_rows: 1 }.pill().as_deref(),
            Some("1 new event")
        );
        assert_eq!(
            Follow::Paused { new_rows: 1234 }.pill().as_deref(),
            Some("1234 new events")
        );
    }
}
