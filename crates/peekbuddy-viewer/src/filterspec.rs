//! The filter as the user states it (#67), and as one line of text.
//!
//! [`FilterSpec`] is what the filter bar edits: a name, kind chips, a root, a
//! subtree, a time span, a glob or regex, and whether rows the view hides are
//! shown. It has a canonical text form that can be copied, pasted and kept in
//! `settings.toml`:
//!
//! ```text
//! name:"report" kind:created,deleted root:"C:\work" in:"C:\work\src"
//! since:1h glob:"*.tmp" hidden:show
//! ```
//!
//! `parse(print(spec)) == spec` for every spec, and `parse` never panics on
//! any text (a fuzz target runs it). [`FilterSpec::to_filter`] turns a spec
//! into the query filter of `peekbuddy-core`, compiling the glob or regex
//! with the same engine that filters at capture (#40), so the semantics are
//! the same. An error names the field it belongs to, so the bar can show it
//! next to that field while the previous results stay.

use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;

use peekbuddy_core::event::{Flags, GapReason, Kind, WatchState};
use peekbuddy_core::export::{iso_utc, parse_iso_utc};
use peekbuddy_core::filter::{FilterSet, KindSet, Pattern, Rule};
use peekbuddy_core::query::{Filter, TimeRange};

/// FILETIME ticks in a second.
const SECOND: u64 = 10_000_000;
/// Ticks in a day.
const DAY: u64 = 86_400 * SECOND;

/// A kind chip of the filter bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Chip {
    /// Files and folders that appeared.
    Created,
    /// Data changes.
    Modified,
    /// Files and folders that went away.
    Deleted,
    /// Both halves of a rename.
    Renamed,
    /// Attributes, security, links.
    Attributes,
    /// Gaps in the stream and failures: something may be missing.
    Gaps,
    /// The engine's own lifecycle and the watches' state.
    Engine,
}

impl Chip {
    /// Every chip, in the order of the bar.
    pub const ALL: [Self; 7] = [
        Self::Created,
        Self::Modified,
        Self::Deleted,
        Self::Renamed,
        Self::Attributes,
        Self::Gaps,
        Self::Engine,
    ];

    /// The word in the text form.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Modified => "modified",
            Self::Deleted => "deleted",
            Self::Renamed => "renamed",
            Self::Attributes => "attributes",
            Self::Gaps => "gaps",
            Self::Engine => "engine",
        }
    }

    /// The chip's label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Created => "Created",
            Self::Modified => "Modified",
            Self::Deleted => "Deleted",
            Self::Renamed => "Renamed",
            Self::Attributes => "Attributes",
            Self::Gaps => "Gaps",
            Self::Engine => "Engine",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|chip| chip.name() == name)
    }

    /// The chip that holds `kind`, for "filter to this kind".
    #[must_use]
    pub fn of(kind: Kind) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|chip| KindSet::of(&chip.kinds()).contains(kind))
    }

    /// The kinds the chip stands for.
    #[must_use]
    pub fn kinds(self) -> Vec<Kind> {
        match self {
            Self::Created => vec![Kind::Created, Kind::ReconciledCreated],
            Self::Modified => vec![
                Kind::Modified,
                Kind::ReconciledModified,
                Kind::StreamChanged,
            ],
            Self::Deleted => vec![Kind::Deleted, Kind::ReconciledDeleted],
            Self::Renamed => vec![Kind::RenamedFrom, Kind::RenamedTo, Kind::ReconciledRenamed],
            Self::Attributes => vec![
                Kind::AttributesChanged,
                Kind::SecurityChanged,
                Kind::LinkChanged,
            ],
            Self::Gaps => vec![
                Kind::Overflow(GapReason::BufferOverflow),
                Kind::AccessDenied,
                Kind::StoreDegraded,
                Kind::EngineAbnormalExit,
            ],
            Self::Engine => vec![
                Kind::WatchStateChanged(WatchState::Live),
                Kind::ReconcileStarted,
                Kind::ReconcileFinished,
                Kind::BaselineEstablished,
                Kind::StoreRecovered,
                Kind::EngineStarted,
                Kind::EngineStopped,
                Kind::SegmentRotated,
                Kind::RetentionPurged,
                Kind::SettingsChanged,
            ],
        }
    }
}

/// How far back the view reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Span {
    /// All of it.
    #[default]
    Any,
    /// The last five minutes.
    Last5Minutes,
    /// The last hour.
    LastHour,
    /// Since local midnight.
    Today,
    /// From and to, FILETIME ticks (UTC); at least one is set.
    Between {
        /// Inclusive start.
        from: Option<u64>,
        /// Inclusive end.
        to: Option<u64>,
    },
}

