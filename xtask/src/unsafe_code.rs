//! `cargo xtask check-unsafe`: every crate root forbids `unsafe` code, except
//! the crates on the allow-list.

use crate::metadata::{self, Crate};

/// Crates allowed to contain `unsafe` code. Changing this list is a reviewed
/// decision (see CONTRIBUTING.md, "Workspace layout").
pub const ALLOWED: &[&str] = &["peekbuddy-alloc", "peekbuddy-win"];

const FORBID: &str = "#![forbid(unsafe_code)]";

pub fn run() -> Result<String, String> {
    let crates = metadata::workspace_crates()?;
    let sources = crates
        .iter()
        .map(|krate| {
            let texts = krate
                .roots
                .iter()
                .map(|root| {
                    std::fs::read_to_string(root)
                        .map_err(|error| format!("{}: {error}", root.display()))
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok((krate.clone(), texts))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let problems = check(&sources, ALLOWED);
    if problems.is_empty() {
        Ok(format!(
            "check-unsafe: ok ({} crates, unsafe allowed in {})",
            crates.len(),
            ALLOWED.join(", ")
        ))
    } else {
        Err(format!("check-unsafe failed:\n  {}", problems.join("\n  ")))
    }
}

/// Returns one message per violation. `sources` pairs each crate with the
/// text of its crate roots.
pub fn check(sources: &[(Crate, Vec<String>)], allowed: &[&str]) -> Vec<String> {
    let mut problems = Vec::new();
    for (krate, texts) in sources {
        if allowed.contains(&krate.name.as_str()) {
            continue;
        }
        for (root, text) in krate.roots.iter().zip(texts) {
            if !text.lines().any(|line| line.trim() == FORBID) {
                problems.push(format!(
                    "{} ({}) is missing `{FORBID}`",
                    krate.name,
                    root.display()
                ));
            }
        }
    }
    for name in allowed {
        if !sources.iter().any(|(krate, _)| krate.name == *name) {
            problems.push(format!("allow-listed crate {name} does not exist"));
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::{Crate, check};
    use std::path::PathBuf;

    fn krate(name: &str, text: &str) -> (Crate, Vec<String>) {
        (
            Crate {
                name: name.to_owned(),
                roots: vec![PathBuf::from(format!("{name}/src/lib.rs"))],
            },
            vec![text.to_owned()],
        )
    }

    #[test]
    fn passes_when_every_crate_forbids_unsafe() {
        let sources = [
            krate("a", "//! Doc.\n#![forbid(unsafe_code)]\n"),
            krate("win", "//! May use unsafe.\n"),
        ];
        assert_eq!(check(&sources, &["win"]), Vec::<String>::new());
    }

    #[test]
    fn fails_on_a_crate_without_forbid() {
        let sources = [krate("a", "//! Doc.\n"), krate("win", "")];
        let problems = check(&sources, &["win"]);
        assert_eq!(problems.len(), 1);
        assert!(problems[0].starts_with("a "));
    }

    #[test]
    fn a_commented_out_forbid_does_not_count() {
        let sources = [krate("a", "// #![forbid(unsafe_code)]\n"), krate("win", "")];
        assert_eq!(check(&sources, &["win"]).len(), 1);
    }

    #[test]
    fn fails_on_a_stale_allow_list_entry() {
        let sources = [krate("a", "#![forbid(unsafe_code)]")];
        assert_eq!(check(&sources, &["gone"]).len(), 1);
    }
}
