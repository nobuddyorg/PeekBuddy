//! Semantic validation of parsed settings.

use std::collections::HashSet;

use crate::filter::glob;
use crate::filter::{FilterSet, Pattern, Rule, presets};
use crate::name::CaseMode;

use super::{COALESCE_MS, COLUMNS, CURRENT_VERSION, MAX_INT, Settings, Watch};

/// What validation needs to know about the machine.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Context {
    /// The data directory. A watch root may not be inside it (ADR 0008).
    pub data_dir: String,
    /// The export directory, refused as a watch root too (F16).
    pub export_dir: Option<String>,
}

/// A validation failure before it is given a position in the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Issue {
    /// Dotted key path, with `watch[i]` for array items.
    pub key_path: String,
    /// What is wrong.
    pub message: String,
    /// Text of the offending value, used to find its line.
    pub value: Option<String>,
    /// A suggested replacement.
    pub suggestion: Option<String>,
}

impl Issue {
    fn new(key_path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            key_path: key_path.into(),
            message: message.into(),
            value: None,
            suggestion: None,
        }
    }

    fn at(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self
    }
}

const SEP: char = '\\';
const VERBATIM: &str = r"\\?\";
const VERBATIM_UNC: &str = r"\\?\UNC\";

/// Normalises an absolute Windows path to the `\\?\` form used internally
/// (F-paths): drive paths become `\\?\C:\x`, UNC paths `\\?\UNC\server\share`.
/// Forward slashes are accepted, repeated separators collapse and a trailing
/// separator is dropped (except on a drive root).
///
/// # Errors
///
/// A message for a relative, device or malformed path.
pub fn normalize_root(path: &str) -> Result<String, String> {
    if path.is_empty() {
        return Err(String::from("the path is empty"));
    }
    if path.contains('\0') {
        return Err(String::from("the path contains a NUL character"));
    }
    let slashes = path.replace('/', "\\");
    let (prefix, rest): (&str, &str) = if let Some(rest) = strip_prefix_ci(&slashes, VERBATIM_UNC) {
        (VERBATIM_UNC, rest)
    } else if let Some(rest) = slashes.strip_prefix(r"\\.\") {
        let _ = rest;
        return Err(String::from("device paths are not supported"));
    } else if let Some(rest) = slashes.strip_prefix(VERBATIM) {
        (VERBATIM, rest)
    } else if let Some(rest) = slashes.strip_prefix(r"\\") {
        (VERBATIM_UNC, rest)
    } else if is_drive_path(&slashes) {
        (VERBATIM, slashes.as_str())
    } else {
        return Err(String::from("the path must be absolute"));
    };

    let mut parts: Vec<&str> = Vec::new();
    for part in rest.split(SEP) {
        match part {
            "" => {}
            "." | ".." => return Err(String::from("`.` and `..` are not allowed in a path")),
            other => parts.push(other),
        }
    }
    if prefix == VERBATIM {
        let drive = parts.first().copied().unwrap_or_default();
        if !is_drive(drive) {
            return Err(String::from("the path must start with a drive letter"));
        }
        if parts.len() == 1 {
            return Ok(format!("{VERBATIM}{drive}{SEP}"));
        }
    } else if parts.len() < 2 {
        return Err(String::from("a UNC path needs a server and a share"));
    }
    Ok(format!("{prefix}{}", parts.join("\\")))
}

fn strip_prefix_ci<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| text.get(prefix.len()..))
        .flatten()
}

fn is_drive(part: &str) -> bool {
    let bytes = part.as_bytes();
    bytes.len() == 2
        && bytes.first().is_some_and(u8::is_ascii_alphabetic)
        && bytes.get(1) == Some(&b':')
}

fn is_drive_path(path: &str) -> bool {
    path.get(..2).is_some_and(is_drive)
        && path
            .get(2..)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(SEP))
}

/// Whether `path` is `root` or below it, ignoring case. Both must be
/// normalised.
fn is_inside(path: &str, root: &str) -> bool {
    let path = path.to_lowercase();
    let root = root.trim_end_matches(SEP).to_lowercase();
    path == root
        || path
            .strip_prefix(&root)
            .is_some_and(|rest| rest.starts_with(SEP))
}

fn is_unc(normalised: &str) -> bool {
    normalised.starts_with(VERBATIM_UNC)
}

fn valid_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

fn glob_issue(key: &str, pattern: &str) -> Option<Issue> {
    glob::parse(pattern, CaseMode::Insensitive)
        .err()
        .map(|error| Issue::new(key, format!("not a valid glob: {error}")).at(pattern))
}

