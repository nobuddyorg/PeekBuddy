//! Name fidelity (#39, F17): lossless UTF-16 names, lossy UTF-8 for text,
//! the exact `name_w` sidecar, 8.3 detection, NTFS case rules and display
//! escaping.
//!
//! Names are `[u16]` units, valid Unicode or not. Everything on the capture
//! path (lossy conversion, folding, comparison, the escape check) writes into
//! caller buffers and never allocates.

mod case;
mod display;
mod upcase;

pub use case::{CaseMode, eq_ignore_case_ordinal, fold_upper, fold_upper_into};
pub use display::{escape_for_display, name_json, needs_escape};

/// The most UTF-8 bytes a name of `units` UTF-16 units can need: three per
/// unit (a surrogate pair is four bytes for two units, a lone surrogate
/// becomes U+FFFD, three bytes).
#[must_use]
pub const fn max_utf8_len(units: usize) -> usize {
    units.saturating_mul(3)
}

/// A borrowed UTF-16 name: the lossless in-core representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WideStr<'a>(&'a [u16]);

impl<'a> WideStr<'a> {
    /// Wrap UTF-16 units.
    #[must_use]
    pub const fn new(units: &'a [u16]) -> Self {
        Self(units)
    }

    /// The units.
    #[must_use]
    pub const fn units(self) -> &'a [u16] {
        self.0
    }

    /// Length in units.
    #[must_use]
    pub const fn len(self) -> usize {
        self.0.len()
    }

    /// Whether the name is empty.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0.is_empty()
    }

    /// Whether the units are valid UTF-16 (no unpaired surrogate).
    #[must_use]
    pub fn is_valid_unicode(self) -> bool {
        char::decode_utf16(self.0.iter().copied()).all(|r| r.is_ok())
    }

    /// The last path component (after the last `\` or `/`).
    #[must_use]
    pub fn file_name(self) -> Self {
        let start = self
            .0
            .iter()
            .rposition(|&u| u == u16::from(b'\\') || u == u16::from(b'/'))
            .map_or(0, |i| i + 1);
        Self(self.0.get(start..).unwrap_or_default())
    }

    /// An owned copy.
    #[must_use]
    pub fn to_owned_name(self) -> WideName {
        WideName(self.0.to_vec())
    }
}

/// An owned UTF-16 name (read side: store rows, IPC, tests).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct WideName(Vec<u16>);

impl WideName {
    /// Take ownership of UTF-16 units.
    #[must_use]
    pub const fn new(units: Vec<u16>) -> Self {
        Self(units)
    }

    /// Borrow as a [`WideStr`].
    #[must_use]
    pub fn as_wide(&self) -> WideStr<'_> {
        WideStr(&self.0)
    }

    /// The units back.
    #[must_use]
    pub fn into_units(self) -> Vec<u16> {
        self.0
    }
}

/// Lossy UTF-8 text of a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lossy<'b> {
    /// The text; lone surrogates are U+FFFD.
    pub text: &'b str,
    /// Whether the text is exact. `false` means the `name_w` sidecar is
    /// needed to name the file exactly.
    pub is_exact: bool,
}

/// Write the lossy UTF-8 form of `name` into `buf`.
///
/// `None` when `buf` is shorter than the output;
/// [`max_utf8_len`] is always enough.
#[must_use]
pub fn to_lossy_utf8<'b>(name: WideStr<'_>, buf: &'b mut [u8]) -> Option<Lossy<'b>> {
    let mut len = 0;
    let mut is_exact = true;
    for r in char::decode_utf16(name.0.iter().copied()) {
        let c = r.unwrap_or_else(|_| {
            is_exact = false;
            char::REPLACEMENT_CHARACTER
        });
        let mut tmp = [0_u8; 4];
        let bytes = c.encode_utf8(&mut tmp).as_bytes();
        let end = len + bytes.len();
        buf.get_mut(len..end)?.copy_from_slice(bytes);
        len = end;
    }
    let text = core::str::from_utf8(buf.get(..len)?).ok()?;
    Some(Lossy { text, is_exact })
}

/// Result of [`encode_name_w`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameW {
    /// The name is valid Unicode: no sidecar (stored as NULL).
    NotNeeded,
    /// The sidecar bytes were written; the count is in the payload.
    Written(usize),
    /// The output buffer is too small (`2 * name.len()` bytes are enough).
    BufferTooSmall,
}

