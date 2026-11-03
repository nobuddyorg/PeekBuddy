//! Settings: the versioned `settings.toml` (#62).
//!
//! The schema, parsing, validation and migrations are pure and live here so
//! they build on Linux, run under Miri and can be fuzzed. File I/O lives in
//! `peekbuddy-win` and the engine. Unknown keys are rejected for the current
//! version with a "did you mean" hint, a newer file is loaded leniently and
//! never written, and a bad file never panics.

pub mod edit;
mod load;
mod migrate;
mod validate;

use serde::{Deserialize, Serialize};

pub use load::{Loaded, Problem, load, load_with};
pub use migrate::{MigrationError, migrate};
pub use validate::{Context, normalize_root};

/// The version this build writes.
pub const CURRENT_VERSION: u32 = 1;

/// Largest value the TOML integer type can hold; every number in the file
/// stays below it.
pub const MAX_INT: u64 = i64::MAX as u64;

/// All settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// File format version.
    pub version: u32,
    /// Autostart intent, theme, first-run flag.
    pub general: General,
    /// Retention caps (#54).
    pub store: Store,
    /// Viewer defaults.
    pub view: View,
    /// What the viewer remembers between runs (#65).
    #[serde(skip_serializing_if = "Viewer::is_default")]
    pub viewer: Viewer,
    /// Global filters (#40).
    pub filters: Filters,
    /// The watched roots.
    #[serde(rename = "watch", skip_serializing_if = "Vec::is_empty")]
    pub watches: Vec<Watch>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: CURRENT_VERSION,
            general: General::default(),
            store: Store::default(),
            view: View::default(),
            viewer: Viewer::default(),
            filters: Filters::default(),
            watches: Vec::new(),
        }
    }
}

/// Colour theme of the viewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    /// Follow Windows.
    #[default]
    System,
    /// Always light.
    Light,
    /// Always dark.
    Dark,
}

/// `[general]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct General {
    /// Start with Windows (the intent; the Run key is #63's job).
    pub autostart: bool,
    /// Viewer theme.
    pub theme: Theme,
    /// Nothing has been configured yet; the tray and the viewer onboard.
    pub first_run: bool,
}

impl Default for General {
    fn default() -> Self {
        Self {
            autostart: false,
            theme: Theme::System,
            first_run: true,
        }
    }
}

/// `[store]`: retention caps. Absent means no cap of that kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Store {
    /// Delete segments older than this many days.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_age_days: Option<u32>,
    /// Delete the oldest segments above this total size, in MB.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_total_mb: Option<u64>,
    /// Delete the oldest segments above this many rows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_rows: Option<u64>,
}

/// Range of `view.coalesce_ms`.
pub const COALESCE_MS: std::ops::RangeInclusive<u32> = 50..=500;

/// `[view]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct View {
    /// Window in which rapid events of one file are coalesced, in ms.
    pub coalesce_ms: u32,
    /// Visible columns, in order.
    pub columns: Vec<String>,
}

impl Default for View {
    fn default() -> Self {
        Self {
            coalesce_ms: 150,
            columns: ["time", "kind", "path"].map(String::from).to_vec(),
        }
    }
}

/// Columns the viewer knows. `process` and `volume` are reserved for later
/// versions (#80); the viewer ignores them.
pub const COLUMNS: &[&str] = &[
    "time", "kind", "path", "size", "process", "volume", "old_path", "source", "raw",
];

/// Smallest and largest window side the viewer restores, in pixels.
pub const WINDOW_SIDE: std::ops::RangeInclusive<u32> = 200..=32_768;

/// Longest filter text the viewer keeps, in bytes.
pub const MAX_FILTER_TEXT: usize = 4096;

/// `[viewer]`: what the viewer remembers between runs. The viewer writes it
/// when its window closes (through the engine, which owns the file); a
/// missing table means the defaults. Window pixels are virtual-screen
/// pixels; the viewer clamps them to a visible monitor on restore.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Viewer {
    /// Left edge of the window.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<i32>,
    /// Top edge of the window.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<i32>,
    /// Width of the window (not maximized).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    /// Height of the window (not maximized).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// The window was maximized.
    #[serde(skip_serializing_if = "is_false")]
    pub maximized: bool,
    /// The filter, in its canonical text form (#67).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub filter: String,
    /// Show one row per stored record instead of derived groups (#66).
    #[serde(skip_serializing_if = "is_false")]
    pub raw: bool,
    /// The details pane is closed (#69). Open is the default.
    #[serde(skip_serializing_if = "is_false")]
    pub details_hidden: bool,
}

#[expect(clippy::trivially_copy_pass_by_ref, reason = "serde's signature")]
const fn is_false(value: &bool) -> bool {
    !*value
}

impl Viewer {
    /// Whether nothing is remembered, so the table is not written.
    #[must_use]
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// `[filters]`: rules for every watch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Filters {
    /// Gitignore-style globs to hide.
    pub exclude_globs: Vec<String>,
    /// Regexes to hide, searched anywhere in the path.
    pub exclude_regex: Vec<String>,
    /// Embedded presets to apply (see `filters::presets`).
    pub presets: Vec<String>,
}

