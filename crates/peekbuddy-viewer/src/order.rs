//! The sort order of the grid (F5): the time column is the id order, newest
//! first (the default) or oldest first.
//!
//! Pure data. [`crate::timeline::Timeline`] does the index arithmetic and
//! [`crate::compact::Folds`] the folding for both orders.

/// Which end of the log the grid starts at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Order {
    /// Newest first: row 0 is the newest, new rows appear at the top.
    #[default]
    Descending,
    /// Oldest first: the last row is the newest, new rows appear at the
    /// bottom.
    Ascending,
}

impl Order {
    /// The other order.
    #[must_use]
    pub const fn flipped(self) -> Self {
        match self {
            Self::Descending => Self::Ascending,
            Self::Ascending => Self::Descending,
        }
    }

    /// The mark after the title of the time column: none for the default
    /// order, so the usual header stays as it was.
    #[must_use]
    pub const fn mark(self) -> &'static str {
        match self {
            Self::Descending => "",
            Self::Ascending => "^",
        }
    }

    /// What clicking the time column does, for its tooltip.
    #[must_use]
    pub const fn tip(self) -> &'static str {
        match self {
            Self::Descending => "Newest first. Click to show the oldest first.",
            Self::Ascending => "Oldest first. Click to show the newest first.",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Order;

    #[test]
    fn it_starts_newest_first_and_flips_back_and_forth() {
        assert_eq!(Order::default(), Order::Descending);
        assert_eq!(Order::Descending.flipped(), Order::Ascending);
        assert_eq!(Order::Ascending.flipped().flipped(), Order::Ascending);
    }

    #[test]
    fn the_marks_and_tips_differ() {
        assert_ne!(Order::Descending.mark(), Order::Ascending.mark());
        assert_ne!(Order::Descending.tip(), Order::Ascending.tip());
    }
}
