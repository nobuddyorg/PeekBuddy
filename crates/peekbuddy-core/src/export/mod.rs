//! Exporting rows as CSV or JSON Lines (#56).
//!
//! One line per stored record (F3), streamed: [`RowWriter`] formats a row
//! into a buffer it reuses, so memory does not grow with the number of rows.
//! [`read`] parses both formats back into rows, for the round-trip tests, the
//! churn comparator and fuzzing.
//!
//! # Columns
//!
//! | Column | Meaning |
//! | --- | --- |
//! | `id` | event id |
//! | `ts_utc` | `YYYY-MM-DDTHH:MM:SS.fffffffZ`, 100 ns precision; authoritative |
//! | `ts_local` | the same instant with the offset valid then, from the exporting machine |
//! | `kind` | [`Kind::name`](crate::event::Kind::name) |
//! | `source` | `rdcw`, `usn`, `reconcile` or `system` |
//! | `path`, `old_path` | lossy UTF-8; components joined by `\` |
//! | `is_dir` | `true` or `false` |
//! | `size`, `attrs` | empty when unknown |
//! | `raw_mask` | the OS value, `0x` hex |
//! | `file_id` | 128-bit file id, 32 hex digits |
//! | `root` | the watch root, lossy |
//! | `marker` | `gap`, `reconcile` or empty |
//! | `flags` | row flag bits |
//! | `path_utf16`, `old_path_utf16`, `root_utf16` | the exact name, only when it is not valid Unicode (F17) |
//!
//! The exact form writes printable ASCII as itself, a backslash as `\\` and
//! every other UTF-16 code unit as `\u{XXXX}`. A lone surrogate therefore
//! never appears as a JSON `\uD800` escape. Bidirectional and other format
//! characters are exported verbatim (they are data); only the viewer escapes
//! them for display.

pub mod read;

use std::io::{self, Write};

use crate::event::Source;
use crate::query::Row;

/// The columns, in order.
pub const COLUMNS: [&str; 17] = [
    "id",
    "ts_utc",
    "ts_local",
    "kind",
    "source",
    "path",
    "old_path",
    "is_dir",
    "size",
    "attrs",
    "raw_mask",
    "file_id",
    "root",
    "marker",
    "flags",
    "path_utf16",
    "old_path_utf16",
];
/// The side column for the root, written after [`COLUMNS`].
pub const ROOT_UTF16: &str = "root_utf16";

/// FILETIME ticks per second.
const TICKS_PER_SECOND: u64 = 10_000_000;
/// The last instant with a four-digit year, 9999-12-31T23:59:59.9999999Z;
/// later timestamps are written as this one.
pub const MAX_TS: u64 = 2_650_467_743_999_999_999;
/// Days from 1601-01-01 to 1970-01-01.
const DAYS_TO_UNIX_EPOCH: i64 = 134_774;
const SECONDS_PER_DAY: i64 = 86_400;

/// The file format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// RFC 4180: CRLF, quoted when needed, UTF-8.
    Csv,
    /// One JSON object per line, UTF-8, no BOM.
    JsonLines,
}

/// Options of an export.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Options {
    /// A UTF-8 byte order mark first (CSV only), for Excel.
    pub bom: bool,
    /// Prefix text cells that start with `=`, `+`, `-` or `@` with `'` (CSV
    /// only). Off by default because it alters data.
    pub excel_safe: bool,
}

/// The UTC offset in effect at a FILETIME instant, in seconds.
pub type OffsetFn<'a> = &'a dyn Fn(u64) -> i32;

/// Source names in the export.
#[must_use]
pub const fn source_name(source: Source) -> &'static str {
    match source {
        Source::Rdcw => "rdcw",
        Source::Usn => "usn",
        Source::Reconcile => "reconcile",
        Source::System => "system",
    }
}

/// The source with this export name.
#[must_use]
pub fn source_from_name(name: &str) -> Option<Source> {
    Source::ALL.into_iter().find(|s| source_name(*s) == name)
}