/// Which matcher the advanced field uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatternKind {
    /// A gitignore-style glob.
    Glob,
    /// A regex searched anywhere in the path.
    Regex,
}

/// The filter fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    /// The name search.
    Name,
    /// The kind chips.
    Kinds,
    /// The watch root.
    Root,
    /// The subtree.
    Subtree,
    /// The time span.
    Span,
    /// The glob or regex.
    Pattern,
}

/// Why a spec cannot be turned into a filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecError {
    /// Where the bar shows it.
    pub field: Field,
    /// What is wrong, in words.
    pub message: String,
}

impl fmt::Display for SpecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

/// What the user asks for.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FilterSpec {
    /// A substring of the name (the last component). Empty: no name filter.
    pub name: String,
    /// The kinds shown; none selected means all.
    pub kinds: BTreeSet<Chip>,
    /// The watch root (an absolute path).
    pub root: Option<String>,
    /// A folder: only rows in it or below.
    pub subtree: Option<String>,
    /// How far back.
    pub span: Span,
    /// A glob or regex over the path; the text is never empty.
    pub pattern: Option<(PatternKind, String)>,
    /// Show the rows that the view hides by default (cloud sync churn).
    pub show_hidden: bool,
}

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

impl FilterSpec {
    /// Whether nothing is asked for.
    #[must_use]
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// The query filter. `now` is the current FILETIME; `offset` the UTC
    /// offset of the local time zone in seconds (for "today").
    ///
    /// # Errors
    ///
    /// The field that cannot be used, with the matcher's message.
    pub fn to_filter(&self, now: u64, offset: i32) -> Result<Filter, SpecError> {
        let kinds = (!self.kinds.is_empty()).then(|| {
            let all: Vec<Kind> = self.kinds.iter().flat_map(|chip| chip.kinds()).collect();
            KindSet::of(&all)
        });
        let pattern = match &self.pattern {
            None => None,
            Some((kind, text)) => {
                let pattern = match kind {
                    PatternKind::Glob => Pattern::Glob(text.clone()),
                    PatternKind::Regex => Pattern::Regex(text.clone()),
                };
                let set =
                    FilterSet::compile(&[Rule::include(pattern)]).map_err(|error| SpecError {
                        field: Field::Pattern,
                        message: error.to_string(),
                    })?;
                Some(Arc::new(set))
            }
        };
        let time = match self.span {
            Span::Any => TimeRange::default(),
            Span::Last5Minutes => TimeRange {
                from: Some(now.saturating_sub(300 * SECOND)),
                to: None,
            },
            Span::LastHour => TimeRange {
                from: Some(now.saturating_sub(3_600 * SECOND)),
                to: None,
            },
            Span::Today => {
                let shift = u64::from(offset.unsigned_abs()) * SECOND;
                let local = if offset >= 0 {
                    now.saturating_add(shift)
                } else {
                    now.saturating_sub(shift)
                };
                let midnight = local - local % DAY;
                let from = if offset >= 0 {
                    midnight.saturating_sub(shift)
                } else {
                    midnight.saturating_add(shift)
                };
                TimeRange {
                    from: Some(from),
                    to: None,
                }
            }
            Span::Between { from, to } => {
                if let (Some(from), Some(to)) = (from, to)
                    && from > to
                {
                    return Err(SpecError {
                        field: Field::Span,
                        message: String::from("the end is before the start"),
                    });
                }
                TimeRange { from, to }
            }
        };
        Ok(Filter {
            kinds,
            root: self.root.as_deref().map(units),
            time,
            subtree: self.subtree.as_deref().map(units),
            name: (!self.name.is_empty()).then(|| self.name.clone()),
            pattern,
            hide: if self.show_hidden {
                Flags::NONE
            } else {
                Flags::CLOUD_CHURN
            },
        })
    }

    /// The filter that narrows the most, for "nothing matches" to name. The
    /// order is by how surprising an empty result is.
    #[must_use]
    pub fn narrowest(&self) -> Option<String> {
        if let Some((kind, text)) = &self.pattern {
            let word = match kind {
                PatternKind::Glob => "glob",
                PatternKind::Regex => "regex",
            };
            return Some(format!("the {word} {text}"));
        }
        if !self.name.is_empty() {
            return Some(format!("the name \"{}\"", self.name));
        }
        if let Some(folder) = &self.subtree {
            return Some(format!("the folder {folder}"));
        }
        if let Some(root) = &self.root {
            return Some(format!("the watch {root}"));
        }
        if !self.kinds.is_empty() {
            return Some(String::from("the kinds chosen"));
        }
        (self.span != Span::Any).then(|| String::from("the time span"))
    }

