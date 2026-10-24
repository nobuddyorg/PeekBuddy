//! Presets (#40): data-driven rule lists, embedded and versioned with the
//! release. Every preset hides in views and never drops an event: a user
//! who wants a drop writes their own `NeverRecord` rule.
//!
//! A preset file is TOML:
//!
//! ```toml
//! version = 1
//! name = "temp-lock-swap"
//! title = "Temporary, lock and swap files"
//!
//! [[rule]]
//! glob = "*.tmp"
//! ```
//!
//! A `[[rule]]` has exactly one of `glob`, `regex`, `ext` (a list) or
//! `flags` (a list of `cloud_churn`, `self_churn`, `temp`, `short_dup`).

use std::fmt;

use serde::Deserialize;

use super::{Pattern, Rule, Scope};
use crate::event::Flags;

/// The preset file format version this build reads.
pub const FORMAT_VERSION: u32 = 1;

const FILES: &[(&str, &str)] = &[
    (
        "temp-lock-swap",
        include_str!("presets/temp-lock-swap.toml"),
    ),
    (
        "thumbs-desktop",
        include_str!("presets/thumbs-desktop.toml"),
    ),
    ("git-internals", include_str!("presets/git-internals.toml")),
    ("build-dirs", include_str!("presets/build-dirs.toml")),
    (
        "browser-caches",
        include_str!("presets/browser-caches.toml"),
    ),
    (
        "search-defender",
        include_str!("presets/search-defender.toml"),
    ),
    ("cloud-churn", include_str!("presets/cloud-churn.toml")),
];

/// Why a preset file does not load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PresetError {
    /// The TOML does not parse or has the wrong shape.
    Parse {
        /// The preset file.
        preset: String,
        /// The parser's message.
        message: String,
    },
    /// The file is for another format version.
    Version {
        /// The preset file.
        preset: String,
        /// The version found.
        found: u32,
    },
    /// A rule has none or more than one of `glob`, `regex`, `ext`, `flags`.
    Shape {
        /// The preset file.
        preset: String,
        /// The rule index.
        rule: usize,
    },
    /// A flag name is not known.
    Flag {
        /// The preset file.
        preset: String,
        /// The unknown name.
        name: String,
    },
}

impl fmt::Display for PresetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse { preset, message } => write!(f, "preset {preset}: {message}"),
            Self::Version { preset, found } => {
                write!(
                    f,
                    "preset {preset}: format version {found}, expected {FORMAT_VERSION}"
                )
            }
            Self::Shape { preset, rule } => {
                write!(
                    f,
                    "preset {preset}: rule {rule} needs exactly one of glob, regex, ext, flags"
                )
            }
            Self::Flag { preset, name } => write!(f, "preset {preset}: unknown flag {name}"),
        }
    }
}

impl std::error::Error for PresetError {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    version: u32,
    name: String,
    title: String,
    #[serde(default, rename = "rule")]
    rules: Vec<RawRule>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRule {
    glob: Option<String>,
    regex: Option<String>,
    ext: Option<Vec<String>>,
    flags: Option<Vec<String>>,
}

/// One loaded preset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preset {
    /// The stable id (the file name without `.toml`).
    pub name: String,
    /// A human title for the settings UI.
    pub title: String,
    /// The patterns, in order.
    pub patterns: Vec<Pattern>,
}

impl Preset {
    /// The preset as rules: exclude, hide in views, in `scope`.
    #[must_use]
    pub fn rules(&self, scope: Scope) -> Vec<Rule> {
        self.patterns
            .iter()
            .map(|pattern| Rule {
                scope,
                ..Rule::exclude(pattern.clone())
            })
            .collect()
    }
}

fn flag(name: &str) -> Option<Flags> {
    match name {
        "cloud_churn" => Some(Flags::CLOUD_CHURN),
        "self_churn" => Some(Flags::SELF_CHURN),
        "temp" => Some(Flags::TEMP),
        "short_dup" => Some(Flags::SHORT_DUP),
        _ => None,
    }
}

