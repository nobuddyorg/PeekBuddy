//! NTFS case rules on UTF-16 units.
//!
//! NTFS folds single 16-bit units through an upcase table; surrogate pairs
//! are not folded. `str::to_lowercase` and SQLite `NOCASE` disagree with
//! that, so never use them for matching. The table is range-encoded in
//! `upcase.rs` and must agree with `CompareStringOrdinal(bIgnoreCase = TRUE)`
//! (a Windows test in `peekbuddy-win` cross-checks every unit).

use super::{WideStr, upcase::UPCASE};

/// How names compare in a directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaseMode {
    /// The NTFS default: ordinal, case-insensitive.
    Insensitive,
    /// A directory flagged `FILE_CS_FLAG_CASE_SENSITIVE_DIR`.
    Sensitive,
}

/// The upper-case form of one UTF-16 unit.
#[must_use]
pub fn fold_upper(unit: u16) -> u16 {
    // Last run whose start is <= unit.
    let idx = UPCASE.partition_point(|&(start, ..)| start <= unit);
    let Some(&(start, end, stride, delta)) = idx.checked_sub(1).and_then(|i| UPCASE.get(i)) else {
        return unit;
    };
    let offset = unit - start;
    if unit > end || !offset.is_multiple_of(u16::from(stride)) {
        return unit;
    }
    u16::try_from(i32::from(unit) + delta).unwrap_or(unit)
}

/// Fold `src` into `dst` unit by unit; returns the units written, or `None`
/// when `dst` is shorter than `src`.
#[must_use]
pub fn fold_upper_into(src: &[u16], dst: &mut [u16]) -> Option<usize> {
    let out = dst.get_mut(..src.len())?;
    for (d, &s) in out.iter_mut().zip(src) {
        *d = fold_upper(s);
    }
    Some(src.len())
}

/// Whether two names are equal under `mode`.
#[must_use]
pub fn eq_ignore_case_ordinal(a: WideStr<'_>, b: WideStr<'_>, mode: CaseMode) -> bool {
    match mode {
        CaseMode::Sensitive => a == b,
        CaseMode::Insensitive => {
            a.len() == b.len()
                && a.units()
                    .iter()
                    .zip(b.units())
                    .all(|(&x, &y)| x == y || fold_upper(x) == fold_upper(y))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn folds_ascii_and_latin() {
        assert_eq!(fold_upper(u16::from(b'a')), u16::from(b'A'));
        assert_eq!(fold_upper(u16::from(b'z')), u16::from(b'Z'));
        assert_eq!(fold_upper(u16::from(b'A')), u16::from(b'A'));
        assert_eq!(fold_upper(u16::from(b'0')), u16::from(b'0'));
        assert_eq!(fold_upper(0x00E4), 0x00C4); // ä -> Ä
        assert_eq!(fold_upper(0x0101), 0x0100); // ā -> Ā (stride-2 run)
        assert_eq!(fold_upper(0x0100), 0x0100);
        assert_eq!(fold_upper(0x03B1), 0x0391); // α -> Α
        assert_eq!(fold_upper(0x0430), 0x0410); // а -> А
        assert_eq!(fold_upper(0x00DF), 0x00DF); // ß has no simple upper-case
        assert_eq!(fold_upper(0xD800), 0xD800);
        assert_eq!(fold_upper(0xFFFF), 0xFFFF);
        assert_eq!(fold_upper(0), 0);
    }

    #[test]
    fn table_is_sorted_and_disjoint() {
        let mut prev_end = None;
        for &(start, end, stride, _) in UPCASE {
            assert!(start <= end);
            assert!(stride == 1 || stride == 2);
            if let Some(p) = prev_end {
                assert!(start > p, "{start:#06X} overlaps");
            }
            prev_end = Some(end);
        }
    }

    #[test]
    fn folding_is_idempotent_for_every_unit() {
        for u in 0..=u16::MAX {
            let f = fold_upper(u);
            assert_eq!(fold_upper(f), f, "{u:#06X}");
        }
    }

    /// FNV-1a over the folded value of every unit. Any change to the table
    /// changes it, so a table update needs a reviewed update of this value
    /// (and the Windows cross-check must still pass).
    #[test]
    fn table_checksum_is_pinned() {
        let mut h: u64 = 0xCBF2_9CE4_8422_2325;
        for u in 0..=u16::MAX {
            for b in fold_upper(u).to_le_bytes() {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0000_0100_0000_01B3);
            }
        }
        assert_eq!(h, 0x6B6A_6531_84D1_823A, "table checksum changed");
    }

    #[test]
    fn surrogates_are_never_folded() {
        for u in 0xD800..=0xDFFF {
            assert_eq!(fold_upper(u), u);
        }
    }

    #[test]
    fn fold_into_needs_room() {
        let mut dst = [0_u16; 2];
        assert_eq!(fold_upper_into(&w("abc"), &mut dst), None);
        assert_eq!(fold_upper_into(&w("ab"), &mut dst), Some(2));
        assert_eq!(dst.as_slice(), w("AB").as_slice());
    }

    #[test]
    fn equality_modes() {
        let (a, b) = (w("Readme.TXT"), w("README.txt"));
        let (a, b) = (WideStr::new(&a), WideStr::new(&b));
        assert!(eq_ignore_case_ordinal(a, b, CaseMode::Insensitive));
        assert!(!eq_ignore_case_ordinal(a, b, CaseMode::Sensitive));
        assert!(eq_ignore_case_ordinal(a, a, CaseMode::Sensitive));
        let c = w("README.tx");
        assert!(!eq_ignore_case_ordinal(
            a,
            WideStr::new(&c),
            CaseMode::Insensitive
        ));
        let d = w("README.tzt");
        assert!(!eq_ignore_case_ordinal(
            a,
            WideStr::new(&d),
            CaseMode::Insensitive
        ));
    }

    proptest! {
        #[test]
        fn insensitive_equality_matches_folding(
            a in proptest::collection::vec(any::<u16>(), 0..16),
            b in proptest::collection::vec(any::<u16>(), 0..16),
        ) {
            let folded = |v: &[u16]| v.iter().map(|&u| fold_upper(u)).collect::<Vec<_>>();
            let expect = folded(&a) == folded(&b);
            prop_assert_eq!(eq_ignore_case_ordinal(WideStr::new(&a), WideStr::new(&b), CaseMode::Insensitive), expect);
            prop_assert!(eq_ignore_case_ordinal(WideStr::new(&a), WideStr::new(&a), CaseMode::Insensitive));
        }
    }
}
