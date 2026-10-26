//! Migrations: pure functions `vN -> vN+1` over the TOML table.

use std::fmt;

use super::CURRENT_VERSION;

/// One step from version `from` to `from + 1`.
pub type Step = fn(toml::Table) -> Result<toml::Table, MigrationError>;

/// Why a migration failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationError {
    /// The version the step started from.
    pub from: u32,
    /// What went wrong.
    pub message: String,
}

impl fmt::Display for MigrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "migration from version {}: {}", self.from, self.message)
    }
}

impl std::error::Error for MigrationError {}

/// The steps of this build: `STEPS[n]` migrates version `n + 1` to `n + 2`.
/// Empty while only version 1 exists.
const STEPS: &[Step] = &[];

/// Migrates `table` from version `from` to [`CURRENT_VERSION`] with this
/// build's steps.
///
/// # Errors
///
/// The first failing step.
pub fn migrate(table: toml::Table, from: u32) -> Result<toml::Table, MigrationError> {
    migrate_with(table, from, CURRENT_VERSION, STEPS)
}

/// Like [`migrate`], with an explicit target and step list (tests use it to
/// exercise the harness before a second version exists).
pub(crate) fn migrate_with(
    mut table: toml::Table,
    from: u32,
    to: u32,
    steps: &[Step],
) -> Result<toml::Table, MigrationError> {
    let mut version = from;
    while version < to {
        let step = usize::try_from(version.saturating_sub(1))
            .ok()
            .and_then(|index| steps.get(index))
            .ok_or_else(|| MigrationError {
                from: version,
                message: String::from("no migration step for this version"),
            })?;
        table = step(table)?;
        version += 1;
        table.insert(
            String::from("version"),
            toml::Value::Integer(i64::from(version)),
        );
    }
    Ok(table)
}

#[cfg(test)]
#[expect(
    clippy::indexing_slicing,
    clippy::unnecessary_wraps,
    reason = "tests index and implement the fallible step signature"
)]
mod tests {
    use super::{MigrationError, Step, migrate, migrate_with};

    fn v1_to_v2(mut table: toml::Table) -> Result<toml::Table, MigrationError> {
        // A pretend change: `general.theme` moves to `general.appearance`.
        if let Some(toml::Value::Table(general)) = table.get_mut("general")
            && let Some(theme) = general.remove("theme")
        {
            general.insert(String::from("appearance"), theme);
        }
        Ok(table)
    }

    fn table(text: &str) -> toml::Table {
        text.parse().unwrap()
    }

    #[test]
    fn the_current_version_needs_no_steps() {
        let input = table("version = 1\n[general]\ntheme = \"dark\"\n");
        assert_eq!(migrate(input.clone(), 1).unwrap(), input);
    }

    #[test]
    fn a_step_runs_and_stamps_the_new_version() {
        let steps: &[Step] = &[v1_to_v2];
        let out = migrate_with(
            table("version = 1\n[general]\ntheme = \"dark\"\n"),
            1,
            2,
            steps,
        )
        .unwrap();
        assert_eq!(out["version"].as_integer(), Some(2));
        assert_eq!(out["general"]["appearance"].as_str(), Some("dark"));
        assert!(out["general"].get("theme").is_none());
    }

    #[test]
    fn a_missing_step_is_an_error_not_a_skip() {
        let error = migrate_with(table("version = 1\n"), 1, 3, &[v1_to_v2]).unwrap_err();
        assert_eq!(error.from, 2);
        assert!(error.to_string().contains("no migration step"));
    }
}
