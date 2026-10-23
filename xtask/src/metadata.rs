//! Reads the workspace layout from `cargo metadata`.

use std::path::PathBuf;
use std::process::Command;

/// A workspace crate and the source files of its library and binary targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crate {
    /// Package name.
    pub name: String,
    /// Crate-root source files (`lib.rs`, `main.rs`).
    pub roots: Vec<PathBuf>,
}

/// Runs `cargo metadata` and returns the workspace's own crates.
pub fn workspace_crates() -> Result<Vec<Crate>, String> {
    let json = cargo(&["metadata", "--no-deps", "--format-version", "1"])?;
    parse_workspace_crates(&json)
}

/// Runs cargo with `args` and returns its stdout.
pub fn cargo(args: &[&str]) -> Result<String, String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let output = Command::new(cargo)
        .args(args)
        .output()
        .map_err(|error| format!("cannot run cargo {}: {error}", args.join(" ")))?;
    if !output.status.success() {
        return Err(format!(
            "cargo {} failed:\n{}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    String::from_utf8(output.stdout).map_err(|error| error.to_string())
}

fn parse_workspace_crates(json: &str) -> Result<Vec<Crate>, String> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let packages = value
        .get("packages")
        .and_then(serde_json::Value::as_array)
        .ok_or("cargo metadata has no packages")?;
    let mut crates = Vec::new();
    for package in packages {
        let name = package
            .get("name")
            .and_then(serde_json::Value::as_str)
            .ok_or("package without a name")?
            .to_owned();
        let roots = package
            .get("targets")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter(|target| {
                target
                    .get("kind")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|kinds| {
                        kinds
                            .iter()
                            .any(|kind| kind == "lib" || kind == "bin" || kind == "proc-macro")
                    })
            })
            .filter_map(|target| target.get("src_path").and_then(serde_json::Value::as_str))
            .map(PathBuf::from)
            .collect();
        crates.push(Crate { name, roots });
    }
    crates.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(crates)
}

#[cfg(test)]
mod tests {
    use super::parse_workspace_crates;

    #[test]
    fn keeps_lib_and_bin_roots_only() {
        let json = r#"{"packages":[{"name":"a","targets":[
            {"kind":["lib"],"src_path":"/a/src/lib.rs"},
            {"kind":["test"],"src_path":"/a/tests/t.rs"},
            {"kind":["bin"],"src_path":"/a/src/main.rs"}]}]}"#;
        let crates = parse_workspace_crates(json).unwrap();
        assert_eq!(crates.len(), 1);
        assert_eq!(crates[0].name, "a");
        assert_eq!(crates[0].roots.len(), 2);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_workspace_crates("not json").is_err());
    }
}
