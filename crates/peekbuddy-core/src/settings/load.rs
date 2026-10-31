//! Parsing `settings.toml` text into [`Settings`] with helpful errors.

use std::fmt;

use super::migrate::{Step, migrate_with};
use super::validate::{Context, Issue, validate};
use super::{
    CURRENT_VERSION, FILTER_KEYS, GENERAL_KEYS, STORE_KEYS, Settings, TOP_KEYS, VIEW_KEYS,
    WATCH_KEYS,
};

/// One thing wrong with the file, with a position when it can be found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// Dotted key path, such as `watch[0].path`; empty for a syntax error.
    pub key_path: String,
    /// 1-based line, when known.
    pub line: Option<usize>,
    /// 1-based column, when known.
    pub column: Option<usize>,
    /// What is wrong.
    pub message: String,
    /// "Did you mean" replacement.
    pub suggestion: Option<String>,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let (Some(line), Some(column)) = (self.line, self.column) {
            write!(f, "{line}:{column}: ")?;
        }
        if !self.key_path.is_empty() {
            write!(f, "{}: ", self.key_path)?;
        }
        write!(f, "{}", self.message)?;
        if let Some(suggestion) = &self.suggestion {
            write!(f, " (did you mean `{suggestion}`?)")?;
        }
        Ok(())
    }
}

/// The result of reading a settings file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Loaded {
    /// A valid file, possibly migrated from an older version (the engine
    /// keeps `settings.v<N>.bak` before writing the migrated file).
    Ok {
        /// The settings, with normalised paths.
        settings: Box<Settings>,
        /// The version the file had before migration.
        migrated_from: Option<u32>,
    },
    /// The file is from a newer build. It is read leniently (keys this build
    /// does not know are ignored) and must never be written back.
    TooNew {
        /// The file's version.
        version: u32,
        /// What could be understood; defaults where nothing could.
        settings: Box<Settings>,
    },
    /// The file is unusable; every problem found is listed. The engine keeps
    /// running with no watches applied and leaves the file untouched.
    Invalid(Vec<Problem>),
}

/// Edit distance, for "did you mean".
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut previous = row.first().copied().unwrap_or(0);
        if let Some(first) = row.first_mut() {
            *first = i + 1;
        }
        for (j, cb) in b.iter().enumerate() {
            let above = row.get(j + 1).copied().unwrap_or(0);
            let left = row.get(j).copied().unwrap_or(0);
            let cost = usize::from(ca != *cb);
            let value = (previous + cost).min(above + 1).min(left + 1);
            previous = above;
            if let Some(slot) = row.get_mut(j + 1) {
                *slot = value;
            }
        }
    }
    row.last().copied().unwrap_or(0)
}

/// The known name closest to `word`, if it is close enough to be a typo.
pub(crate) fn closest<'a>(word: &str, known: impl IntoIterator<Item = &'a str>) -> Option<String> {
    known
        .into_iter()
        .map(|candidate| (distance(&word.to_lowercase(), candidate), candidate))
        .filter(|(d, candidate)| *d <= 2.max(candidate.len() / 4) && *d < word.len().max(1))
        .min_by_key(|(d, _)| *d)
        .map(|(_, candidate)| candidate.to_owned())
}

fn line_column(text: &str, offset: usize) -> (usize, usize) {
    let head = text.get(..offset.min(text.len())).unwrap_or_default();
    let line = head.matches('\n').count() + 1;
    let column = head
        .rsplit('\n')
        .next()
        .map_or(0, |last| last.chars().count())
        + 1;
    (line, column)
}

/// Position of the first line that sets `key` (`key =` at the start).
fn locate_key(text: &str, key: &str) -> Option<usize> {
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let header = trimmed
            .trim_start_matches('[')
            .strip_prefix(key)
            .is_some_and(|rest| rest.starts_with(']') || rest.starts_with('.'))
            && trimmed.starts_with('[');
        if header
            || trimmed
                .strip_prefix(key)
                .is_some_and(|rest| rest.trim_start().starts_with('='))
        {
            return Some(offset + (line.len() - trimmed.len()));
        }
        offset += line.len();
    }
    None
}

fn locate_value(text: &str, value: &str) -> Option<usize> {
    let quoted_basic = format!("\"{value}\"");
    let quoted_literal = format!("'{value}'");
    text.find(&quoted_basic)
        .or_else(|| text.find(&quoted_literal))
        .or_else(|| text.find(value))
}

fn from_issue(text: &str, issue: Issue) -> Problem {
    let last = issue
        .key_path
        .rsplit('.')
        .next()
        .map_or("", |part| part.split('[').next().unwrap_or(part));
    let offset = issue
        .value
        .as_deref()
        .and_then(|value| locate_value(text, value))
        .or_else(|| locate_key(text, last));
    let (line, column) = offset.map_or((None, None), |at| {
        let (line, column) = line_column(text, at);
        (Some(line), Some(column))
    });
    Problem {
        key_path: issue.key_path,
        line,
        column,
        message: issue.message,
        suggestion: issue.suggestion,
    }
}