/// The marker column: `gap`, `reconcile` or empty.
#[must_use]
pub const fn marker(row: &Row) -> &'static str {
    if row.kind.is_gap() {
        "gap"
    } else if row.kind.is_reconcile() {
        "reconcile"
    } else {
        ""
    }
}

/// Writes rows to `W`, reusing one line buffer.
#[derive(Debug)]
pub struct RowWriter<W: Write> {
    out: W,
    format: Format,
    options: Options,
    line: String,
    text: String,
    rows: u64,
}

impl<W: Write> RowWriter<W> {
    /// A writer; the header (CSV) and the byte order mark go out first.
    ///
    /// # Errors
    ///
    /// The writer's error.
    pub fn new(mut out: W, format: Format, options: Options) -> io::Result<Self> {
        if format == Format::Csv {
            if options.bom {
                out.write_all(b"\xEF\xBB\xBF")?;
            }
            let mut header = COLUMNS.join(",");
            header.push(',');
            header.push_str(ROOT_UTF16);
            header.push_str("\r\n");
            out.write_all(header.as_bytes())?;
        }
        Ok(Self {
            out,
            format,
            options,
            line: String::with_capacity(512),
            text: String::with_capacity(256),
            rows: 0,
        })
    }

    /// Rows written so far.
    #[must_use]
    pub const fn rows(&self) -> u64 {
        self.rows
    }

    /// Writes one row.
    ///
    /// # Errors
    ///
    /// The writer's error.
    pub fn write_row(&mut self, row: &Row, offset: OffsetFn<'_>) -> io::Result<()> {
        self.line.clear();
        match self.format {
            Format::Csv => self.csv_row(row, offset),
            Format::JsonLines => self.json_row(row, offset),
        }
        self.rows += 1;
        self.out.write_all(self.line.as_bytes())
    }

    /// Flushes and returns the writer.
    ///
    /// # Errors
    ///
    /// The writer's error.
    pub fn finish(mut self) -> io::Result<W> {
        self.out.flush()?;
        Ok(self.out)
    }

    fn csv_row(&mut self, row: &Row, offset: OffsetFn<'_>) {
        let safe = self.options.excel_safe;
        let mut first = true;
        let mut cell = |line: &mut String, value: &str, text: bool| {
            if !first {
                line.push(',');
            }
            first = false;
            csv_cell(line, value, safe && text);
        };
        let mut scratch = String::new();
        let mut put = |line: &mut String, value: &dyn std::fmt::Display, text: bool| {
            scratch.clear();
            let _ = std::fmt::Write::write_fmt(&mut scratch, format_args!("{value}"));
            cell(line, &scratch, text);
        };
        let utc = iso_utc(row.ts);
        let local = iso_local(row.ts, offset(row.ts));
        put(&mut self.line, &row.id, false);
        put(&mut self.line, &utc, false);
        put(&mut self.line, &local, false);
        put(&mut self.line, &row.kind.name(), false);
        put(&mut self.line, &source_name(row.source), false);
        lossy(&mut self.text, &row.path);
        put(&mut self.line, &self.text.clone(), true);
        lossy(&mut self.text, &row.old_path);
        put(&mut self.line, &self.text.clone(), true);
        put(&mut self.line, &row.is_dir, false);
        put(&mut self.line, &opt(row.size), false);
        put(&mut self.line, &opt(row.attrs), false);
        put(
            &mut self.line,
            &format_args!("0x{:08X}", row.raw_mask),
            false,
        );
        put(&mut self.line, &format_args!("{:032x}", row.file_id), false);
        lossy(&mut self.text, &row.root);
        put(&mut self.line, &self.text.clone(), true);
        put(&mut self.line, &marker(row), false);
        put(&mut self.line, &row.flags, false);
        for units in [&row.path, &row.old_path, &row.root] {
            let exact = exact_if_needed(units);
            put(&mut self.line, &exact, false);
        }
        self.line.push_str("\r\n");
    }

