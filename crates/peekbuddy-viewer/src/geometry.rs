//! Where the viewer window opens (#65).
//!
//! The viewer remembers its window in `settings.toml` (`[viewer]`). A saved
//! window may no longer fit: the monitor it was on is unplugged, the
//! resolution changed, a docking station is gone. [`restore`] decides what to
//! do with it, in virtual-screen pixels:
//!
//! 1. no monitor is known: leave it to the OS;
//! 2. the saved window overlaps a monitor's work area by a usable amount:
//!    keep it there, shrunk and shifted so it lies inside that work area;
//! 3. otherwise: centre it on the primary work area (the one holding the
//!    origin, else the first).

use peekbuddy_core::settings::Viewer;

/// The smallest side a restored window gets, unless the work area is smaller.
pub const MIN_SIDE: i32 = 480;

/// How much of the window has to be on a monitor for that monitor to count
/// as its home, in pixels on each side: enough to grab the title bar.
const GRAB: i32 = 96;

/// A rectangle in virtual-screen pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PxRect {
    /// Left edge.
    pub left: i32,
    /// Top edge.
    pub top: i32,
    /// Right edge, exclusive.
    pub right: i32,
    /// Bottom edge, exclusive.
    pub bottom: i32,
}

impl PxRect {
    /// A rectangle from its corner and size.
    #[must_use]
    pub const fn at(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            left: x,
            top: y,
            right: x.saturating_add(width),
            bottom: y.saturating_add(height),
        }
    }

    /// Width.
    #[must_use]
    pub const fn width(&self) -> i32 {
        self.right.saturating_sub(self.left)
    }

    /// Height.
    #[must_use]
    pub const fn height(&self) -> i32 {
        self.bottom.saturating_sub(self.top)
    }

    fn overlap(&self, other: &Self) -> (i32, i32) {
        (
            self.right
                .min(other.right)
                .saturating_sub(self.left.max(other.left)),
            self.bottom
                .min(other.bottom)
                .saturating_sub(self.top.max(other.top)),
        )
    }

    fn contains_origin(&self) -> bool {
        self.left <= 0 && self.top <= 0 && self.right > 0 && self.bottom > 0
    }
}

/// What the window should do on start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    /// Where the window (not maximized) sits.
    pub rect: PxRect,
    /// Maximize it after placing.
    pub maximized: bool,
}

/// The saved window of `viewer`, when it has one.
#[must_use]
pub fn saved(viewer: &Viewer) -> Option<PxRect> {
    let (x, y, width, height) = (viewer.x?, viewer.y?, viewer.width?, viewer.height?);
    Some(PxRect::at(
        x,
        y,
        i32::try_from(width).ok()?,
        i32::try_from(height).ok()?,
    ))
}

/// Fits `rect` into `area`: no larger than the area, inside it.
fn fit(rect: PxRect, area: &PxRect) -> PxRect {
    let width = rect.width().max(MIN_SIDE).min(area.width());
    let height = rect.height().max(MIN_SIDE).min(area.height());
    let left = rect
        .left
        .clamp(area.left, area.right.saturating_sub(width).max(area.left));
    let top = rect
        .top
        .clamp(area.top, area.bottom.saturating_sub(height).max(area.top));
    PxRect::at(left, top, width, height)
}

/// Where the saved window goes on the monitors that exist now. `None` leaves
/// the placement to the OS (no saved window, or no monitor known).
#[must_use]
pub fn restore(viewer: &Viewer, work_areas: &[PxRect]) -> Option<Placement> {
    let saved = saved(viewer)?;
    let primary = work_areas
        .iter()
        .find(|area| area.contains_origin())
        .or_else(|| work_areas.first())?;
    let home = work_areas
        .iter()
        .filter(|area| {
            let (across, down) = saved.overlap(area);
            across >= GRAB && down >= GRAB
        })
        .max_by_key(|area| {
            let (across, down) = saved.overlap(area);
            i64::from(across) * i64::from(down)
        });
    let rect = if let Some(area) = home {
        fit(saved, area)
    } else {
        let size = fit(saved, primary);
        let left = primary.left + (primary.width() - size.width()) / 2;
        let top = primary.top + (primary.height() - size.height()) / 2;
        PxRect::at(left, top, size.width(), size.height())
    };
    Some(Placement {
        rect,
        maximized: viewer.maximized,
    })
}