/// Unknown keys in `table` against `known`, as issues under `prefix`.
fn unknown_keys(table: &toml::Table, known: &[&str], prefix: &str, out: &mut Vec<Issue>) {
    for key in table.keys() {
        if !known.contains(&key.as_str()) {
            let path = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            out.push(Issue {
                key_path: path,
                message: format!("unknown key `{key}`"),
                value: None,
                suggestion: closest(key, known.iter().copied()),
            });
        }
    }
}

fn unknown_issues(table: &toml::Table) -> Vec<Issue> {
    let mut out = Vec::new();
    unknown_keys(table, TOP_KEYS, "", &mut out);
    for (name, keys) in [
        ("general", GENERAL_KEYS),
        ("store", STORE_KEYS),
        ("view", VIEW_KEYS),
        ("filters", FILTER_KEYS),
    ] {
        if let Some(toml::Value::Table(section)) = table.get(name) {
            unknown_keys(section, keys, name, &mut out);
        }
    }
    if let Some(toml::Value::Array(items)) = table.get("watch") {
        for (index, item) in items.iter().enumerate() {
            if let toml::Value::Table(watch) = item {
                unknown_keys(watch, WATCH_KEYS, &format!("watch[{index}]"), &mut out);
            }
        }
    }
    out
}

fn syntax_problem(text: &str, error: &toml::de::Error) -> Problem {
    let (line, column) = error.span().map_or((None, None), |span| {
        let (line, column) = line_column(text, span.start);
        (Some(line), Some(column))
    });
    Problem {
        key_path: String::new(),
        line,
        column,
        message: error.message().trim().to_owned(),
        suggestion: None,
    }
}

/// Reads `text` with this build's migration steps.
#[must_use]
pub fn load(text: &str, ctx: &Context) -> Loaded {
    load_with(text, ctx, CURRENT_VERSION, &[])
}

/// Reads `text` as a settings file of version `target`, migrating older files
/// with `steps` (test seam; [`load`] uses the build's own, currently none).
#[must_use]
pub fn load_with(text: &str, ctx: &Context, target: u32, steps: &[Step]) -> Loaded {
    let mut table: toml::Table = match text.parse() {
        Ok(table) => table,
        Err(error) => return Loaded::Invalid(vec![syntax_problem(text, &error)]),
    };
    let version = match table.get("version") {
        Some(toml::Value::Integer(version)) => u32::try_from(*version).ok().filter(|v| *v >= 1),
        _ => None,
    };
    let Some(version) = version else {
        return Loaded::Invalid(vec![from_issue(
            text,
            Issue {
                key_path: String::from("version"),
                message: String::from("`version` must be an integer of at least 1"),
                value: None,
                suggestion: None,
            },
        )]);
    };

    if version > target {
        // Keys this build does not know are ignored by the deserialiser, and a
        // file it cannot read at all still loads as the defaults.
        let settings = table.try_into::<Settings>().unwrap_or_default();
        return Loaded::TooNew {
            version,
            settings: Box::new(settings),
        };
    }
    let migrated_from = (version < target).then_some(version);
    if migrated_from.is_some() {
        table = match migrate_with(table, version, target, steps) {
            Ok(table) => table,
            Err(error) => {
                return Loaded::Invalid(vec![Problem {
                    key_path: String::from("version"),
                    line: None,
                    column: None,
                    message: error.to_string(),
                    suggestion: None,
                }]);
            }
        };
    }

    let unknown = unknown_issues(&table);
    if !unknown.is_empty() {
        return Loaded::Invalid(
            unknown
                .into_iter()
                .map(|issue| from_issue(text, issue))
                .collect(),
        );
    }
    let mut settings: Settings = match table.try_into() {
        Ok(settings) => settings,
        Err(error) => {
            // The table has lost its spans; the original text still has them.
            let located = match (migrated_from, toml::from_str::<Settings>(text)) {
                (None, Err(spanned)) => spanned,
                _ => error,
            };
            return Loaded::Invalid(vec![syntax_problem(text, &located)]);
        }
    };
    let issues = validate(&mut settings, ctx);
    if issues.is_empty() {
        Loaded::Ok {
            settings: Box::new(settings),
            migrated_from,
        }
    } else {
        Loaded::Invalid(
            issues
                .into_iter()
                .map(|issue| from_issue(text, issue))
                .collect(),
        )
    }
}

#[cfg(test)]
#[expect(
    clippy::panic,
    clippy::indexing_slicing,
    clippy::format_push_string,
    clippy::unnecessary_wraps,
    reason = "tests may panic and index"
)]
mod tests;