    fn json_row(&mut self, row: &Row, offset: OffsetFn<'_>) {
        use std::fmt::Write as _;
        let line = &mut self.line;
        let _ = write!(line, "{{\"id\":{}", row.id);
        json_field(line, "ts_utc", &iso_utc(row.ts));
        json_field(line, "ts_local", &iso_local(row.ts, offset(row.ts)));
        json_field(line, "kind", row.kind.name());
        json_field(line, "source", source_name(row.source));
        lossy(&mut self.text, &row.path);
        json_field(line, "path", &self.text);
        lossy(&mut self.text, &row.old_path);
        json_field(line, "old_path", &self.text);
        let _ = write!(line, ",\"is_dir\":{}", row.is_dir);
        match row.size {
            Some(size) => {
                let _ = write!(line, ",\"size\":{size}");
            }
            None => line.push_str(",\"size\":null"),
        }
        match row.attrs {
            Some(attrs) => {
                let _ = write!(line, ",\"attrs\":{attrs}");
            }
            None => line.push_str(",\"attrs\":null"),
        }
        json_field(line, "raw_mask", &format!("0x{:08X}", row.raw_mask));
        json_field(line, "file_id", &format!("{:032x}", row.file_id));
        lossy(&mut self.text, &row.root);
        json_field(line, "root", &self.text);
        json_field(line, "marker", marker(row));
        let _ = write!(line, ",\"flags\":{}", row.flags);
        for (name, units) in [
            ("path_utf16", &row.path),
            ("old_path_utf16", &row.old_path),
            (ROOT_UTF16, &row.root),
        ] {
            let exact = exact_if_needed(units);
            if !exact.is_empty() {
                json_field(line, name, &exact);
            }
        }
        line.push_str("}\n");
    }
}

fn opt<T: std::fmt::Display>(value: Option<T>) -> String {
    value.map_or_else(String::new, |v| v.to_string())
}

/// Lossy UTF-8 of UTF-16 units into `out` (replacing its content).
fn lossy(out: &mut String, units: &[u16]) {
    out.clear();
    out.extend(
        char::decode_utf16(units.iter().copied())
            .map(|unit| unit.unwrap_or(char::REPLACEMENT_CHARACTER)),
    );
}

/// The exact form when the units are not valid Unicode, else empty.
fn exact_if_needed(units: &[u16]) -> String {
    if char::decode_utf16(units.iter().copied()).all(|unit| unit.is_ok()) {
        String::new()
    } else {
        exact(units)
    }
}

/// The exact form of UTF-16 units: printable ASCII as itself, `\` as `\\`,
/// everything else as `\u{XXXX}` per code unit.
#[must_use]
pub fn exact(units: &[u16]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(units.len());
    for &unit in units {
        match unit {
            0x5C => out.push_str("\\\\"),
            0x20..=0x7E => out.push(char::from(u8::try_from(unit).unwrap_or(b'?'))),
            other => {
                let _ = write!(out, "\\u{{{other:04X}}}");
            }
        }
    }
    out
}

/// The units of an [`exact`] string; `None` when it is not one.
#[must_use]
pub fn parse_exact(text: &str) -> Option<Vec<u16>> {
    let mut out = Vec::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            let mut buf = [0u16; 2];
            out.extend_from_slice(ch.encode_utf16(&mut buf));
            continue;
        }
        match chars.next()? {
            '\\' => out.push(0x5C),
            'u' => {
                if chars.next()? != '{' {
                    return None;
                }
                let mut value: u32 = 0;
                let mut digits = 0;
                loop {
                    let digit = chars.next()?;
                    if digit == '}' {
                        break;
                    }
                    value = value.checked_mul(16)?.checked_add(digit.to_digit(16)?)?;
                    digits += 1;
                    if digits > 4 {
                        return None;
                    }
                }
                if digits == 0 {
                    return None;
                }
                out.push(u16::try_from(value).ok()?);
            }
            _ => return None,
        }
    }
    Some(out)
}