    /// The canonical text form.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if !self.name.is_empty() {
            parts.push(format!("name:{}", quote(&self.name)));
        }
        if !self.kinds.is_empty() {
            let words: Vec<&str> = self.kinds.iter().map(|chip| chip.name()).collect();
            parts.push(format!("kind:{}", words.join(",")));
        }
        if let Some(root) = &self.root {
            parts.push(format!("root:{}", quote(root)));
        }
        if let Some(folder) = &self.subtree {
            parts.push(format!("in:{}", quote(folder)));
        }
        match self.span {
            Span::Any => {}
            Span::Last5Minutes => parts.push(String::from("since:5m")),
            Span::LastHour => parts.push(String::from("since:1h")),
            Span::Today => parts.push(String::from("since:today")),
            Span::Between { from, to } => {
                if let Some(from) = from {
                    parts.push(format!("from:{}", iso_utc(from)));
                }
                if let Some(to) = to {
                    parts.push(format!("to:{}", iso_utc(to)));
                }
            }
        }
        match &self.pattern {
            Some((PatternKind::Glob, text)) => parts.push(format!("glob:{}", quote(text))),
            Some((PatternKind::Regex, text)) => parts.push(format!("re:{}", quote(text))),
            None => {}
        }
        if self.show_hidden {
            parts.push(String::from("hidden:show"));
        }
        parts.join(" ")
    }

    /// Reads the text form.
    ///
    /// # Errors
    ///
    /// The first problem, with its position in the text.
    pub fn parse(text: &str) -> Result<Self, ParseError> {
        Parser::new(text).run()
    }
}

/// `text` in quotes, with `"` and `\` escaped.
fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        if matches!(ch, '"' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('"');
    out
}

/// A problem in the text form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// Byte offset of the problem.
    pub at: usize,
    /// What is wrong.
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (at {})", self.message, self.at)
    }
}

impl std::error::Error for ParseError {}

struct Parser<'a> {
    text: &'a str,
    at: usize,
}

impl<'a> Parser<'a> {
    const fn new(text: &'a str) -> Self {
        Self { text, at: 0 }
    }

    fn fail<T>(at: usize, message: impl Into<String>) -> Result<T, ParseError> {
        Err(ParseError {
            at,
            message: message.into(),
        })
    }