fn regex_issue(key: &str, pattern: &str) -> Option<Issue> {
    let rule = Rule::exclude(Pattern::Regex(pattern.to_owned()));
    FilterSet::compile(&[rule]).err().map(|error| {
        let why = match error {
            crate::filter::FilterError::Regex { message, .. } => message,
            other => other.to_string(),
        };
        Issue::new(key, format!("not a valid regex: {why}")).at(pattern)
    })
}

fn preset_issues(key: &str, names: &[String], known: &[String], out: &mut Vec<Issue>) {
    for (index, name) in names.iter().enumerate() {
        if !known.contains(name) {
            let mut issue = Issue::new(
                format!("{key}[{index}]"),
                format!("unknown preset `{name}`"),
            )
            .at(name.clone());
            issue.suggestion = super::load::closest(name, known.iter().map(String::as_str));
            out.push(issue);
        }
    }
}

fn watch_issues(
    index: usize,
    watch: &mut Watch,
    ctx: &Context,
    known_presets: &[String],
    out: &mut Vec<Issue>,
) {
    let key = |name: &str| format!("watch[{index}].{name}");
    if !valid_id(&watch.id) {
        out.push(
            Issue::new(
                key("id"),
                "an id is 1 to 64 characters of a-z, 0-9, `-` and `_`",
            )
            .at(watch.id.clone()),
        );
    }
    let normalised = match normalize_root(&watch.path) {
        Ok(path) => Some(path),
        Err(message) => {
            out.push(Issue::new(key("path"), message).at(watch.path.clone()));
            None
        }
    };
    if let Some(path) = &normalised {
        for (what, refused) in [
            ("data directory", Some(&ctx.data_dir)),
            ("export directory", ctx.export_dir.as_ref()),
        ] {
            let refused = refused.and_then(|dir| normalize_root(dir).ok());
            if refused.as_deref().is_some_and(|dir| is_inside(path, dir)) {
                out.push(
                    Issue::new(key("path"), format!("a watch cannot be inside the {what}"))
                        .at(watch.path.clone()),
                );
            }
        }
        if let Some(kib) = watch.buffer_kib {
            let limit = if is_unc(path) { 64 } else { 1024 };
            if !(4..=limit).contains(&kib) {
                out.push(
                    Issue::new(
                        key("buffer_kib"),
                        format!(
                            "the buffer must be 4 to {limit} KiB for {}",
                            if is_unc(path) {
                                "a network path"
                            } else {
                                "a local path"
                            }
                        ),
                    )
                    .at(kib.to_string()),
                );
            }
        }
    } else if watch
        .buffer_kib
        .is_some_and(|kib| !(4..=1024).contains(&kib))
    {
        out.push(Issue::new(
            key("buffer_kib"),
            "the buffer must be 4 to 1024 KiB",
        ));
    }
    for (i, pattern) in watch.exclude_globs.iter().enumerate() {
        out.extend(glob_issue(
            &format!("watch[{index}].exclude_globs[{i}]"),
            pattern,
        ));
    }
    preset_issues(&key("presets"), &watch.presets, known_presets, out);
    if let Some(path) = normalised {
        watch.path = path;
    }
}

/// Retention caps and viewer defaults.
fn store_and_view_issues(settings: &Settings) -> Vec<Issue> {
    let mut out = Vec::new();
    let store = &settings.store;
    if store
        .max_age_days
        .is_some_and(|days| !(1..=36_500).contains(&days))
    {
        out.push(Issue::new("store.max_age_days", "must be 1 to 36500"));
    }
    if store
        .max_total_mb
        .is_some_and(|mb| !(1..=MAX_INT).contains(&mb))
    {
        out.push(Issue::new("store.max_total_mb", "must be at least 1"));
    }
    if store
        .max_rows
        .is_some_and(|rows| !(1..=MAX_INT).contains(&rows))
    {
        out.push(Issue::new("store.max_rows", "must be at least 1"));
    }
    if !COALESCE_MS.contains(&settings.view.coalesce_ms) {
        out.push(
            Issue::new(
                "view.coalesce_ms",
                format!(
                    "must be {} to {} ms",
                    COALESCE_MS.start(),
                    COALESCE_MS.end()
                ),
            )
            .at(settings.view.coalesce_ms.to_string()),
        );
    }
    let mut seen = HashSet::new();
    if settings.view.columns.is_empty() {
        out.push(Issue::new(
            "view.columns",
            "at least one column is required",
        ));
    }
    for (i, column) in settings.view.columns.iter().enumerate() {
        let key = format!("view.columns[{i}]");
        if !COLUMNS.contains(&column.as_str()) {
            let mut issue =
                Issue::new(key, format!("unknown column `{column}`")).at(column.clone());
            issue.suggestion = super::load::closest(column, COLUMNS.iter().copied());
            out.push(issue);
        } else if !seen.insert(column.as_str()) {
            out.push(
                Issue::new(key, format!("column `{column}` is listed twice")).at(column.clone()),
            );
        }
    }
    out
}