/// One `[[watch]]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Watch {
    /// Stable id; never reused.
    pub id: String,
    /// Absolute path of the root (stored as written, normalised on load).
    pub path: String,
    /// Watch the whole subtree.
    pub recursive: bool,
    /// Off means paused (#63).
    pub enabled: bool,
    /// Globs to hide in this watch only.
    pub exclude_globs: Vec<String>,
    /// Presets for this watch only.
    pub presets: Vec<String>,
    /// Change-notification buffer size in KiB, overriding the default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub buffer_kib: Option<u32>,
    /// Also watch where a link in the tree points (F18, opt-in).
    pub follow_link_target: bool,
    /// For a watch the engine added for a link target: the id of the watch
    /// whose link it is. Never read from or written to the file.
    #[serde(skip)]
    pub derived_from: Option<String>,
}

impl Default for Watch {
    fn default() -> Self {
        Self {
            id: String::new(),
            path: String::new(),
            recursive: true,
            enabled: true,
            exclude_globs: Vec::new(),
            presets: Vec::new(),
            buffer_kib: None,
            follow_link_target: false,
            derived_from: None,
        }
    }
}

/// Keys of each table, for the unknown-key check and "did you mean".
pub(crate) const TOP_KEYS: &[&str] = &[
    "version", "general", "store", "view", "viewer", "filters", "watch",
];
pub(crate) const GENERAL_KEYS: &[&str] = &["autostart", "theme", "first_run"];
pub(crate) const STORE_KEYS: &[&str] = &["max_age_days", "max_total_mb", "max_rows"];
pub(crate) const VIEW_KEYS: &[&str] = &["coalesce_ms", "columns"];
pub(crate) const VIEWER_KEYS: &[&str] = &[
    "x",
    "y",
    "width",
    "height",
    "maximized",
    "filter",
    "raw",
    "details_hidden",
];
pub(crate) const FILTER_KEYS: &[&str] = &["exclude_globs", "exclude_regex", "presets"];
pub(crate) const WATCH_KEYS: &[&str] = &[
    "id",
    "path",
    "recursive",
    "enabled",
    "exclude_globs",
    "presets",
    "buffer_kib",
    "follow_link_target",
];

/// Writes `settings` as TOML. The output parses back to the same value.
///
/// # Errors
///
/// The serializer's message; only possible for values outside the TOML
/// integer range, which validation refuses.
pub fn to_toml(settings: &Settings) -> Result<String, String> {
    if settings
        .watches
        .iter()
        .any(|watch| watch.derived_from.is_some())
    {
        let mut own = settings.clone();
        own.watches.retain(|watch| watch.derived_from.is_none());
        return toml::to_string_pretty(&own).map_err(|error| error.to_string());
    }
    toml::to_string_pretty(settings).map_err(|error| error.to_string())
}

/// The file written on first run: nothing watched, first-run flag set.
#[must_use]
pub fn default_toml() -> String {
    let mut settings = Settings::default();
    // The caps of a fresh install are written out, so they are visible and
    // editable; a file without them has no caps.
    settings.store.max_age_days = Some(DEFAULT_MAX_AGE_DAYS);
    settings.store.max_total_mb = Some(DEFAULT_MAX_TOTAL_MB);
    to_toml(&settings).unwrap_or_default()
}

/// The retention age written to a new settings file: 30 days.
pub const DEFAULT_MAX_AGE_DAYS: u32 = 30;
/// The retention size written to a new settings file: 2 GB.
pub const DEFAULT_MAX_TOTAL_MB: u64 = 2048;

#[cfg(test)]
mod tests {
    use super::{Settings, Watch, default_toml, to_toml};

    #[test]
    fn the_default_file_is_pinned() {
        insta::assert_snapshot!(default_toml());
    }

    #[test]
    fn the_default_round_trips() {
        let text = to_toml(&Settings::default()).unwrap();
        assert_eq!(
            toml::from_str::<Settings>(&text).unwrap(),
            Settings::default()
        );
    }

    #[test]
    fn a_derived_watch_is_never_written() {
        let mut settings = Settings::default();
        settings.watches.push(Watch {
            id: "w0".into(),
            path: "C:/tree".into(),
            follow_link_target: true,
            ..Watch::default()
        });
        settings.watches.push(Watch {
            id: "w0.link1".into(),
            path: "C:/elsewhere".into(),
            derived_from: Some("w0".into()),
            ..Watch::default()
        });
        let text = to_toml(&settings).unwrap();
        assert!(!text.contains("elsewhere"), "{text}");
        let back = toml::from_str::<Settings>(&text).unwrap();
        assert_eq!(back.watches.len(), 1);
        assert!(
            back.watches
                .iter()
                .all(|watch| watch.derived_from.is_none())
        );
        // The settings in memory are untouched.
        assert_eq!(settings.watches.len(), 2);
    }
}
