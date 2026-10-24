//! Display escaping and the JSON helper.

use core::fmt::Write;

use super::{WideStr, to_lossy_utf8};

/// Characters that are invisible or reorder text; egui 0.37 hides them, so
/// the viewer shows them as escapes (F17).
const fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{2069}'
            | '\u{206A}'..='\u{206F}'
            | '\u{FEFF}'
    )
}

/// Whether [`escape_for_display`] would change `name`. Allocation-free.
#[must_use]
pub fn needs_escape(name: WideStr<'_>) -> bool {
    char::decode_utf16(name.units().iter().copied()).any(|r| r.map_or(true, is_invisible))
}

/// `name` for display: lone surrogates as `\u{D800}`, invisible format
/// characters as `\u{200B}`. May allocate; never on the capture path.
#[must_use]
pub fn escape_for_display(name: WideStr<'_>) -> String {
    let mut out = String::with_capacity(name.len());
    for r in char::decode_utf16(name.units().iter().copied()) {
        match r {
            Ok(c) if is_invisible(c) => push_escape(&mut out, u32::from(c)),
            Ok(c) => out.push(c),
            Err(e) => push_escape(&mut out, u32::from(e.unpaired_surrogate())),
        }
    }
    out
}

fn push_escape(out: &mut String, code: u32) {
    let _ = write!(out, "\\u{{{code:04X}}}");
}

/// JSON for a name: `name` (lossy text) and, only when the name is not valid
/// Unicode, `name_utf16` as an array of units. Never a raw `\uD800` escape,
/// which `serde_json` rejects.
#[must_use]
pub fn name_json(name: WideStr<'_>) -> serde_json::Value {
    let mut buf = vec![0_u8; super::max_utf8_len(name.len())];
    let mut obj = serde_json::Map::new();
    if let Some(l) = to_lossy_utf8(name, &mut buf) {
        obj.insert("name".into(), l.text.into());
        if !l.is_exact {
            obj.insert("name_utf16".into(), name.units().to_vec().into());
        }
    }
    serde_json::Value::Object(obj)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn escapes_every_listed_invisible_character() {
        let all = (0x200B..=0x200F)
            .chain(0x202A..=0x202E)
            .chain(0x2060..=0x2064)
            .chain(0x2066..=0x2069)
            .chain(0x206A..=0x206F)
            .chain([0xFEFF_u32]);
        for code in all {
            let units = [0x61, u16::try_from(code).unwrap(), 0x62];
            assert!(needs_escape(WideStr::new(&units)), "{code:#X}");
            assert_eq!(
                escape_for_display(WideStr::new(&units)),
                format!("a\\u{{{code:04X}}}b")
            );
        }
    }

    #[test]
    fn neighbours_of_the_ranges_are_left_alone() {
        for code in [
            0x200A_u16, 0x2010, 0x2029, 0x202F, 0x205F, 0x2065, 0x2070, 0xFEFE, 0xFF00,
        ] {
            assert!(!needs_escape(WideStr::new(&[code])), "{code:#X}");
        }
    }

    #[test]
    fn escapes_lone_surrogates_but_not_pairs() {
        assert_eq!(escape_for_display(WideStr::new(&[0xD800])), "\\u{D800}");
        assert_eq!(
            escape_for_display(WideStr::new(&[0xDFFF, 0x61])),
            "\\u{DFFF}a"
        );
        assert_eq!(
            escape_for_display(WideStr::new(&[0xD83D, 0xDE00])),
            "\u{1F600}"
        );
        assert_eq!(
            escape_for_display(WideStr::new(&[0xDE00, 0xD83D])),
            "\\u{DE00}\\u{D83D}"
        );
        assert!(needs_escape(WideStr::new(&[0xD800])));
        assert!(!needs_escape(WideStr::new(&w("plain.txt"))));
    }

    #[test]
    fn json_parses_for_lone_surrogates() {
        let units = [0x61, 0xD800, 0x62];
        let v = name_json(WideStr::new(&units));
        let text = serde_json::to_string(&v).unwrap();
        let back: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(back.get("name"), Some(&serde_json::json!("a\u{FFFD}b")));
        assert_eq!(
            back.get("name_utf16"),
            Some(&serde_json::json!([0x61, 0xD800, 0x62]))
        );
        assert!(!text.contains("\\ud800") && !text.contains("\\uD800"));
    }

    #[test]
    fn json_omits_the_array_for_valid_names() {
        let v = name_json(WideStr::new(&w("ok.txt")));
        assert_eq!(v.get("name"), Some(&serde_json::json!("ok.txt")));
        assert!(v.get("name_utf16").is_none());
    }

    proptest! {
        #[test]
        fn escape_is_identity_exactly_when_not_needed(units in proptest::collection::vec(any::<u16>(), 0..24)) {
            let name = WideStr::new(&units);
            let s = escape_for_display(name);
            if needs_escape(name) {
                prop_assert!(s.contains("\\u"));
            } else {
                prop_assert_eq!(s.encode_utf16().collect::<Vec<_>>(), units);
            }
        }

        #[test]
        fn json_always_round_trips_through_serde(units in proptest::collection::vec(any::<u16>(), 0..24)) {
            let text = serde_json::to_string(&name_json(WideStr::new(&units))).unwrap();
            let back: serde_json::Value = serde_json::from_str(&text).unwrap();
            let exact = char::decode_utf16(units.iter().copied()).all(|r| r.is_ok());
            prop_assert_eq!(back.get("name_utf16").is_some(), !exact);
        }
    }
}