/// Validates `settings` and normalises its watch paths in place. Returns
/// every problem found, not just the first.
pub(crate) fn validate(settings: &mut Settings, ctx: &Context) -> Vec<Issue> {
    let mut out = Vec::new();
    if settings.version != CURRENT_VERSION {
        out.push(Issue::new(
            "version",
            format!("this build reads version {CURRENT_VERSION}"),
        ));
    }
    out.extend(store_and_view_issues(settings));
    let known_presets: Vec<String> = presets::all()
        .map(|all| all.into_iter().map(|preset| preset.name).collect())
        .unwrap_or_default();
    for (i, pattern) in settings.filters.exclude_globs.iter().enumerate() {
        out.extend(glob_issue(&format!("filters.exclude_globs[{i}]"), pattern));
    }
    for (i, pattern) in settings.filters.exclude_regex.iter().enumerate() {
        out.extend(regex_issue(&format!("filters.exclude_regex[{i}]"), pattern));
    }
    preset_issues(
        "filters.presets",
        &settings.filters.presets,
        &known_presets,
        &mut out,
    );

    let mut ids: HashSet<String> = HashSet::new();
    let mut roots: Vec<(String, String)> = Vec::new();
    for (index, watch) in settings.watches.iter_mut().enumerate() {
        watch_issues(index, watch, ctx, &known_presets, &mut out);
        if !ids.insert(watch.id.clone()) {
            out.push(
                Issue::new(
                    format!("watch[{index}].id"),
                    format!("the id `{}` is used twice", watch.id),
                )
                .at(watch.id.clone()),
            );
        }
        let lowered = watch.path.to_lowercase();
        if let Some((other, _)) = roots.iter().find(|(_, path)| *path == lowered) {
            out.push(
                Issue::new(
                    format!("watch[{index}].path"),
                    format!("the same root as watch `{other}`"),
                )
                .at(watch.path.clone()),
            );
        } else {
            roots.push((watch.id.clone(), lowered));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{Context, is_inside, normalize_root};

    #[test]
    fn paths_are_normalised_to_the_verbatim_form() {
        for (input, expected) in [
            (r"C:\Users\Me", r"\\?\C:\Users\Me"),
            ("c:/Users//Me/", r"\\?\c:\Users\Me"),
            (r"C:\", r"\\?\C:\"),
            (r"C:", r"\\?\C:\"),
            (r"\\?\D:\data\", r"\\?\D:\data"),
            (r"\\server\share\dir", r"\\?\UNC\server\share\dir"),
            (r"\\?\UNC\server\share", r"\\?\UNC\server\share"),
        ] {
            assert_eq!(normalize_root(input).unwrap(), expected, "{input}");
        }
    }

    #[test]
    fn bad_paths_are_refused_with_a_reason() {
        for (input, why) in [
            ("", "empty"),
            ("relative\\dir", "absolute"),
            (r"\Users\Me", "absolute"),
            (r"\\.\PhysicalDrive0", "device"),
            (r"C:\a\..\b", "`..`"),
            (r"\\server", "share"),
            ("C:\\a\0b", "NUL"),
        ] {
            let message = normalize_root(input).unwrap_err();
            assert!(message.contains(why), "{input}: {message}");
        }
    }

    #[test]
    fn inside_is_whole_components_and_ignores_case() {
        let data = r"\\?\C:\Users\Me\AppData\Local\nobuddy\PeekBuddy";
        assert!(is_inside(data, data));
        assert!(is_inside(
            &format!(r"{data}\segments"),
            &data.to_uppercase()
        ));
        assert!(!is_inside(&format!("{data}2"), data));
        assert!(!is_inside(r"\\?\C:\Users\Me", data));
    }

    #[test]
    fn the_context_defaults_to_nothing_refused() {
        assert_eq!(Context::default().data_dir, "");
    }
}
