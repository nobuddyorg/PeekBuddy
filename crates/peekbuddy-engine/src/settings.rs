//! Loading `settings.toml` at startup (#62).
//!
//! The rules: a missing file is created with the defaults; a corrupt file
//! never stops the engine and is never overwritten; a file from a newer build
//! is read leniently and never written; an older file is migrated after a
//! `settings.v<N>.bak` copy. Nothing watches the file (F6): hand edits take
//! effect on the next start.

use std::path::Path;

use peekbuddy_core::settings::{self, Context, Loaded, Settings};

use crate::datadir::DataDir;

/// How the file was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// There was no file; the defaults were written (first run).
    Created,
    /// The file is valid; it was migrated from the given version if older.
    Loaded {
        /// The version before migration.
        migrated_from: Option<u32>,
    },
    /// The file is from a newer build: read leniently, never written.
    TooNew {
        /// The file's version.
        version: u32,
    },
    /// The file is unusable. No watches are applied, the file is untouched.
    Invalid {
        /// One line per problem, with positions where known.
        problems: Vec<String>,
    },
}

impl State {
    /// One line for `--diagnostics` and the tray's problem state.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Created => String::from("created with the defaults (first run)"),
            Self::Loaded {
                migrated_from: None,
            } => String::from("ok"),
            Self::Loaded {
                migrated_from: Some(from),
            } => {
                format!("ok, migrated from version {from} (backup kept)")
            }
            Self::TooNew { version } => format!(
                "version {version} is newer than this build reads: loaded leniently, not written"
            ),
            Self::Invalid { problems } => format!(
                "invalid, {} problem(s), no watches applied, file left as is: {}",
                problems.len(),
                problems.first().map_or("", String::as_str)
            ),
        }
    }

    /// Whether the user has to look at the file.
    #[must_use]
    pub const fn is_problem(&self) -> bool {
        matches!(self, Self::Invalid { .. } | Self::TooNew { .. })
    }
}

/// The settings in force and how they were obtained.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedSettings {
    /// What the engine uses; the defaults when the file is unusable.
    pub settings: Settings,
    /// How the file was found.
    pub state: State,
}

fn context(data: &DataDir) -> Context {
    Context {
        data_dir: data.root.to_string_lossy().into_owned(),
        export_dir: None,
    }
}

/// Keeps `settings.v<from>.bak` (once) and writes the migrated file.
fn persist_migration(
    path: &Path,
    from: u32,
    original: &str,
    settings: &Settings,
) -> std::io::Result<()> {
    let backup = path.with_file_name(format!("settings.v{from}.bak"));
    if !backup.exists() {
        peekbuddy_win::atomic::write_atomic(&backup, original.as_bytes())?;
    }
    let text = settings::to_toml(settings).map_err(std::io::Error::other)?;
    peekbuddy_win::atomic::write_atomic(path, text.as_bytes())
}

fn invalid(problem: String) -> LoadedSettings {
    LoadedSettings {
        settings: Settings::default(),
        state: State::Invalid {
            problems: vec![problem],
        },
    }
}

/// Loads the settings of `data`, creating the default file on first run.
#[must_use]
pub fn load_or_create(data: &DataDir) -> LoadedSettings {
    let path = &data.settings;
    let text = match std::fs::read(path) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(_) => return invalid(String::from("the file is not valid UTF-8")),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let defaults = Settings::default();
            let written = settings::to_toml(&defaults)
                .map_err(std::io::Error::other)
                .and_then(|text| peekbuddy_win::atomic::write_atomic(path, text.as_bytes()));
            return match written {
                Ok(()) => LoadedSettings {
                    settings: defaults,
                    state: State::Created,
                },
                Err(error) => invalid(format!("the default file could not be written: {error}")),
            };
        }
        Err(error) => return invalid(format!("the file could not be read: {error}")),
    };
    match settings::load(&text, &context(data)) {
        Loaded::Ok {
            settings,
            migrated_from,
        } => {
            if let Some(from) = migrated_from
                && let Err(error) = persist_migration(path, from, &text, &settings)
            {
                return invalid(format!("the migrated file could not be written: {error}"));
            }
            LoadedSettings {
                settings: *settings,
                state: State::Loaded { migrated_from },
            }
        }
        Loaded::TooNew { version, settings } => LoadedSettings {
            settings: *settings,
            state: State::TooNew { version },
        },
        Loaded::Invalid(problems) => LoadedSettings {
            settings: Settings::default(),
            state: State::Invalid {
                problems: problems.iter().map(ToString::to_string).collect(),
            },
        },
    }
}