/// Parses one preset file.
///
/// # Errors
///
/// A [`PresetError`] for bad TOML, another format version or a bad rule.
pub fn parse(id: &str, text: &str) -> Result<Preset, PresetError> {
    let file: File = toml::from_str(text).map_err(|e| PresetError::Parse {
        preset: id.to_owned(),
        message: e.to_string(),
    })?;
    if file.version != FORMAT_VERSION {
        return Err(PresetError::Version {
            preset: id.to_owned(),
            found: file.version,
        });
    }
    let mut patterns = Vec::with_capacity(file.rules.len());
    for (index, raw) in file.rules.into_iter().enumerate() {
        let shape = || PresetError::Shape {
            preset: id.to_owned(),
            rule: index,
        };
        let pattern = match (raw.glob, raw.regex, raw.ext, raw.flags) {
            (Some(glob), None, None, None) => Pattern::Glob(glob),
            (None, Some(regex), None, None) => Pattern::Regex(regex),
            (None, None, Some(ext), None) => Pattern::Ext(ext),
            (None, None, None, Some(names)) => {
                let mut bits = Flags::NONE;
                for name in &names {
                    let known = flag(name).ok_or_else(|| PresetError::Flag {
                        preset: id.to_owned(),
                        name: name.clone(),
                    })?;
                    bits = bits.union(known);
                }
                Pattern::Flags(bits)
            }
            _ => return Err(shape()),
        };
        patterns.push(pattern);
    }
    Ok(Preset {
        name: file.name,
        title: file.title,
        patterns,
    })
}

