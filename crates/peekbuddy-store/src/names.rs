//! Names in the `node` table: lossy UTF-8 text plus an exact UTF-16LE
//! sidecar (F17).
//!
//! `name` is always valid UTF-8 (lone surrogates become U+FFFD) and is what
//! search and display use. `name_w` holds the exact UTF-16LE bytes, only when
//! the name is not valid Unicode; otherwise it is NULL. SQLite's `STRICT`
//! does not validate UTF-8, so WTF-8 is never stored in `TEXT`.

use peekbuddy_core::name::{WideName, WideStr, decode_name_w};

/// The `name` and `name_w` columns of one name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameColumns {
    /// Lossy UTF-8 text.
    pub text: String,
    /// Exact UTF-16LE bytes, when the name is not valid Unicode.
    pub wide: Option<Vec<u8>>,
}

/// The columns for `name`.
#[must_use]
pub fn to_columns(name: WideStr<'_>) -> NameColumns {
    let text: String = char::decode_utf16(name.units().iter().copied())
        .map(|unit| unit.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect();
    let wide = (!name.is_valid_unicode()).then(|| {
        name.units()
            .iter()
            .flat_map(|unit| unit.to_le_bytes())
            .collect()
    });
    NameColumns { text, wide }
}

/// The exact name from its columns; `None` when `wide` has an odd length.
#[must_use]
pub fn from_columns(text: &str, wide: Option<&[u8]>) -> Option<WideName> {
    match wide {
        Some(bytes) => decode_name_w(bytes),
        None => Some(WideName::new(text.encode_utf16().collect())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn units(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    #[test]
    fn valid_names_have_no_sidecar() {
        let u = units("h\u{e9}llo \u{1f600}.txt");
        let c = to_columns(WideStr::new(&u));
        assert_eq!(c.text, "h\u{e9}llo \u{1f600}.txt");
        assert_eq!(c.wide, None);
        assert_eq!(from_columns(&c.text, None).unwrap().into_units(), u);
    }

    #[test]
    fn lone_surrogates_keep_an_exact_sidecar() {
        let u = vec![0x61, 0xD800, 0x62];
        let c = to_columns(WideStr::new(&u));
        assert_eq!(c.text, "a\u{FFFD}b");
        assert_eq!(c.wide.as_deref(), Some(&[0x61, 0, 0x00, 0xD8, 0x62, 0][..]));
        let back = from_columns(&c.text, c.wide.as_deref()).unwrap();
        assert_eq!(back.into_units(), u);
    }

    #[test]
    fn an_odd_sidecar_is_rejected() {
        assert!(from_columns("x", Some(&[1, 2, 3])).is_none());
    }
}