    fn rest(&self) -> &'a str {
        self.text.get(self.at..).unwrap_or_default()
    }

    fn skip_space(&mut self) {
        let rest = self.rest();
        self.at += rest.len() - rest.trim_start().len();
    }

    /// The key of the next token, up to its colon.
    fn key(&mut self) -> Result<&'a str, ParseError> {
        let rest = self.rest();
        let end = rest
            .find(|ch: char| !(ch.is_ascii_lowercase() || ch == '_'))
            .unwrap_or(rest.len());
        let key = rest.get(..end).unwrap_or_default();
        if key.is_empty() || !rest.get(end..).is_some_and(|tail| tail.starts_with(':')) {
            return Self::fail(self.at, "expected a key and a colon, like name:\"x\"");
        }
        self.at += end + 1;
        Ok(key)
    }

    /// A quoted or a bare value.
    fn value(&mut self) -> Result<String, ParseError> {
        let start = self.at;
        let rest = self.rest();
        if let Some(body) = rest.strip_prefix('"') {
            let mut out = String::new();
            let mut chars = body.char_indices();
            while let Some((offset, ch)) = chars.next() {
                match ch {
                    '"' => {
                        self.at += 1 + offset + 1;
                        return Ok(out);
                    }
                    '\\' => match chars.next() {
                        Some((_, escaped @ ('"' | '\\'))) => out.push(escaped),
                        _ => {
                            return Self::fail(
                                start + 1 + offset,
                                "\\ must be followed by \" or \\",
                            );
                        }
                    },
                    other => out.push(other),
                }
            }
            return Self::fail(start, "this quote is never closed");
        }
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let word = rest.get(..end).unwrap_or_default();
        if word.is_empty() {
            return Self::fail(start, "a value is missing");
        }
        self.at += end;
        Ok(word.to_owned())
    }

    fn run(mut self) -> Result<FilterSpec, ParseError> {
        let mut spec = FilterSpec::default();
        let mut seen: Vec<&str> = Vec::new();
        let (mut from, mut to) = (None, None);
        loop {
            self.skip_space();
            if self.rest().is_empty() {
                break;
            }
            let key_at = self.at;
            let key = self.key()?;
            let value_at = self.at;
            let value = self.value()?;
            if seen.contains(&key) {
                return Self::fail(key_at, format!("{key}: is given twice"));
            }
            seen.push(key);
            match key {
                "name" => spec.name = value,
                "kind" => {
                    for word in value.split(',') {
                        match Chip::from_name(word) {
                            Some(chip) => {
                                spec.kinds.insert(chip);
                            }
                            None => {
                                return Self::fail(
                                    value_at,
                                    format!(
                                        "unknown kind `{word}` (created, modified, deleted, \
                                         renamed, attributes, gaps, engine)"
                                    ),
                                );
                            }
                        }
                    }
                }
                "root" => spec.root = Some(value),
                "in" => spec.subtree = Some(value),
                "since" => {
                    spec.span = match value.as_str() {
                        "5m" => Span::Last5Minutes,
                        "1h" => Span::LastHour,
                        "today" => Span::Today,
                        _ => return Self::fail(value_at, "since: takes 5m, 1h or today"),
                    };
                }
                "from" | "to" => {
                    let Some(ticks) = parse_iso_utc(&value) else {
                        return Self::fail(
                            value_at,
                            format!("{key}: takes a UTC time like 2026-10-26T12:00:00.0000000Z"),
                        );
                    };
                    if key == "from" {
                        from = Some(ticks);
                    } else {
                        to = Some(ticks);
                    }
                }
                "glob" | "re" => {
                    if spec.pattern.is_some() {
                        return Self::fail(key_at, "give a glob or a regex, not both");
                    }
                    if !value.is_empty() {
                        let kind = if key == "glob" {
                            PatternKind::Glob
                        } else {
                            PatternKind::Regex
                        };
                        spec.pattern = Some((kind, value));
                    }
                }
                "hidden" => {
                    if value != "show" {
                        return Self::fail(value_at, "hidden: takes show");
                    }
                    spec.show_hidden = true;
                }
                other => return Self::fail(key_at, format!("unknown key `{other}`")),
            }
        }
        if from.is_some() || to.is_some() {
            if spec.span != Span::Any {
                return Self::fail(0, "since: and from:/to: exclude each other");
            }
            spec.span = Span::Between { from, to };
        }
        Ok(spec)
    }
}

/// A time typed by the user, in local time: `2026-10-26`,
/// `2026-10-26 14:30` or `2026-10-26 14:30:15` (a `T` instead of the space is
/// accepted too). `offset` is the UTC offset in seconds east. With `end` the
/// result is the last tick of the day, minute or second given, so that a
/// range ending on a date includes that whole date.
///
/// # Errors
///
/// What is wrong with the text, in words for the field.
pub fn parse_local_time(text: &str, offset: i32, end: bool) -> Result<u64, String> {
    const HELP: &str = "use 2026-10-26, 2026-10-26 14:30 or 2026-10-26 14:30:15";
    let text = text.trim();
    let (date, clock) = match text.split_once([' ', 'T']) {
        Some((date, clock)) => (date, Some(clock.trim())),
        None => (text, None),
    };
    if date.len() != 10 {
        return Err(HELP.to_owned());
    }
    let (clock, unit) = match clock {
        None => (String::from("00:00:00"), DAY),
        Some(clock) if clock.len() == 5 => (format!("{clock}:00"), 60 * SECOND),
        Some(clock) if clock.len() == 8 => (clock.to_owned(), SECOND),
        Some(_) => return Err(HELP.to_owned()),
    };
    let utc = parse_iso_utc(&format!("{date}T{clock}.0000000Z")).ok_or_else(|| HELP.to_owned())?;
    let shift = u64::from(offset.unsigned_abs()) * SECOND;
    let local = if end {
        utc.saturating_add(unit - 1)
    } else {
        utc
    };
    // Local time is UTC plus the offset.
    Ok(if offset >= 0 {
        local.saturating_sub(shift)
    } else {
        local.saturating_add(shift)
    })
}

/// A time as `parse_local_time` reads it: `2026-10-26 14:30:15`, local.
#[must_use]
pub fn format_local_time(ticks: u64, offset: i32) -> String {
    let iso = peekbuddy_core::export::iso_local(ticks, offset);
    format!(
        "{} {}",
        iso.get(..10).unwrap_or_default(),
        iso.get(11..19).unwrap_or_default()
    )
}

#[cfg(test)]
mod tests;