/// Encode the exact `name_w` sidecar (UTF-16LE bytes), only when `name` is
/// not valid Unicode.
#[must_use]
pub fn encode_name_w(name: WideStr<'_>, out: &mut [u8]) -> NameW {
    if name.is_valid_unicode() {
        return NameW::NotNeeded;
    }
    let need = name.0.len() * 2;
    let Some(dst) = out.get_mut(..need) else {
        return NameW::BufferTooSmall;
    };
    let (pairs, _) = dst.as_chunks_mut::<2>();
    for (pair, unit) in pairs.iter_mut().zip(name.0) {
        *pair = unit.to_le_bytes();
    }
    NameW::Written(need)
}

/// Decode `name_w` bytes back to a name; `None` for an odd length.
#[must_use]
pub fn decode_name_w(bytes: &[u8]) -> Option<WideName> {
    let (pairs, rest) = bytes.as_chunks::<2>();
    if !rest.is_empty() {
        return None;
    }
    Some(WideName(
        pairs.iter().map(|p| u16::from_le_bytes(*p)).collect(),
    ))
}

/// Whether the last component of `name` has the shape of an 8.3 alias:
/// `BASE~N` (1 to 6 base characters, `N` digits, 8 characters at most) with
/// an optional extension of up to 3 characters.
///
/// A shape check only: a long name can look like this by accident.
#[must_use]
pub fn looks_like_short_name(name: WideStr<'_>) -> bool {
    let leaf = name.file_name().0;
    let (base, ext) = match leaf.iter().position(|&u| u == u16::from(b'.')) {
        Some(dot) => (leaf.get(..dot), leaf.get(dot + 1..)),
        None => (Some(leaf), Some(&[][..])),
    };
    let (Some(base), Some(ext)) = (base, ext) else {
        return false;
    };
    if base.len() > 8 || ext.len() > 3 || !ext.iter().all(|&u| is_short_char(u)) {
        return false;
    }
    let Some(tilde) = base.iter().rposition(|&u| u == u16::from(b'~')) else {
        return false;
    };
    let (stem, digits) = (base.get(..tilde), base.get(tilde + 1..));
    let (Some(stem), Some(digits)) = (stem, digits) else {
        return false;
    };
    !stem.is_empty()
        && stem.iter().all(|&u| is_short_char(u))
        && !digits.is_empty()
        && digits
            .iter()
            .all(|&u| (u16::from(b'0')..=u16::from(b'9')).contains(&u))
}

