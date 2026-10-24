//! The glob dialect (#40): gitignore-style patterns translated to regex
//! source for one case mode.
//!
//! Dialect, relative to the watch root:
//!
//! - `*` and `?` never cross a separator; `**` does. Both `/` and `\` are
//!   separators, in patterns and in paths.
//! - A leading `/` anchors to the root. So does any other separator inside
//!   the pattern; a pattern without one matches at any depth.
//! - A trailing `/` matches directories only.
//! - A leading `!` re-includes (the rule must be an exclude rule).
//! - `[abc]`, `[a-z]` and `[!abc]` are classes; separators never match inside
//!   a class. There is no backslash escaping: write `[*]` for a literal `*`.
//! - A pattern also matches everything below a path it matches, because an
//!   ancestor of a path is always a directory.
//!
//! The regex matches a prefix of the path from its start (`\A`, no end
//! anchor). The engine accepts a match that ends at the end of the path or
//! right before a separator, and for a directory-only glob requires a
//! directory in the first case (an ancestor is always one). Putting the "and
//! anything below" tail into the regex instead would make a combined DFA
//! exponential in the number of patterns.
//!
//! In an insensitive mode every literal and every class member is folded with
//! the ordinal table of #39, so that the folded candidate matches exactly
//! when the unfolded names are equal under that table.

use std::fmt::{self, Write as _};

use crate::name::{CaseMode, fold_upper};

/// The longest pattern accepted, in bytes.
pub const MAX_PATTERN_BYTES: usize = 4096;

/// Why a glob does not parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GlobError {
    /// Nothing is left of the pattern after the `!` and `/` markers.
    Empty,
    /// The pattern is longer than [`MAX_PATTERN_BYTES`].
    TooLong,
    /// The pattern holds a NUL character.
    Nul,
    /// A `[` without a closing `]`.
    UnclosedClass,
    /// A range whose end comes before its start.
    ReversedRange,
}

impl fmt::Display for GlobError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Empty => "the pattern is empty",
            Self::TooLong => "the pattern is too long",
            Self::Nul => "the pattern contains a NUL character",
            Self::UnclosedClass => "a `[` has no closing `]`",
            Self::ReversedRange => "a character range ends before it starts",
        })
    }
}

impl std::error::Error for GlobError {}

/// A parsed glob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedGlob {
    /// Regex source for a prefix of the path, anchored at its start with
    /// `\A`. Search it unanchored and check the match end as described in
    /// the module docs.
    pub regex: String,
    /// The pattern ended with a separator: it matches directories only.
    pub dir_only: bool,
    /// The pattern started with `!`.
    pub negated: bool,
}

const SEP: char = '/';
const BACKSLASH: char = '\\';
const SEP_CLASS: &str = r"[/\\]";
/// One char that is not a separator.
const NOT_SEP_CHAR: &str = r"[^/\\]";
/// Any byte, newline included. Matching bytewise keeps the DFA small; the
/// haystack is valid UTF-8 and every pattern piece that follows starts on a
/// char boundary, so a run of bytes never splits a char.
const ANY_BYTES: &str = r"(?s-u:.)";
/// Any byte except a separator (the `*` wildcard, bytewise like `ANY_BYTES`).
const NOT_SEP_BYTES: &str = r"(?-u:[^/\\])";

fn is_sep(c: char) -> bool {
    c == SEP || c == BACKSLASH
}

/// The fold of one char under `mode`: BMP chars go through the ordinal
/// table, everything else stays.
fn fold_char(c: char, mode: CaseMode) -> char {
    if mode == CaseMode::Sensitive {
        return c;
    }
    let mut units = [0_u16; 2];
    match c.encode_utf16(&mut units) {
        [unit] => char::from_u32(u32::from(fold_upper(*unit))).unwrap_or(c),
        _ => c,
    }
}

/// Appends `c` as a regex literal.
fn push_literal(out: &mut String, c: char) {
    if matches!(
        c,
        '\\' | '.'
            | '+'
            | '*'
            | '?'
            | '('
            | ')'
            | '|'
            | '['
            | ']'
            | '{'
            | '}'
            | '^'
            | '$'
            | '#'
            | '&'
            | '-'
            | '~'
    ) {
        out.push('\\');
    }
    out.push(c);
}

/// A character class as a sorted list of inclusive ranges.
#[derive(Debug, Default)]
struct ClassSet(Vec<(u32, u32)>);

impl ClassSet {
    fn add(&mut self, lo: u32, hi: u32) {
        self.0.push((lo, hi));
    }

    /// Adds `lo..=hi`, folded char by char when the mode folds.
    fn add_folded(&mut self, lo: char, hi: char, mode: CaseMode) {
        if mode == CaseMode::Sensitive {
            self.add(u32::from(lo), u32::from(hi));
            return;
        }
        // Chars outside the BMP stay as they are.
        let bmp_end = u32::from(hi).min(0xFFFF);
        for code in u32::from(lo)..=bmp_end {
            if let Some(c) = char::from_u32(code) {
                let f = u32::from(fold_char(c, mode));
                self.add(f, f);
            }
        }
        let astral_lo = u32::from(lo).max(0x1_0000);
        if astral_lo <= u32::from(hi) {
            self.add(astral_lo, u32::from(hi));
        }
    }

