//! Cross-checks of the core name rules against the OS (Windows only, #39).
#![cfg(windows)]

use std::os::windows::ffi::{OsStrExt, OsStringExt};

use peekbuddy_core::name::{CaseMode, WideStr, eq_ignore_case_ordinal, fold_upper};
use proptest::prelude::*;
use windows_sys::Win32::Globalization::{CSTR_EQUAL, CompareStringOrdinal, LCMapStringEx};

const LCMAP_UPPERCASE: u32 = 0x0000_0200;

/// `CompareStringOrdinal(.., bIgnoreCase = TRUE)` equality.
fn os_equal(a: &[u16], b: &[u16]) -> bool {
    let la = i32::try_from(a.len()).unwrap_or(i32::MAX);
    let lb = i32::try_from(b.len()).unwrap_or(i32::MAX);
    // SAFETY: both pointers and lengths describe live slices for the call.
    let r = unsafe { CompareStringOrdinal(a.as_ptr(), la, b.as_ptr(), lb, 1) };
    assert_ne!(r, 0, "CompareStringOrdinal failed");
    r == CSTR_EQUAL
}

/// The OS simple upper-case of one unit (invariant locale, no linguistic casing).
fn os_upper(unit: u16) -> u16 {
    let src = [unit];
    let mut dst = [0_u16; 2];
    let invariant = [0_u16];
    // SAFETY: the locale name is NUL-terminated; src and dst are live arrays
    // whose lengths are passed; the optional pointers may be null.
    let n = unsafe {
        LCMapStringEx(
            invariant.as_ptr(),
            LCMAP_UPPERCASE,
            src.as_ptr(),
            1,
            dst.as_mut_ptr(),
            2,
            core::ptr::null(),
            core::ptr::null(),
            0,
        )
    };
    assert_eq!(n, 1, "LCMapStringEx failed for {unit:#06X}");
    dst[0]
}

#[test]
fn os_string_round_trip_is_lossless_for_lone_surrogates() {
    let units = [0x61_u16, 0xD800, 0x62, 0xDC00, 0xD83D, 0xDE00];
    let os = std::ffi::OsString::from_wide(&units);
    assert_eq!(os.encode_wide().collect::<Vec<_>>(), units);
}

#[test]
fn fold_matches_the_os_upper_case_for_every_unit() {
    let mismatches: Vec<String> = (0..=u16::MAX)
        .filter(|&u| fold_upper(u) != os_upper(u))
        .map(|u| format!("{u:04X}:{:04X}/{:04X}", fold_upper(u), os_upper(u)))
        .collect();
    assert!(
        mismatches.is_empty(),
        "{} units differ (unit:ours/os): {}",
        mismatches.len(),
        mismatches.join(" ")
    );
}

#[test]
fn ordinal_ignore_case_agrees_for_every_single_unit() {
    let mut bad = Vec::new();
    for u in 0..=u16::MAX {
        let partners = [
            u,
            u ^ 1,
            u.wrapping_add(1),
            u.wrapping_sub(1),
            fold_upper(u),
            os_upper(u),
            0x41 + (u & 0x1F),
        ];
        for v in partners {
            let ours = eq_ignore_case_ordinal(
                WideStr::new(&[u]),
                WideStr::new(&[v]),
                CaseMode::Insensitive,
            );
            let os = os_equal(&[u], &[v]);
            if ours != os {
                bad.push(format!("{u:04X}/{v:04X}:ours={ours}/os={os}"));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "{} pairs differ: {}",
        bad.len(),
        bad.join(" ")
    );
}

proptest! {
    #[test]
    fn ordinal_ignore_case_agrees_for_random_pairs(
        a in proptest::collection::vec(any::<u16>(), 0..16),
        b in proptest::collection::vec(any::<u16>(), 0..16),
    ) {
        let ours = eq_ignore_case_ordinal(WideStr::new(&a), WideStr::new(&b), CaseMode::Insensitive);
        prop_assert_eq!(ours, os_equal(&a, &b));
    }

    #[test]
    fn ordinal_ignore_case_agrees_for_case_variants(a in proptest::collection::vec(any::<u16>(), 0..16)) {
        let up: Vec<u16> = a.iter().map(|&u| fold_upper(u)).collect();
        prop_assert!(eq_ignore_case_ordinal(WideStr::new(&a), WideStr::new(&up), CaseMode::Insensitive));
        prop_assert!(os_equal(&a, &up));
    }
}
