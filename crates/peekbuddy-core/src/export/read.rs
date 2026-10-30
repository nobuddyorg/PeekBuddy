//! Reading an export back into rows (#56).
//!
//! The inverse of [`RowWriter`](super::RowWriter), for the round-trip tests,
//! the churn comparator (#25) and fuzzing. Input is arbitrary text: nothing
//! here panics, every defect is a [`ReadError`] with the line it was on.

use std::fmt;

use super::{COLUMNS, ROOT_UTF16, parse_exact, parse_iso_utc, source_from_name};
use crate::event::Kind;
use crate::query::Row;

/// Why a line could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadError {
    /// 1-based line (CSV: record) number.
    pub line: usize,
    /// What was wrong.
    pub what: String,
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.what)
    }
}

impl std::error::Error for ReadError {}

fn err(line: usize, what: impl Into<String>) -> ReadError {
    ReadError {
        line,
        what: what.into(),
    }
}

/// How to read a CSV written with the Excel-safe option.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReadOptions {
    /// Remove the `'` that the Excel-safe mode put before `=`, `+`, `-`, `@`.
    pub excel_safe: bool,
}

/// Reads a CSV export (with or without the byte order mark).
///
/// # Errors
///
/// The first record that is not a valid row.
pub fn read_csv(text: &str, options: ReadOptions) -> Result<Vec<Row>, ReadError> {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let mut records = csv_records(text)?.into_iter();
    let Some(header) = records.next() else {
        return Ok(Vec::new());
    };
    for (expected, found) in COLUMNS.iter().zip(&header) {
        if *expected != found.as_str() {
            return Err(err(1, format!("column {expected} expected, found {found}")));
        }
    }
    let mut rows = Vec::new();
    for (index, record) in records.enumerate() {
        let line = index + 2;
        let by_name = |name: &str| -> Option<String> {
            let at = header.iter().position(|column| column == name)?;
            let value = record.get(at)?.clone();
            Some(if options.excel_safe && is_guarded(&value) {
                value.chars().skip(1).collect()
            } else {
                value
            })
        };
        if record.len() < COLUMNS.len() {
            return Err(err(line, "too few columns"));
        }
        rows.push(row_from(&by_name, line)?);
    }
    Ok(rows)
}

/// Reads a JSON Lines export.
///
/// # Errors
///
/// The first line that is not a valid row.
pub fn read_jsonl(text: &str) -> Result<Vec<Row>, ReadError> {
    let mut rows = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let number = index + 1;
        if line.trim().is_empty() {
            continue;
        }
        let value: serde_json::Value =
            serde_json::from_str(line).map_err(|e| err(number, e.to_string()))?;
        let Some(object) = value.as_object() else {
            return Err(err(number, "not an object"));
        };
        let by_name = |name: &str| -> Option<String> {
            match object.get(name)? {
                serde_json::Value::Null => Some(String::new()),
                serde_json::Value::String(s) => Some(s.clone()),
                other => Some(other.to_string()),
            }
        };
        rows.push(row_from(&by_name, number)?);
    }
    Ok(rows)
}

/// An Excel-safe cell: a `'` before a formula starter.
fn is_guarded(value: &str) -> bool {
    let mut chars = value.chars();
    chars.next() == Some('\'')
        && chars
            .next()
            .is_some_and(|c| matches!(c, '=' | '+' | '-' | '@' | '\t' | '\r'))
}

fn row_from(get: &dyn Fn(&str) -> Option<String>, line: usize) -> Result<Row, ReadError> {
    let text = |name: &str| get(name).unwrap_or_default();
    let need = |name: &str| get(name).ok_or_else(|| err(line, format!("{name} missing")));
    let number = |name: &str| -> Result<i64, ReadError> {
        need(name)?
            .parse()
            .map_err(|_| err(line, format!("{name} is not a number")))
    };
    let optional = |name: &str| -> Result<Option<u64>, ReadError> {
        let value = text(name);
        if value.is_empty() {
            return Ok(None);
        }
        value
            .parse()
            .map(Some)
            .map_err(|_| err(line, format!("{name} is not a number")))
    };
    let hex = |name: &str, prefix: &str| -> Result<u128, ReadError> {
        let value = need(name)?;
        let digits = value.strip_prefix(prefix).unwrap_or(&value);
        u128::from_str_radix(digits, 16).map_err(|_| err(line, format!("{name} is not hex")))
    };
    let units = |plain: &str, exact: &str| -> Result<Vec<u16>, ReadError> {
        let side = text(exact);
        if side.is_empty() {
            Ok(text(plain).encode_utf16().collect())
        } else {
            parse_exact(&side).ok_or_else(|| err(line, format!("{exact} is not an exact name")))
        }
    };
    let path = units("path", "path_utf16")?;
    let name = path
        .rsplit(|unit| *unit == 0x5C)
        .next()
        .map(<[u16]>::to_vec)
        .unwrap_or_default();
    Ok(Row {
        id: number("id")?,
        ts: parse_iso_utc(&need("ts_utc")?)
            .ok_or_else(|| err(line, "ts_utc is not a timestamp"))?,
        kind: Kind::from_name(&need("kind")?).ok_or_else(|| err(line, "unknown kind"))?,
        source: source_from_name(&need("source")?).ok_or_else(|| err(line, "unknown source"))?,
        raw_mask: u32::try_from(hex("raw_mask", "0x")?)
            .map_err(|_| err(line, "raw_mask too big"))?,
        is_dir: match need("is_dir")?.as_str() {
            "true" => true,
            "false" => false,
            _ => return Err(err(line, "is_dir is not a boolean")),
        },
        size: optional("size")?,
        attrs: optional("attrs")?
            .map(|v| u32::try_from(v).map_err(|_| err(line, "attrs too big")))
            .transpose()?,
        file_id: hex("file_id", "")?,
        name,
        path,
        old_path: units("old_path", "old_path_utf16")?,
        root: units("root", ROOT_UTF16)?,
        flags: u16::try_from(number("flags")?).map_err(|_| err(line, "flags out of range"))?,
    })
}

/// Splits RFC 4180 text into records of cells.
fn csv_records(text: &str) -> Result<Vec<Vec<String>>, ReadError> {
    let mut records = Vec::new();
    let mut record: Vec<String> = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut was_quoted = false;
    let mut chars = text.chars().peekable();
    let mut line = 1;
    while let Some(ch) = chars.next() {
        if quoted {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    cell.push('"');
                } else {
                    quoted = false;
                }
            } else {
                if ch == '\n' {
                    line += 1;
                }
                cell.push(ch);
            }
            continue;
        }
        match ch {
            '"' if cell.is_empty() && !was_quoted => {
                quoted = true;
                was_quoted = true;
            }
            '"' => return Err(err(line, "a quote inside an unquoted cell")),
            ',' => {
                record.push(std::mem::take(&mut cell));
                was_quoted = false;
            }
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' => {
                record.push(std::mem::take(&mut cell));
                was_quoted = false;
                records.push(std::mem::take(&mut record));
                line += 1;
            }
            other => {
                if was_quoted {
                    return Err(err(line, "text after a closing quote"));
                }
                cell.push(other);
            }
        }
    }
    if quoted {
        return Err(err(line, "unterminated quote"));
    }
    if !cell.is_empty() || !record.is_empty() || was_quoted {
        record.push(cell);
        records.push(record);
    }
    Ok(records)
}
