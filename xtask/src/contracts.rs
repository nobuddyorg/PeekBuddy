//! `cargo xtask check-contracts`: a public type of `peekbuddy-win` that owns
//! an OS resource (it implements `Drop`) or crosses threads through an
//! `unsafe impl Send` / `Sync` states its contract in a `# Contract` section
//! of its documentation: what it owns, when that is released, and which
//! threads may use it (#43).
//!
//! Public functions are covered by the lints already: `missing_docs`,
//! `missing_errors_doc`, `missing_panics_doc` and `missing_safety_doc` are
//! warnings that CI denies.

use std::path::{Path, PathBuf};

/// The crate checked.
const CRATE_SRC: &str = "crates/peekbuddy-win/src";
/// The heading a guarded type's documentation must have.
const HEADING: &str = "/// # Contract";

/// A source file: its path for messages and its text.
pub struct Source {
    pub path: PathBuf,
    pub text: String,
}

pub fn run() -> Result<String, String> {
    let mut sources = Vec::new();
    let mut stack = vec![Path::new(CRATE_SRC).to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries =
            std::fs::read_dir(&dir).map_err(|error| format!("{}: {error}", dir.display()))?;
        for entry in entries {
            let path = entry.map_err(|error| error.to_string())?.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs")
                && path.file_name().is_some_and(|name| name != "tests.rs")
            {
                let text = std::fs::read_to_string(&path)
                    .map_err(|error| format!("{}: {error}", path.display()))?;
                sources.push(Source { path, text });
            }
        }
    }
    sources.sort_by(|a, b| a.path.cmp(&b.path));
    let (problems, guarded) = check(&sources);
    if guarded == 0 {
        return Err(String::from(
            "check-contracts found no guarded type at all: the scanner is broken",
        ));
    }
    if problems.is_empty() {
        Ok(format!(
            "check-contracts: ok ({guarded} public types with a `# Contract` section)"
        ))
    } else {
        Err(format!(
            "check-contracts failed (add a `# Contract` section: what the type owns, when it is released, which threads may use it):\n  {}",
            problems.join("\n  ")
        ))
    }
}

/// The type a line implements `Drop`, `Send` or `Sync` for, with the reason.
fn guarded(line: &str) -> Option<(String, &'static str)> {
    let line = line.trim_start();
    let (rest, why) = [
        ("impl Drop for ", "implements Drop"),
        ("unsafe impl Send for ", "has an unsafe impl Send"),
        ("unsafe impl Sync for ", "has an unsafe impl Sync"),
    ]
    .iter()
    .find_map(|(prefix, why)| line.strip_prefix(prefix).map(|rest| (rest, *why)))?;
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    (!name.is_empty()).then_some((name, why))
}

/// Whether a line declares the public type `name`.
fn declares(line: &str, name: &str) -> bool {
    let line = line.trim_start();
    ["pub struct ", "pub enum "].iter().any(|prefix| {
        line.strip_prefix(prefix).is_some_and(|rest| {
            rest.strip_prefix(name)
                .is_some_and(|after| !after.starts_with(|c: char| c.is_alphanumeric() || c == '_'))
        })
    })
}

/// Whether the doc comment above line `index` has the heading.
fn has_contract(lines: &[&str], index: usize) -> bool {
    let mut at = index;
    while let Some(previous) = at.checked_sub(1) {
        let line = lines.get(previous).map_or("", |line| line.trim());
        if line.starts_with("#[") || line.starts_with("///") {
            if line == HEADING {
                return true;
            }
            at = previous;
        } else {
            break;
        }
    }
    false
}

/// One message per public guarded type without the heading, and how many
/// public guarded types there are.
pub fn check(sources: &[Source]) -> (Vec<String>, usize) {
    let mut names: Vec<(String, &'static str)> = Vec::new();
    for source in sources {
        for line in source.text.lines() {
            if let Some(found) = guarded(line)
                && !names.iter().any(|(name, _)| *name == found.0)
            {
                names.push(found);
            }
        }
    }
    let mut problems = Vec::new();
    let mut count = 0;
    for (name, why) in &names {
        for source in sources {
            let lines: Vec<&str> = source.text.lines().collect();
            for (index, line) in lines.iter().enumerate() {
                if declares(line, name) {
                    count += 1;
                    if !has_contract(&lines, index) {
                        problems.push(format!(
                            "{}:{}: `{name}` {why} but its documentation has no `# Contract` section",
                            source.path.display(),
                            index + 1
                        ));
                    }
                }
            }
        }
    }
    (problems, count)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{Source, check};

    fn source(text: &str) -> Source {
        Source {
            path: PathBuf::from("a.rs"),
            text: text.to_owned(),
        }
    }

    #[test]
    fn a_dropping_type_without_the_heading_is_listed() {
        let (problems, count) = check(&[source(
            "/// A handle.\npub struct Handle(u32);\nimpl Drop for Handle {\n    fn drop(&mut self) {}\n}\n",
        )]);
        assert_eq!(count, 1);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("a.rs:2"), "{problems:?}");
        assert!(problems[0].contains("implements Drop"), "{problems:?}");
    }

    #[test]
    fn the_heading_above_attributes_counts() {
        let (problems, count) = check(&[source(
            "/// A handle.\n///\n/// # Contract\n///\n/// Closes on drop.\n#[derive(Debug)]\npub struct Handle(u32);\nimpl Drop for Handle {\n    fn drop(&mut self) {}\n}\n",
        )]);
        assert_eq!((problems.len(), count), (0, 1), "{problems:?}");
    }

    #[test]
    fn unsafe_send_and_sync_are_guarded_too() {
        let (problems, count) = check(&[source(
            "/// A watch.\npub struct Watch;\nunsafe impl Send for Watch {}\nunsafe impl Sync for Watch {}\n",
        )]);
        assert_eq!(count, 1);
        assert_eq!(problems.len(), 1, "{problems:?}");
    }

    #[test]
    fn private_types_and_look_alike_names_are_ignored() {
        let (problems, count) = check(&[source(
            "struct Private;\nimpl Drop for Private {\n    fn drop(&mut self) {}\n}\n/// Other.\npub struct PrivateToo;\n",
        )]);
        assert_eq!((problems.len(), count), (0, 0), "{problems:?}");
    }

    #[test]
    fn a_heading_of_another_type_does_not_count() {
        let (problems, _) = check(&[source(
            "/// # Contract\npub struct Other;\n/// Plain.\npub struct Handle;\nimpl Drop for Handle {\n    fn drop(&mut self) {}\n}\n",
        )]);
        assert_eq!(problems.len(), 1, "{problems:?}");
    }
}