/// Every embedded preset, in a fixed order.
///
/// # Errors
///
/// A [`PresetError`] if an embedded file is broken (a build-time mistake that
/// the tests catch).
pub fn all() -> Result<Vec<Preset>, PresetError> {
    FILES.iter().map(|(id, text)| parse(id, text)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::{Action, FilterSet, Polarity};

    #[test]
    fn every_embedded_preset_loads_and_compiles() {
        let presets = all().unwrap();
        assert_eq!(presets.len(), FILES.len());
        let mut rules = Vec::new();
        for preset in &presets {
            assert!(!preset.patterns.is_empty(), "{}", preset.name);
            assert_ne!(preset.title, "");
            rules.extend(preset.rules(Scope::Global));
        }
        FilterSet::compile(&rules).unwrap();
    }

    #[test]
    fn all_presets_together_stay_small() {
        let rules: Vec<Rule> = all()
            .unwrap()
            .iter()
            .flat_map(|p| p.rules(Scope::Global))
            .collect();
        let set = FilterSet::compile(&rules).unwrap();
        let bytes = set.memory_usage();
        assert!(bytes > 0);
        // Recorded in the filter reference; a jump means the glob
        // translation or a preset changed shape.
        assert!(bytes < 512 * 1024, "{bytes} bytes");
    }

    #[test]
    fn names_match_the_file_ids_and_are_unique() {
        let presets = all().unwrap();
        for (preset, (id, _)) in presets.iter().zip(FILES) {
            assert_eq!(preset.name, *id);
        }
        let mut names: Vec<_> = presets.iter().map(|p| p.name.clone()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), presets.len());
    }

    #[test]
    fn no_preset_drops_events() {
        for preset in all().unwrap() {
            for rule in preset.rules(Scope::Global) {
                assert_eq!(rule.action, Action::HideInView, "{}", preset.name);
                assert_eq!(rule.polarity, Polarity::Exclude, "{}", preset.name);
            }
        }
    }

    #[test]
    fn rules_take_the_scope() {
        let presets = all().unwrap();
        let preset = presets.first().unwrap();
        let rules = preset.rules(Scope::Watch(crate::event::RootIdx(3)));
        assert_eq!(rules.len(), preset.patterns.len());
        assert!(
            rules
                .iter()
                .all(|r| r.scope == Scope::Watch(crate::event::RootIdx(3)))
        );
    }

    #[test]
    fn compiled_rule_lists_are_pinned() {
        for preset in all().unwrap() {
            insta::assert_debug_snapshot!(preset.name.clone(), preset.rules(Scope::Global));
        }
    }

    #[test]
    fn the_presets_hide_what_they_promise() {
        use crate::event::tests::sample;
        use crate::event::{Event, Kind};
        use crate::name::WideStr;
        let mut rules = Vec::new();
        for preset in all().unwrap() {
            rules.extend(preset.rules(Scope::Global));
        }
        let set = FilterSet::compile(&rules).unwrap();
        let mut scratch = super::super::Scratch::new();
        let mut visible = |path: &str, is_dir: bool, flags: Flags| {
            let units: Vec<u16> = path.encode_utf16().collect();
            let ev = Event {
                is_dir,
                flags,
                kind: Kind::Modified,
                ..sample()
            };
            set.eval(&ev, WideStr::new(&units), &mut scratch).visible
        };
        for hidden in [
            "~$report.docx",
            "a\\b.tmp",
            "x.swp",
            ".~lock.sheet.ods#",
            "d\\Thumbs.db",
            "desktop.ini",
            ".git\\objects\\ab\\cdef",
            "app\\node_modules\\x\\index.js",
            "target\\debug\\a.o",
            "Profile\\Cache\\f_0001",
            "Windows.edb",
            "edb00012.jrs",
        ] {
            assert!(!visible(hidden, false, Flags::NONE), "{hidden}");
        }
        assert!(!visible("hydrated.docx", false, Flags::CLOUD_CHURN));
        for shown in [
            "report.docx",
            "src\\main.rs",
            ".gitignore",
            "tmp",
            "notes.txt.bak",
        ] {
            assert!(visible(shown, false, Flags::NONE), "{shown}");
        }
        // A file named like a directory preset is not hidden by it.
        assert!(visible("target", false, Flags::NONE));
        assert!(!visible("target", true, Flags::NONE));
    }

    #[test]
    fn parse_errors() {
        let bad = |text: &str| parse("t", text).unwrap_err();
        assert!(matches!(bad("not toml ["), PresetError::Parse { .. }));
        assert!(matches!(
            bad("version = 2\nname = \"a\"\ntitle = \"b\"\n"),
            PresetError::Version { found: 2, .. }
        ));
        let head = "version = 1\nname = \"a\"\ntitle = \"b\"\n";
        assert!(matches!(
            bad(&format!("{head}[[rule]]\n")),
            PresetError::Shape { rule: 0, .. }
        ));
        assert!(matches!(
            bad(&format!("{head}[[rule]]\nglob = \"a\"\nregex = \"b\"\n")),
            PresetError::Shape { .. }
        ));
        assert!(matches!(
            bad(&format!("{head}[[rule]]\nflags = [\"nope\"]\n")),
            PresetError::Flag { .. }
        ));
        assert!(matches!(
            bad(&format!("{head}[[rule]]\nglob = \"a\"\nbogus = 1\n")),
            PresetError::Parse { .. }
        ));
        let ok = parse(
            "t",
            &format!("{head}[[rule]]\next = [\"a\"]\n[[rule]]\nregex = \"x\"\n"),
        )
        .unwrap();
        assert_eq!(ok.patterns.len(), 2);
        assert!(bad("x [").to_string().starts_with("preset t: "));
        let shown = |e: PresetError| e.to_string();
        let p = || "p".to_owned();
        assert_eq!(
            shown(PresetError::Version {
                preset: p(),
                found: 2
            }),
            "preset p: format version 2, expected 1"
        );
        assert_eq!(
            shown(PresetError::Shape {
                preset: p(),
                rule: 3
            }),
            "preset p: rule 3 needs exactly one of glob, regex, ext, flags"
        );
        assert_eq!(
            shown(PresetError::Flag {
                preset: p(),
                name: "x".into()
            }),
            "preset p: unknown flag x"
        );
        assert_eq!(
            shown(PresetError::Parse {
                preset: p(),
                message: "m".into()
            }),
            "preset p: m"
        );
    }
}