/// Reads the settings file without creating, migrating or writing anything,
/// for `--diagnostics`. Problem lines are listed in full.
#[must_use]
pub fn inspect(path: &Path, data_dir: &str) -> String {
    let Ok(bytes) = std::fs::read(path) else {
        return String::from("no settings file yet (created on the first engine start)");
    };
    let Ok(text) = String::from_utf8(bytes) else {
        return String::from("invalid: the file is not valid UTF-8");
    };
    let ctx = Context {
        data_dir: data_dir.to_owned(),
        export_dir: None,
    };
    match settings::load(&text, &ctx) {
        Loaded::Ok {
            settings,
            migrated_from,
        } => format!(
            "{}, {} watch(es){}",
            "ok",
            settings.watches.len(),
            migrated_from.map_or_else(String::new, |from| format!(
                ", will migrate from version {from}"
            ))
        ),
        Loaded::TooNew { version, .. } => State::TooNew { version }.describe(),
        Loaded::Invalid(problems) => {
            let lines: Vec<String> = problems.iter().map(ToString::to_string).collect();
            format!("invalid, {} problem(s): {}", lines.len(), lines.join("; "))
        }
    }
}

#[cfg(test)]
mod tests {
    use peekbuddy_core::settings::Settings;

    use super::{State, inspect, load_or_create, persist_migration};
    use crate::datadir::DataDir;
    use crate::portable::Decision;

    fn data(name: &str) -> DataDir {
        let root = std::env::temp_dir().join(format!("pb-settings-{name}-{}", std::process::id()));
        DataDir::open(&root, Decision::Installed(None)).unwrap()
    }

    fn clean(data: &DataDir) {
        std::fs::remove_dir_all(&data.root).unwrap();
    }

    #[test]
    fn a_first_run_writes_the_defaults_and_the_next_start_reads_them() {
        let data = data("first");
        let first = load_or_create(&data);
        assert_eq!(first.state, State::Created);
        assert_eq!(first.settings, Settings::default());
        assert!(data.settings.is_file());
        let second = load_or_create(&data);
        assert_eq!(
            second.state,
            State::Loaded {
                migrated_from: None
            }
        );
        assert!(second.settings.general.first_run);
        clean(&data);
    }

    #[test]
    fn a_corrupt_file_does_not_stop_the_engine_and_is_never_overwritten() {
        let data = data("corrupt");
        let bad = b"version = 1\n[general\nthis is not toml";
        std::fs::write(&data.settings, bad).unwrap();
        let loaded = load_or_create(&data);
        assert!(loaded.state.is_problem());
        assert!(matches!(loaded.state, State::Invalid { ref problems } if !problems.is_empty()));
        assert_eq!(loaded.settings.watches, []);
        assert_eq!(std::fs::read(&data.settings).unwrap(), bad);
        clean(&data);
    }

    #[test]
    fn invalid_utf8_is_a_problem_not_a_panic() {
        let data = data("utf8");
        std::fs::write(&data.settings, [0xff, 0xfe, 0x00]).unwrap();
        assert!(load_or_create(&data).state.is_problem());
        clean(&data);
    }

    #[test]
    fn a_newer_file_is_read_leniently_and_left_alone() {
        let data = data("newer");
        let text = b"version = 9\n[general]\ntheme = \"dark\"\nfuture = 1\n";
        std::fs::write(&data.settings, text).unwrap();
        let loaded = load_or_create(&data);
        assert_eq!(loaded.state, State::TooNew { version: 9 });
        assert_eq!(std::fs::read(&data.settings).unwrap(), text);
        clean(&data);
    }

    #[test]
    fn a_migration_keeps_one_backup_of_the_original() {
        let data = data("migrate");
        let original = "version = 0\n";
        std::fs::write(&data.settings, original).unwrap();
        persist_migration(&data.settings, 0, original, &Settings::default()).unwrap();
        let backup = data.settings.with_file_name("settings.v0.bak");
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), original);
        assert!(
            std::fs::read_to_string(&data.settings)
                .unwrap()
                .contains("version = 1")
        );
        // A second migration attempt never replaces the first backup.
        persist_migration(&data.settings, 0, "something else", &Settings::default()).unwrap();
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), original);
        clean(&data);
    }

    #[test]
    fn inspect_reports_without_writing() {
        let data = data("inspect");
        assert!(inspect(&data.settings, "").contains("no settings file yet"));
        assert!(!data.settings.exists());
        std::fs::write(
            &data.settings,
            "version = 1
[general]
autostrat = true
",
        )
        .unwrap();
        let report = inspect(&data.settings, "");
        assert!(report.contains("invalid, 1 problem(s)"), "{report}");
        assert!(report.contains("did you mean `autostart`"), "{report}");
        std::fs::write(
            &data.settings,
            "version = 1
",
        )
        .unwrap();
        assert!(inspect(&data.settings, "").starts_with("ok, 0 watch(es)"));
        clean(&data);
    }

    #[test]
    fn states_describe_themselves() {
        assert!(State::Created.describe().contains("first run"));
        assert!(
            State::TooNew { version: 4 }
                .describe()
                .contains("not written")
        );
        let invalid = State::Invalid {
            problems: vec![String::from("3:1: oops")],
        };
        assert!(invalid.describe().contains("3:1: oops"));
    }
}
