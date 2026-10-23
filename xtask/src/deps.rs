//! `cargo xtask check-deps`: the always-running engine crates never pull in a
//! GPU or UI stack (ADR 0001, F1).

use crate::metadata;

/// Crates that must stay free of GPU and UI dependencies.
pub const ENGINE_CRATES: &[&str] = &["peekbuddy-core", "peekbuddy-engine"];

/// Crates the engine must never depend on, directly or transitively.
pub const FORBIDDEN: &[&str] = &["eframe", "egui", "winit", "wgpu", "glow"];

pub fn run() -> Result<String, String> {
    let mut problems = Vec::new();
    for krate in ENGINE_CRATES {
        let tree = metadata::cargo(&[
            "tree",
            "--package",
            krate,
            "--edges",
            "normal,build",
            "--target",
            "all",
            "--prefix",
            "none",
            "--format",
            "{p}",
        ])?;
        problems.extend(check(krate, &tree, FORBIDDEN));
    }
    if problems.is_empty() {
        Ok(format!(
            "check-deps: ok ({} free of {})",
            ENGINE_CRATES.join(", "),
            FORBIDDEN.join(", ")
        ))
    } else {
        Err(format!("check-deps failed:\n  {}", problems.join("\n  ")))
    }
}

/// Checks the `cargo tree --prefix none --format {p}` output of `krate`.
pub fn check(krate: &str, tree: &str, forbidden: &[&str]) -> Vec<String> {
    let mut problems: Vec<String> = tree
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter(|name| forbidden.contains(name))
        .map(|name| format!("{krate} depends on forbidden crate {name}"))
        .collect();
    problems.sort();
    problems.dedup();
    problems
}

#[cfg(test)]
mod tests {
    use super::{FORBIDDEN, check};

    #[test]
    fn clean_tree_passes() {
        let tree =
            "peekbuddy-engine v0.0.0 (/w)\npeekbuddy-core v0.0.0 (/w)\nwindows-sys v0.61.2\n";
        assert_eq!(
            check("peekbuddy-engine", tree, FORBIDDEN),
            Vec::<String>::new()
        );
    }

    #[test]
    fn transitive_ui_crate_fails_once() {
        let tree = "peekbuddy-engine v0.0.0\neframe v0.37.0\negui v0.37.0\negui v0.37.0 (*)\n";
        let problems = check("peekbuddy-engine", tree, FORBIDDEN);
        assert_eq!(problems.len(), 2);
    }

    #[test]
    fn similar_names_do_not_match() {
        let tree = "egui_extras_but_not v1.0.0\nwgpu-types v1.0.0\n";
        assert_eq!(
            check("peekbuddy-core", tree, FORBIDDEN),
            Vec::<String>::new()
        );
    }
}