/// One CSV cell: quoted when it contains a comma, quote, CR or LF.
fn csv_cell(line: &mut String, value: &str, excel_safe: bool) {
    let guard = excel_safe && value.starts_with(['=', '+', '-', '@', '\t', '\r']);
    let needs_quotes = value.contains([',', '"', '\r', '\n']) || guard && value.contains('\'');
    if needs_quotes {
        line.push('"');
        if guard {
            line.push('\'');
        }
        for ch in value.chars() {
            if ch == '"' {
                line.push('"');
            }
            line.push(ch);
        }
        line.push('"');
    } else {
        if guard {
            line.push('\'');
        }
        line.push_str(value);
    }
}

/// `,"name":"value"` with JSON escaping.
fn json_field(line: &mut String, name: &str, value: &str) {
    line.push(',');
    line.push('"');
    line.push_str(name);
    line.push_str("\":\"");
    for ch in value.chars() {
        match ch {
            '"' => line.push_str("\\\""),
            '\\' => line.push_str("\\\\"),
            '\n' => line.push_str("\\n"),
            '\r' => line.push_str("\\r"),
            '\t' => line.push_str("\\t"),
            c if u32::from(c) < 0x20 => {
                let _ = std::fmt::Write::write_fmt(line, format_args!("\\u{:04x}", u32::from(c)));
            }
            c => line.push(c),
        }
    }
    line.push('"');
}

/// `YYYY-MM-DDTHH:MM:SS.fffffffZ` for a FILETIME instant.
#[must_use]
pub fn iso_utc(ts: u64) -> String {
    let (text, _) = civil(ts, 0);
    format!("{text}Z")
}

/// The same instant with an offset: `…+02:00`.
#[must_use]
pub fn iso_local(ts: u64, offset_seconds: i32) -> String {
    let (text, _) = civil(ts, offset_seconds);
    let sign = if offset_seconds < 0 { '-' } else { '+' };
    let abs = offset_seconds.unsigned_abs();
    format!("{text}{sign}{:02}:{:02}", abs / 3600, abs % 3600 / 60)
}

/// Date and time of `ts` shifted by `offset_seconds`, fraction in 100 ns.
fn civil(ts: u64, offset_seconds: i32) -> (String, i64) {
    let ts = ts.min(MAX_TS);
    let seconds = i64::try_from(ts / TICKS_PER_SECOND).unwrap_or(i64::MAX / 2);
    let ticks = ts % TICKS_PER_SECOND;
    let shifted = seconds.saturating_add(i64::from(offset_seconds));
    let days = shifted.div_euclid(SECONDS_PER_DAY) - DAYS_TO_UNIX_EPOCH;
    let in_day = shifted.rem_euclid(SECONDS_PER_DAY);
    let (year, month, day) = civil_from_days(days);
    let text = format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{ticks:07}",
        in_day / 3600,
        in_day % 3600 / 60,
        in_day % 60
    );
    (text, shifted)
}

/// Year, month, day of a count of days since 1970-01-01 (proleptic Gregorian).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// The FILETIME ticks of an ISO UTC string written by [`iso_utc`].
#[must_use]
pub fn parse_iso_utc(text: &str) -> Option<u64> {
    let bytes = text.as_bytes();
    if bytes.len() != 28 || bytes.last() != Some(&b'Z') {
        return None;
    }
    let num = |from: usize, to: usize| -> Option<i64> {
        let part = text.get(from..to)?;
        if !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        part.parse().ok()
    };
    for (at, sep) in [
        (4, b'-'),
        (7, b'-'),
        (10, b'T'),
        (13, b':'),
        (16, b':'),
        (19, b'.'),
    ] {
        if bytes.get(at) != Some(&sep) {
            return None;
        }
    }
    let (year, month, day) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (hour, minute, second, ticks) = (num(11, 13)?, num(14, 16)?, num(17, 19)?, num(20, 27)?);
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    let days = days_from_civil(year, month, day) + DAYS_TO_UNIX_EPOCH;
    let total = (days * SECONDS_PER_DAY + hour * 3600 + minute * 60 + second)
        .checked_mul(i64::try_from(TICKS_PER_SECOND).ok()?)?
        .checked_add(ticks)?;
    u64::try_from(total).ok()
}

/// Days since 1970-01-01 of a civil date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year.rem_euclid(400);
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests;