/// Characters allowed in a generated short name: upper-case ASCII letters,
/// digits and the punctuation `! # $ % & ' ( ) - @ ^ _ { } ~` plus the grave
/// accent.
fn is_short_char(u: u16) -> bool {
    u8::try_from(u).is_ok_and(|b| {
        b.is_ascii_uppercase()
            || b.is_ascii_digit()
            || matches!(
                b,
                b'!' | b'#'
                    | b'$'
                    | b'%'
                    | b'&'
                    | b'\''
                    | b'('
                    | b')'
                    | b'-'
                    | b'@'
                    | b'^'
                    | b'_'
                    | b'`'
                    | b'{'
                    | b'}'
                    | b'~'
            )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    /// Arbitrary units without NUL and the reserved characters.
    fn arb_units() -> impl Strategy<Value = Vec<u16>> {
        proptest::collection::vec(
            any::<u16>().prop_filter("reserved", |u| {
                *u >= 0x20
                    && !matches!(
                        u8::try_from(*u),
                        Ok(b'<' | b'>' | b':' | b'"' | b'/' | b'\\' | b'|' | b'?' | b'*')
                    )
            }),
            0..40,
        )
    }

    #[test]
    fn wide_str_basics() {
        let n = w("ab");
        let s = WideStr::new(&n);
        assert_eq!((s.len(), s.is_empty()), (2, false));
        assert!(WideStr::new(&[]).is_empty());
        assert!(s.is_valid_unicode());
        assert!(!WideStr::new(&[0xDC00]).is_valid_unicode());
    }

    #[test]
    fn lossy_replaces_lone_surrogates() {
        let units = [0x61, 0xD800, 0x62, 0xDC00, 0xD83D, 0xDE00];
        let mut buf = [0_u8; 32];
        let l = to_lossy_utf8(WideStr::new(&units), &mut buf).unwrap();
        assert_eq!(l.text, "a\u{FFFD}b\u{FFFD}\u{1F600}");
        assert!(!l.is_exact);
    }

    #[test]
    fn lossy_exact_for_valid_names() {
        let units = w("grüße 日本語 😀");
        let mut buf = vec![0_u8; max_utf8_len(units.len())];
        let l = to_lossy_utf8(WideStr::new(&units), &mut buf).unwrap();
        assert_eq!(l.text, "grüße 日本語 😀");
        assert!(l.is_exact);
    }

    #[test]
    fn lossy_buffer_too_small_is_none() {
        let units = w("abc");
        let mut buf = [0_u8; 2];
        assert_eq!(to_lossy_utf8(WideStr::new(&units), &mut buf), None);
        let mut buf = [0_u8; 3];
        assert!(to_lossy_utf8(WideStr::new(&units), &mut buf).is_some());
    }

    #[test]
    fn lossy_worst_case_fits_max_utf8_len() {
        let units = vec![0xD800_u16; 5];
        let mut buf = vec![0_u8; max_utf8_len(units.len())];
        assert!(to_lossy_utf8(WideStr::new(&units), &mut buf).is_some());
        let units = vec![0xFFFF_u16; 5];
        assert!(to_lossy_utf8(WideStr::new(&units), &mut buf).is_some());
    }

    #[test]
    fn name_w_only_when_not_valid_unicode() {
        let mut out = [0_u8; 16];
        assert_eq!(
            encode_name_w(WideStr::new(&w("abc")), &mut out),
            NameW::NotNeeded
        );
        let bad = [0x61, 0xD800];
        assert_eq!(
            encode_name_w(WideStr::new(&bad), &mut out),
            NameW::Written(4)
        );
        assert_eq!(out.get(..4), Some(&[0x61, 0x00, 0x00, 0xD8][..]));
        let mut small = [0_u8; 3];
        assert_eq!(
            encode_name_w(WideStr::new(&bad), &mut small),
            NameW::BufferTooSmall
        );
        assert_eq!(decode_name_w(&[1, 2, 3]), None);
        assert_eq!(decode_name_w(&[]), Some(WideName::default()));
    }

    #[test]
    fn file_name_takes_the_last_component() {
        let n = w("dir\\sub/NAME~1.TXT");
        assert_eq!(
            WideStr::new(&n).file_name().units(),
            w("NAME~1.TXT").as_slice()
        );
        let n = w("plain");
        assert_eq!(WideStr::new(&n).file_name().units(), n.as_slice());
        let n = w("dir\\");
        assert!(WideStr::new(&n).file_name().is_empty());
    }

    #[test]
    fn short_name_shapes() {
        for yes in [
            "PROGRA~1",
            "NAME~1.TXT",
            "A~1",
            "LONGN~12.EX",
            "dir\\DOCUME~1",
            "ABC~1234.A",
            "_X~9.C#",
        ] {
            assert!(looks_like_short_name(WideStr::new(&w(yes))), "{yes}");
        }
        for no in [
            "",
            "~1",
            "NAME.TXT",
            "NAME~.TXT",
            "NAME~1.TEXT",
            "TOOLONGNAME~1",
            "name~1.txt",
            "NA ME~1",
            "NAME~1X",
            "a.b~1",
            "dir\\",
            "NAME~A",
            "NAME~1.T X",
        ] {
            assert!(!looks_like_short_name(WideStr::new(&w(no))), "{no}");
        }
    }

    proptest! {
        #[test]
        fn wide_name_round_trips(units in arb_units()) {
            let name = WideStr::new(&units).to_owned_name();
            prop_assert_eq!(name.as_wide().units(), units.as_slice());
            prop_assert_eq!(name.into_units(), units);
        }

        #[test]
        fn name_w_round_trips(units in arb_units()) {
            let mut out = vec![0_u8; units.len() * 2];
            match encode_name_w(WideStr::new(&units), &mut out) {
                NameW::NotNeeded => prop_assert!(WideStr::new(&units).is_valid_unicode()),
                NameW::Written(n) => {
                    prop_assert!(!WideStr::new(&units).is_valid_unicode());
                    let back = decode_name_w(out.get(..n).unwrap()).unwrap();
                    prop_assert_eq!(back.into_units(), units);
                }
                NameW::BufferTooSmall => prop_assert!(false, "buffer is big enough"),
            }
        }

        #[test]
        fn lossy_is_utf8_and_exact_iff_valid(units in arb_units()) {
            let mut buf = vec![0_u8; max_utf8_len(units.len())];
            let l = to_lossy_utf8(WideStr::new(&units), &mut buf).unwrap();
            let decodes = char::decode_utf16(units.iter().copied()).all(|r| r.is_ok());
            prop_assert_eq!(l.is_exact, decodes);
            if l.is_exact {
                prop_assert_eq!(l.text.encode_utf16().collect::<Vec<_>>(), units);
            }
        }
    }
}