    fn merged(&self) -> Vec<(u32, u32)> {
        let mut ranges = self.0.clone();
        ranges.sort_unstable();
        let mut out: Vec<(u32, u32)> = Vec::with_capacity(ranges.len());
        for (lo, hi) in ranges {
            match out.last_mut() {
                Some(last) if lo <= last.1.saturating_add(1) => last.1 = last.1.max(hi),
                _ => out.push((lo, hi)),
            }
        }
        out
    }

    /// Regex source for the class; `negated` adds the separators so that
    /// they never match, a positive class has them removed.
    fn to_regex(&self, negated: bool) -> String {
        let mut set = ClassSet(self.0.clone());
        if negated {
            set.add(u32::from(SEP), u32::from(SEP));
            set.add(u32::from(BACKSLASH), u32::from(BACKSLASH));
        }
        let mut ranges = set.merged();
        if !negated {
            ranges = subtract_separators(&ranges);
            if ranges.is_empty() {
                // Nothing can match: the class of everything, negated.
                return r"[^\x{0}-\x{10FFFF}]".to_owned();
            }
        }
        let mut out = String::from(if negated { "[^" } else { "[" });
        for (lo, hi) in ranges {
            if lo == hi {
                let _ = write!(out, r"\x{{{lo:X}}}");
            } else {
                let _ = write!(out, r"\x{{{lo:X}}}-\x{{{hi:X}}}");
            }
        }
        out.push(']');
        out
    }
}

/// `ranges` without `/` and `\`.
fn subtract_separators(ranges: &[(u32, u32)]) -> Vec<(u32, u32)> {
    let mut out = Vec::with_capacity(ranges.len() + 2);
    for &(lo, hi) in ranges {
        let mut start = lo;
        for sep in [u32::from(SEP), u32::from(BACKSLASH)] {
            if (start..=hi).contains(&sep) {
                if sep > start {
                    out.push((start, sep - 1));
                }
                start = sep + 1;
            }
        }
        if start <= hi {
            out.push((start, hi));
        }
    }
    out
}

/// Parses a class whose `[` has been consumed; consumes through the closing
/// `]` and returns the regex.
fn parse_class(
    chars: &mut std::slice::Iter<'_, char>,
    mode: CaseMode,
) -> Result<String, GlobError> {
    let negated = matches!(chars.as_slice().first(), Some('!' | '^'));
    if negated {
        chars.next();
    }
    let mut set = ClassSet::default();
    let mut members = 0_usize;
    loop {
        let Some(&c) = chars.next() else {
            return Err(GlobError::UnclosedClass);
        };
        // A `]` right after the opening is a member.
        if c == ']' && members > 0 {
            break;
        }
        // `a-z`, unless the `-` is last.
        if let ['-', end, ..] = *chars.as_slice()
            && end != ']'
        {
            if end < c {
                return Err(GlobError::ReversedRange);
            }
            set.add_folded(c, end, mode);
            chars.next();
            chars.next();
        } else {
            set.add_folded(c, c, mode);
        }
        members += 1;
    }
    Ok(set.to_regex(negated))
}

/// Translates one segment (no separators, not `**`).
fn segment_regex(segment: &[char], mode: CaseMode) -> Result<String, GlobError> {
    let mut out = String::new();
    let mut chars = segment.iter();
    while let Some(&c) = chars.next() {
        match c {
            '*' => {
                out.push_str(NOT_SEP_BYTES);
                out.push('*');
            }
            '?' => out.push_str(NOT_SEP_CHAR),
            '[' => out.push_str(&parse_class(&mut chars, mode)?),
            other => push_literal(&mut out, fold_char(other, mode)),
        }
    }
    Ok(out)
}