/// `viewer` with `rect` remembered as its window.
#[must_use]
pub fn remember(mut viewer: Viewer, rect: PxRect, maximized: bool) -> Viewer {
    viewer.x = Some(rect.left);
    viewer.y = Some(rect.top);
    viewer.width = u32::try_from(rect.width()).ok();
    viewer.height = u32::try_from(rect.height()).ok();
    viewer.maximized = maximized;
    viewer
}

#[cfg(test)]
mod tests {
    use peekbuddy_core::settings::{Viewer, WINDOW_SIDE};
    use proptest::prelude::*;

    use super::{MIN_SIDE, Placement, PxRect, remember, restore, saved};

    const MAIN: PxRect = PxRect::at(0, 0, 1920, 1040);
    const RIGHT: PxRect = PxRect::at(1920, 0, 2560, 1400);

    fn viewer(x: i32, y: i32, width: u32, height: u32) -> Viewer {
        Viewer {
            x: Some(x),
            y: Some(y),
            width: Some(width),
            height: Some(height),
            ..Viewer::default()
        }
    }

    #[test]
    fn nothing_saved_or_no_monitor_leaves_it_to_the_os() {
        assert_eq!(restore(&Viewer::default(), &[MAIN]), None);
        assert_eq!(restore(&viewer(10, 10, 800, 600), &[]), None);
        let mut partial = viewer(10, 10, 800, 600);
        partial.width = None;
        assert_eq!(saved(&partial), None);
    }

    #[test]
    fn a_window_that_fits_stays_where_it_was() {
        let placement = restore(&viewer(100, 80, 1200, 800), &[MAIN]).unwrap();
        assert_eq!(placement.rect, PxRect::at(100, 80, 1200, 800));
        assert!(!placement.maximized);
    }

    #[test]
    fn the_maximized_flag_comes_along() {
        let mut v = viewer(100, 80, 1200, 800);
        v.maximized = true;
        assert!(restore(&v, &[MAIN]).unwrap().maximized);
    }

    #[test]
    fn an_unplugged_monitor_brings_the_window_to_the_primary_one() {
        // It was on the second monitor, which is gone.
        let placement = restore(&viewer(2200, 200, 1000, 700), &[MAIN]).unwrap();
        assert_eq!(placement.rect, PxRect::at(460, 170, 1000, 700));
    }

    #[test]
    fn the_monitor_with_most_of_the_window_is_its_home() {
        let spanning = viewer(1700, 100, 1000, 700);
        let placement = restore(&spanning, &[MAIN, RIGHT]).unwrap();
        // 780 px on the right monitor against 220 on the main one.
        assert!(placement.rect.left >= RIGHT.left, "{placement:?}");
        assert_eq!(placement.rect.width(), 1000);
    }

    #[test]
    fn the_home_monitor_is_the_one_with_the_most_overlapping_area() {
        // By area the second monitor wins (400 x 300 against 100 x 900); by
        // the sum of the sides the first would.
        let narrow = PxRect::at(0, 0, 100, 900);
        let wide = PxRect::at(100, 0, 400, 300);
        let placement = restore(&viewer(0, 0, 500, 900), &[narrow, wide]).unwrap();
        assert!(placement.rect.left >= wide.left, "{placement:?}");
        // By area the first monitor wins (500 x 500 against 600 x 300); by
        // the quotient of the sides the second would.
        let square = PxRect::at(0, 0, 500, 500);
        let flat = PxRect::at(500, 200, 600, 300);
        let placement = restore(&viewer(0, 0, 1100, 500), &[square, flat]).unwrap();
        assert!(placement.rect.right <= square.right, "{placement:?}");
    }

