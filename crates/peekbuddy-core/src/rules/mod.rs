//! Rules and notifications (#81): the model in `settings.toml`, the
//! evaluator that runs on the capture side and the placeholder substitution
//! of the run-a-command action.
//!
//! A rule says: when at least `threshold` events that match (watches, glob,
//! kinds) happen within `window_s` seconds, do the actions, then stay quiet
//! for `cooldown_s`. Local only: a toast, a tray balloon, or a command the
//! user explicitly opted into. Nothing here touches Win32.
//!
//! - [`Rule`] and [`Command`] are what the file holds, with [`check`] for the
//!   semantic validation (the settings validator calls it).
//! - [`eval::Evaluator`] is compiled from the rules and evaluates one event
//!   without allocating (F10).
//! - [`subst::expand`] fills the argv template of a command: a placeholder is
//!   only ever a whole argument, so a hostile file name cannot add arguments.

pub mod eval;
pub mod notify;
pub mod subst;

use serde::{Deserialize, Serialize};

use crate::event::Kind;
use crate::filter::{FilterSet, KindSet, Pattern, Rule as FilterRule};

/// Most rules in a file.
pub const MAX_RULES: usize = 100;
/// Largest `threshold`; the counter holds this many timestamps per rule.
pub const MAX_THRESHOLD: u32 = 1_000;
/// Longest window, in seconds.
pub const MAX_WINDOW_S: u32 = 3_600;
/// Longest cooldown, in seconds (one day).
pub const MAX_COOLDOWN_S: u32 = 86_400;
/// Longest time a command may run, in seconds.
pub const MAX_TIMEOUT_S: u32 = 3_600;
/// Longest rule name, in characters.
pub const MAX_NAME_CHARS: usize = 80;
/// Most arguments of a command, the program included.
pub const MAX_ARGV: usize = 32;
/// Longest single argument template, in bytes.
pub const MAX_ARG_BYTES: usize = 1_024;

/// The keys of one `[[rule]]`, for the unknown-key check.
pub const RULE_KEYS: &[&str] = &[
    "name",
    "enabled",
    "watches",
    "glob",
    "kinds",
    "threshold",
    "window_s",
    "cooldown_s",
    "toast",
    "command",
];

/// The keys of the `command` table of a rule.
pub const COMMAND_KEYS: &[&str] = &["argv", "timeout_s", "confirmed"];

/// Run a program when a rule fires (explicit opt-in, documented risk).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Command {
    /// The program, then its arguments. An argument that is exactly
    /// `%path%`, `%kind%`, `%root%` or `%ts%` is replaced by that value; no
    /// other text is interpreted, and nothing goes through a shell.
    pub argv: Vec<String>,
    /// Seconds before the program is killed.
    pub timeout_s: u32,
    /// The user has read the warning and wants the command to run. A rule
    /// with a command and no `confirmed = true` is refused.
    pub confirmed: bool,
}

impl Default for Command {
    fn default() -> Self {
        Self {
            argv: Vec::new(),
            timeout_s: 30,
            confirmed: false,
        }
    }
}

/// One `[[rule]]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rule {
    /// What the notification calls the rule.
    pub name: String,
    /// Off keeps the rule in the file without evaluating it.
    pub enabled: bool,
    /// Ids of the watches the rule applies to; empty is every watch.
    pub watches: Vec<String>,
    /// A gitignore-style glob on the path (the filter engine of #40); empty
    /// matches every path.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub glob: String,
    /// Event kinds by their export names (`created`, `deleted`, ...); empty
    /// is every kind of file change.
    pub kinds: Vec<String>,
    /// Matches needed within the window.
    pub threshold: u32,
    /// The window, in seconds.
    pub window_s: u32,
    /// Seconds between two firings of the rule.
    pub cooldown_s: u32,
    /// Show a toast (or a tray balloon when toasts are not available).
    pub toast: bool,
    /// Run a program.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<Command>,
}

impl Default for Rule {
    fn default() -> Self {
        Self {
            name: String::new(),
            enabled: true,
            watches: Vec::new(),
            glob: String::new(),
            kinds: Vec::new(),
            threshold: 1,
            window_s: 10,
            cooldown_s: 60,
            toast: true,
            command: None,
        }
    }
}

/// Whether `kind` is a change of a path, which a rule can match.
#[must_use]
pub const fn is_path_kind(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::Created
            | Kind::Modified
            | Kind::Deleted
            | Kind::RenamedFrom
            | Kind::RenamedTo
            | Kind::AttributesChanged
            | Kind::SecurityChanged
            | Kind::StreamChanged
            | Kind::LinkChanged
            | Kind::ReconciledCreated
            | Kind::ReconciledModified
            | Kind::ReconciledDeleted
            | Kind::ReconciledRenamed
    )
}