/// Parses `pattern` for `mode`.
///
/// # Errors
///
/// A [`GlobError`] for an empty, over-long or malformed pattern.
pub fn parse(pattern: &str, mode: CaseMode) -> Result<ParsedGlob, GlobError> {
    if pattern.len() > MAX_PATTERN_BYTES {
        return Err(GlobError::TooLong);
    }
    if pattern.contains('\0') {
        return Err(GlobError::Nul);
    }
    let mut chars: Vec<char> = pattern.chars().collect();
    let negated = chars.first() == Some(&'!');
    if negated {
        chars.remove(0);
    }
    let dir_only = chars.last().copied().is_some_and(is_sep);
    if dir_only {
        chars.pop();
    }
    let leading_sep = chars.first().copied().is_some_and(is_sep);
    if leading_sep {
        chars.remove(0);
    }
    if chars.is_empty() {
        return Err(GlobError::Empty);
    }
    let anchored = leading_sep || chars.iter().copied().any(is_sep);
    let mut segments: Vec<&[char]> = chars.split(|&c| is_sep(c)).collect();
    // `**/**` means the same as `**`.
    segments.dedup_by(|a, b| *a == ['*', '*'] && *b == ['*', '*']);
    let mut body = String::new();
    if !anchored {
        // Any depth: an optional run of directories first.
        let _ = write!(body, "(?:{ANY_BYTES}*{SEP_CLASS})?");
    }
    let last = segments.len() - 1;
    let mut after_globstar = true;
    for (k, segment) in segments.iter().enumerate() {
        let globstar = *segment == ['*', '*'];
        if globstar {
            if segments.len() == 1 {
                let _ = write!(body, "{ANY_BYTES}*");
            } else if k == 0 {
                let _ = write!(body, "(?:{ANY_BYTES}*{SEP_CLASS})?");
            } else if k == last {
                let _ = write!(body, "{SEP_CLASS}{ANY_BYTES}+");
            } else {
                let _ = write!(body, "{SEP_CLASS}(?:{ANY_BYTES}*{SEP_CLASS})?");
            }
            after_globstar = true;
            continue;
        }
        if !after_globstar {
            body.push_str(SEP_CLASS);
        }
        body.push_str(&segment_regex(segment, mode)?);
        after_globstar = false;
    }
    let regex = format!(r"\A{body}");
    Ok(ParsedGlob {
        regex,
        dir_only,
        negated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors() {
        let p = |s: &str| parse(s, CaseMode::Sensitive).unwrap_err();
        assert_eq!(p(""), GlobError::Empty);
        assert_eq!(p("!"), GlobError::Empty);
        assert_eq!(p("/"), GlobError::Empty);
        assert_eq!(p("!/"), GlobError::Empty);
        assert_eq!(p("a\0b"), GlobError::Nul);
        assert_eq!(p("[abc"), GlobError::UnclosedClass);
        assert_eq!(p("[]"), GlobError::UnclosedClass);
        assert_eq!(p("[!"), GlobError::UnclosedClass);
        assert_eq!(p("[z-a]"), GlobError::ReversedRange);
        assert_eq!(p(&"a".repeat(MAX_PATTERN_BYTES + 1)), GlobError::TooLong);
        assert!(parse(&"a".repeat(MAX_PATTERN_BYTES), CaseMode::Sensitive).is_ok());
    }

    #[test]
    fn class_regexes_merge_and_fold_their_ranges() {
        let class = |pattern: &str, mode| parse(pattern, mode).unwrap().regex;
        assert!(class("[abc]", CaseMode::Sensitive).contains(r"[\x{61}-\x{63}]"));
        assert!(class("[a-cb-d]", CaseMode::Sensitive).contains(r"[\x{61}-\x{64}]"));
        assert!(class("[ac]", CaseMode::Sensitive).contains(r"[\x{61}\x{63}]"));
        assert!(class("[a-a]", CaseMode::Sensitive).contains(r"[\x{61}]"));
        assert!(class("[!a]", CaseMode::Sensitive).contains(r"[^\x{2F}\x{5C}\x{61}]"));
        // Folded one member at a time.
        assert!(class("[a-c]", CaseMode::Insensitive).contains(r"[\x{41}-\x{43}]"));
        // Outside the BMP nothing folds; the range stays whole.
        assert!(
            class("[\u{10000}-\u{10FFFF}]", CaseMode::Insensitive)
                .contains(r"[\x{10000}-\x{10FFFF}]")
        );
        // A range that ends at U+FFFF adds nothing outside the BMP.
        let top = class("[\u{FFFE}-\u{FFFF}]", CaseMode::Insensitive);
        assert!(top.contains(r"[\x{FFFE}-\x{FFFF}]"), "{top}");
        // A range that starts below and ends above U+FFFF has both parts.
        let both = class("[\u{FFFF}-\u{10001}]", CaseMode::Insensitive);
        assert!(both.contains(r"\x{FFFF}-\x{10001}"), "{both}");
        // Separators never match inside a class.
        assert!(class("[.-0]", CaseMode::Sensitive).contains(r"[\x{2E}\x{30}]"));
    }

    #[test]
    fn negation_and_dir_only_flags() {
        let g = parse("!build/", CaseMode::Sensitive).unwrap();
        assert!(g.negated);
        assert!(g.dir_only);
        let g = parse("build", CaseMode::Sensitive).unwrap();
        assert!(!g.negated);
        assert!(!g.dir_only);
        assert!(parse("build\\", CaseMode::Sensitive).unwrap().dir_only);
    }

    #[test]
    fn literals_are_escaped_and_folded() {
        let g = parse("a.b+c", CaseMode::Sensitive).unwrap();
        assert!(g.regex.contains(r"a\.b\+c"), "{}", g.regex);
        let g = parse("a.b", CaseMode::Insensitive).unwrap();
        assert!(g.regex.contains(r"A\.B"), "{}", g.regex);
    }

    #[test]
    fn classes_drop_separators() {
        assert_eq!(
            subtract_separators(&[(0x2E, 0x30)]),
            vec![(0x2E, 0x2E), (0x30, 0x30)]
        );
        assert_eq!(subtract_separators(&[(0x5C, 0x5C)]), vec![]);
        assert_eq!(
            subtract_separators(&[(0x20, 0x7E)]),
            vec![(0x20, 0x2E), (0x30, 0x5B), (0x5D, 0x7E)]
        );
    }
}