    #[test]
    fn a_monitor_that_ends_at_the_origin_is_not_the_primary_one() {
        // The left monitor ends exactly at x = 0; the origin is on the right
        // one, which is where a window that lost its monitor goes.
        let left = PxRect::at(-1920, 0, 1920, 1080);
        let right = PxRect::at(0, 0, 1920, 1080);
        let placement = restore(&viewer(5000, 5000, 800, 600), &[left, right]).unwrap();
        assert!(placement.rect.left >= right.left, "{placement:?}");
        assert!(placement.rect.right <= right.right, "{placement:?}");
    }

    #[test]
    fn a_window_larger_than_the_work_area_is_shrunk_to_it() {
        let placement = restore(&viewer(-50, -50, 4000, 3000), &[MAIN]).unwrap();
        assert_eq!(placement.rect, PxRect::at(0, 0, 1920, 1040));
    }

    #[test]
    fn a_window_hanging_off_the_edge_is_pulled_in() {
        let placement = restore(&viewer(1700, 900, 800, 600), &[MAIN]).unwrap();
        assert_eq!(placement.rect, PxRect::at(1120, 440, 800, 600));
    }

    #[test]
    fn a_tiny_saved_window_gets_the_minimum_side() {
        let placement = restore(&viewer(100, 100, 200, 200), &[MAIN]).unwrap();
        assert_eq!(placement.rect.width(), MIN_SIDE);
        assert_eq!(placement.rect.height(), MIN_SIDE);
    }

    #[test]
    fn a_work_area_below_the_minimum_wins_over_it() {
        let small = PxRect::at(0, 0, 400, 300);
        let placement = restore(&viewer(0, 0, 800, 600), &[small]).unwrap();
        assert_eq!(placement.rect, small);
    }

    #[test]
    fn the_primary_monitor_is_the_one_holding_the_origin() {
        let left = PxRect::at(-1920, 0, 1920, 1040);
        let placement = restore(&viewer(5000, 5000, 1000, 700), &[left, MAIN]).unwrap();
        assert_eq!(placement.rect, PxRect::at(460, 170, 1000, 700));
    }

    #[test]
    fn remembering_round_trips_through_saved() {
        let rect = PxRect::at(-30, 40, 1000, 700);
        let v = remember(Viewer::default(), rect, true);
        assert_eq!(saved(&v), Some(rect));
        assert!(v.maximized);
        assert!(v.width.is_some_and(|w| WINDOW_SIDE.contains(&w)));
    }

    #[test]
    fn a_window_of_no_size_is_not_remembered_as_a_size() {
        let v = remember(Viewer::default(), PxRect::at(0, 0, -5, 0), false);
        assert_eq!(v.width, None);
        assert_eq!(saved(&v), None);
    }

    fn arb_area() -> impl Strategy<Value = PxRect> {
        (-4000i32..4000, -2000i32..2000, 300i32..4000, 300i32..2500)
            .prop_map(|(x, y, w, h)| PxRect::at(x, y, w, h))
    }

    proptest! {
        #[test]
        fn a_restored_window_always_lies_inside_a_work_area(
            x in -8000i32..8000,
            y in -8000i32..8000,
            width in 200u32..8000,
            height in 200u32..8000,
            areas in proptest::collection::vec(arb_area(), 1..4),
        ) {
            let Placement { rect, .. } = restore(&viewer(x, y, width, height), &areas).unwrap();
            let inside = areas.iter().any(|area| {
                rect.left >= area.left
                    && rect.top >= area.top
                    && rect.right <= area.right
                    && rect.bottom <= area.bottom
            });
            prop_assert!(inside, "{rect:?} is outside {areas:?}");
            prop_assert!(rect.width() > 0 && rect.height() > 0);
        }
    }
}