/// The kinds named by `names`; every path kind when it is empty.
///
/// # Errors
///
/// The first name that is not the export name of a path kind.
pub fn kind_set(names: &[String]) -> Result<KindSet, String> {
    if names.is_empty() {
        let all: Vec<Kind> = (0..32_u8)
            .filter_map(|tag| Kind::from_wire(tag, 0))
            .filter(|kind| is_path_kind(*kind))
            .collect();
        return Ok(KindSet::of(&all));
    }
    let mut kinds = Vec::with_capacity(names.len());
    for name in names {
        match Kind::from_name(name) {
            Some(kind) if is_path_kind(kind) => kinds.push(kind),
            _ => return Err(format!("`{name}` is not a kind of file change")),
        }
    }
    Ok(KindSet::of(&kinds))
}

/// The filter set for a glob; `None` for the empty glob.
///
/// # Errors
///
/// The glob error as text.
pub fn glob_set(glob: &str) -> Result<Option<FilterSet>, String> {
    if glob.trim().is_empty() {
        return Ok(None);
    }
    FilterSet::compile(&[FilterRule::exclude(Pattern::Glob(glob.to_owned()))])
        .map(Some)
        .map_err(|error| format!("{error:?}"))
}

/// A problem with one rule: the key inside the rule and what is wrong.
pub type Problem = (String, String);

/// Checks one rule against the ids of the watches in the file.
#[must_use]
pub fn check(rule: &Rule, watch_ids: &[&str]) -> Vec<Problem> {
    let mut out = Vec::new();
    let name = rule.name.trim();
    if name.is_empty() {
        out.push((String::from("name"), String::from("must not be empty")));
    } else if rule.name.chars().count() > MAX_NAME_CHARS {
        out.push((
            String::from("name"),
            format!("is longer than {MAX_NAME_CHARS} characters"),
        ));
    }
    for (i, id) in rule.watches.iter().enumerate() {
        if !watch_ids.contains(&id.as_str()) {
            out.push((
                format!("watches[{i}]"),
                format!("no watch has the id `{id}`"),
            ));
        }
    }
    if let Err(error) = glob_set(&rule.glob) {
        out.push((String::from("glob"), error));
    }
    if let Err(error) = kind_set(&rule.kinds) {
        out.push((String::from("kinds"), error));
    }
    for (key, value, max) in [
        ("threshold", rule.threshold, MAX_THRESHOLD),
        ("window_s", rule.window_s, MAX_WINDOW_S),
    ] {
        if !(1..=max).contains(&value) {
            out.push((key.to_owned(), format!("must be 1 to {max}")));
        }
    }
    if rule.cooldown_s > MAX_COOLDOWN_S {
        out.push((
            String::from("cooldown_s"),
            format!("must be 0 to {MAX_COOLDOWN_S}"),
        ));
    }
    if !rule.toast && rule.command.is_none() {
        out.push((
            String::from("toast"),
            String::from("a rule with no toast needs a command, or it does nothing"),
        ));
    }
    if let Some(command) = &rule.command {
        out.extend(check_command(command));
    }
    out
}

fn check_command(command: &Command) -> Vec<Problem> {
    let mut out = Vec::new();
    if !command.confirmed {
        out.push((
            String::from("command.confirmed"),
            String::from(
                "a command runs with file names from the watched folders as input; \
                 set `confirmed = true` once you have read the warning",
            ),
        ));
    }
    if command.argv.is_empty() || command.argv.len() > MAX_ARGV {
        out.push((
            String::from("command.argv"),
            format!("needs 1 to {MAX_ARGV} entries: the program, then its arguments"),
        ));
    }
    if let Some(program) = command.argv.first() {
        if subst::is_placeholder(program) {
            out.push((
                String::from("command.argv[0]"),
                String::from("the program cannot come from the event"),
            ));
        } else if !is_absolute(program) {
            out.push((
                String::from("command.argv[0]"),
                String::from("must be the full path of the program"),
            ));
        }
    }
    for (i, arg) in command.argv.iter().enumerate() {
        if arg.contains('\0') || arg.len() > MAX_ARG_BYTES {
            out.push((
                format!("command.argv[{i}]"),
                format!("must be at most {MAX_ARG_BYTES} bytes and have no NUL"),
            ));
        }
    }
    if !(1..=MAX_TIMEOUT_S).contains(&command.timeout_s) {
        out.push((
            String::from("command.timeout_s"),
            format!("must be 1 to {MAX_TIMEOUT_S}"),
        ));
    }
    out
}

/// A drive path (`C:\x`) or a UNC path; relative names are searched in the
/// current directory by `CreateProcess`, which a rule must not rely on.
fn is_absolute(program: &str) -> bool {
    let bytes = program.as_bytes();
    let drive = bytes.len() > 3
        && bytes.first().is_some_and(u8::is_ascii_alphabetic)
        && bytes.get(1) == Some(&b':')
        && matches!(bytes.get(2), Some(b'\\' | b'/'));
    drive || program.starts_with(r"\\")
}

#[cfg(test)]
mod tests;
