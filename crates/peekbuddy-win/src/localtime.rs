//! The local calendar day of a timestamp (#54): which day segment an event
//! belongs to.

use windows_sys::Win32::Foundation::{FILETIME, SYSTEMTIME};
use windows_sys::Win32::System::Time::{
    FileTimeToSystemTime, SystemTimeToFileTime, SystemTimeToTzSpecificLocalTime,
};

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

/// The offset of the user's local time zone from UTC at the instant
/// `filetime`, in seconds, with the daylight saving rules in force on that
/// date (the viewer shows local times and puts UTC in the tooltip). A
/// timestamp the system cannot convert has offset 0.
#[must_use]
pub fn utc_offset_seconds(filetime: u64) -> i32 {
    let ticks = FILETIME {
        dwLowDateTime: (filetime & 0xFFFF_FFFF) as u32,
        dwHighDateTime: (filetime >> 32) as u32,
    };
    let mut utc = SYSTEMTIME::default();
    // SAFETY: both pointers are valid for the call.
    if unsafe { FileTimeToSystemTime(&raw const ticks, &raw mut utc) } == 0 {
        return 0;
    }
    let mut local = SYSTEMTIME::default();
    // SAFETY: a null time zone means the current one; both pointers are valid.
    let ok = unsafe {
        SystemTimeToTzSpecificLocalTime(std::ptr::null(), &raw const utc, &raw mut local)
    };
    if ok == 0 {
        return 0;
    }
    // Both back to ticks, so the sub-millisecond part cancels out.
    let (mut utc_ticks, mut local_ticks) = (FILETIME::default(), FILETIME::default());
    // SAFETY: both pointers are valid for the call.
    let a = unsafe { SystemTimeToFileTime(&raw const utc, &raw mut utc_ticks) };
    // SAFETY: both pointers are valid for the call.
    let b = unsafe { SystemTimeToFileTime(&raw const local, &raw mut local_ticks) };
    if a == 0 || b == 0 {
        return 0;
    }
    offset_between(ticks_of(utc_ticks), ticks_of(local_ticks))
}

/// The ticks of a `FILETIME`.
fn ticks_of(time: FILETIME) -> u64 {
    (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
}

/// Seconds from `utc` to `local`, both in ticks of the same instant.
fn offset_between(utc: u64, local: u64) -> i32 {
    let seconds = (i128::from(local) - i128::from(utc)) / i128::from(TICKS_PER_SECOND);
    i32::try_from(seconds).unwrap_or(0)
}

const TICKS_PER_SECOND: u64 = 10_000_000;

#[cfg(test)]
mod tests {
    use super::{local_day, offset_between, ticks_of, utc_offset_seconds};
    use windows_sys::Win32::Foundation::FILETIME;

    #[test]
    fn the_offset_is_a_whole_number_of_quarter_hours_within_the_real_range() {
        let offset = utc_offset_seconds(NOON_UTC);
        assert!((-12 * 3600..=14 * 3600).contains(&offset), "{offset}");
        assert_eq!(offset % 900, 0, "{offset}");
    }

    #[test]
    fn the_offset_and_the_day_agree() {
        // Shifting noon UTC by the offset lands on the day `local_day` says.
        let shifted =
            NOON_UTC.saturating_add_signed(i64::from(utc_offset_seconds(NOON_UTC)) * 10_000_000);
        let (_, _, day) = local_day(NOON_UTC);
        let (_, _, shifted_day) = local_day(shifted);
        assert!(day.abs_diff(shifted_day) <= 1);
    }

    #[test]
    fn the_two_halves_of_a_filetime_are_one_number() {
        let time = FILETIME {
            dwLowDateTime: 0x1234_5678,
            dwHighDateTime: 0x9ABC_DEF0,
        };
        assert_eq!(ticks_of(time), 0x9ABC_DEF0_1234_5678);
    }

    #[test]
    fn the_offset_is_local_minus_utc_in_whole_seconds() {
        let hour = 3_600 * 10_000_000;
        assert_eq!(offset_between(1_000 * hour, 1_002 * hour), 7_200);
        assert_eq!(offset_between(1_000 * hour, 995 * hour), -18_000);
        assert_eq!(offset_between(5 * hour, 5 * hour), 0);
        // Sub-second ticks do not make a second.
        assert_eq!(offset_between(0, 10_000_000 - 1), 0);
        // An offset that cannot be an i32 is no offset.
        assert_eq!(offset_between(0, u64::MAX), 0);
    }

    #[test]
    fn a_time_the_system_cannot_convert_has_no_offset() {
        assert_eq!(utc_offset_seconds(u64::MAX), 0);
    }

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
