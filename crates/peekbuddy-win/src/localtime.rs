//! The local calendar day of a timestamp (#54): which day segment an event
//! belongs to.

use windows_sys::Win32::Foundation::{FILETIME, SYSTEMTIME};
use windows_sys::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};

/// The year, month (1 to 12) and day (1 to 31) of `filetime` (100 ns ticks
/// since 1601, UTC) in the user's local time zone, with the daylight saving
/// rules in force on that date. A timestamp the system cannot convert gives
/// the UTC day.
#[must_use]
pub fn local_day(filetime: u64) -> (i32, u8, u8) {
    let ticks = FILETIME {
        dwLowDateTime: (filetime & 0xFFFF_FFFF) as u32,
        dwHighDateTime: (filetime >> 32) as u32,
    };
    let mut utc = SYSTEMTIME::default();
    // SAFETY: both pointers are valid for the call.
    if unsafe { FileTimeToSystemTime(&raw const ticks, &raw mut utc) } == 0 {
        return (1601, 1, 1);
    }
    let mut local = SYSTEMTIME::default();
    // SAFETY: a null time zone means the current one; both pointers are valid.
    let ok = unsafe {
        SystemTimeToTzSpecificLocalTime(std::ptr::null(), &raw const utc, &raw mut local)
    };
    let day = if ok == 0 { utc } else { local };
    (
        i32::from(day.wYear),
        u8::try_from(day.wMonth).unwrap_or(1),
        u8::try_from(day.wDay).unwrap_or(1),
    )
}

#[cfg(test)]
mod tests {
    use super::local_day;

    /// 2026-10-26 12:00:00 UTC in FILETIME ticks.
    const NOON_UTC: u64 = 134_374_896_000_000_000;

    #[test]
    fn noon_utc_is_the_same_or_the_neighbouring_day_everywhere() {
        let (year, month, day) = local_day(NOON_UTC);
        assert_eq!((year, month), (2026, 10));
        // Time zones go from UTC-12 to UTC+14.
        assert!((25..=27).contains(&day), "{day}");
    }

    #[test]
    fn a_time_the_system_cannot_convert_is_the_first_day_of_the_epoch() {
        assert_eq!(local_day(u64::MAX), (1601, 1, 1));
    }

    #[test]
    fn the_day_changes_on_a_whole_minute_so_both_halves_of_the_ticks_count() {
        // Local midnight is a whole number of minutes from UTC midnight, so the
        // first tick of a local day is a multiple of 600 million ticks; a
        // function that drops or garbles the low 32 bits finds another tick.
        let start = NOON_UTC;
        let first = local_day(start);
        let (mut low, mut high) = (start, start + 2 * 24 * 36_000_000_000);
        assert_ne!(local_day(high), first);
        while high - low > 1 {
            let middle = low + (high - low) / 2;
            if local_day(middle) == first {
                low = middle;
            } else {
                high = middle;
            }
        }
        assert_eq!(high % 600_000_000, 0, "{high}");
    }

    #[test]
    fn a_day_later_is_the_next_day() {
        let a = local_day(NOON_UTC);
        let b = local_day(NOON_UTC + 24 * 36_000_000_000);
        assert!(b > a, "{a:?} {b:?}");
    }
}
