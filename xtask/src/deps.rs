//! `cargo xtask check-deps`: the always-running engine crates never pull in a
//! GPU or UI stack (ADR 0001, F1), and nothing uses PDH (#24).

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
    let features = metadata::cargo(&[
        "tree",
        "--workspace",
        "--target",
        "all",
        "--edges",
        "features",
        "--prefix",
        "none",
        "--format",
        "{p}",
    ])?;
    problems.extend(pdh_features(&features));
    problems.extend(pdh_sources(&product_sources()?));
    if problems.is_empty() {
        Ok(format!(
            "check-deps: ok ({} free of {}; no PDH)",
            ENGINE_CRATES.join(", "),
            FORBIDDEN.join(", ")
        ))
    } else {
        Err(format!("check-deps failed:\n  {}", problems.join("\n  ")))
    }
}

/// The windows-sys / windows feature that exposes the PDH API. Spelled in
/// two parts so this file does not match its own source scan.
const PDH_FEATURE: &str = concat!("Win32_System_", "Performance");

/// Directories with product code: PDH is banned there (#24). Its
/// "% Processor Time" exceeds 100 % on multi-core machines and the legacy
/// Process counterset matches processes by name.
const PRODUCT_DIRS: &[&str] = &["crates", "benches"];

/// Problems in `cargo tree -e features --prefix none --format {p}` output.
pub fn pdh_features(tree: &str) -> Vec<String> {
    let needle = format!("feature \"{PDH_FEATURE}\"");
    let mut problems: Vec<String> = tree
        .lines()
        .filter(|line| line.contains(&needle))
        .filter_map(|line| line.split_whitespace().next())
        .map(|krate| format!("{krate} enables {PDH_FEATURE} (PDH is banned, #24)"))
        .collect();
    problems.sort();
    problems.dedup();
    problems
}

/// Problems in `(path, text)` source files: a `Pdh*` API name or `pdh.dll`.
pub fn pdh_sources(files: &[(String, String)]) -> Vec<String> {
    let mut problems = Vec::new();
    for (path, text) in files {
        for (number, line) in text.lines().enumerate() {
            let api = line.match_indices("Pdh").any(|(at, _)| {
                let before = line[..at].chars().next_back();
                let after = line[at + 3..].chars().next();
                !before.is_some_and(|c| c.is_alphanumeric() || c == '_')
                    && after.is_some_and(|c| c.is_ascii_uppercase())
            });
            if api || line.to_ascii_lowercase().contains("pdh.dll") {
                problems.push(format!("{path}:{}: PDH is banned (#24)", number + 1));
            }
        }
    }
    problems
}

/// Every `.rs` and `Cargo.toml` file under [`PRODUCT_DIRS`].
fn product_sources() -> Result<Vec<(String, String)>, String> {
    let mut files = Vec::new();
    let mut dirs: Vec<std::path::PathBuf> = PRODUCT_DIRS.iter().map(Into::into).collect();
    while let Some(dir) = dirs.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for entry in entries {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs")
                || path.file_name().is_some_and(|name| name == "Cargo.toml")
            {
                let text = std::fs::read_to_string(&path)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                files.push((path.display().to_string(), text));
            }
        }
    }
    files.sort();
    Ok(files)
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
    use super::{FORBIDDEN, PDH_FEATURE, check, pdh_features, pdh_sources};

    fn files(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(path, text)| ((*path).to_owned(), (*text).to_owned()))
            .collect()
    }

    #[test]
    fn the_pdh_feature_fails() {
        let tree = format!(
            "windows-sys feature \"Win32_Foundation\"\nwindows-sys feature \"{PDH_FEATURE}\"\n\
             windows-sys feature \"{PDH_FEATURE}\" (*)\n"
        );
        assert_eq!(pdh_features(&tree).len(), 1);
        assert_eq!(
            pdh_features("windows-sys feature \"Win32_System_Threading\"\n"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn pdh_calls_and_the_dll_fail() {
        let bad = files(&[
            ("a.rs", "let q = PdhOpenQueryW(null, 0, &mut h);"),
            ("b.rs", "// loads PDH.DLL\nfn x() {}"),
            (
                "c.rs",
                "use windows_sys::Win32::System::Performance::PdhCollectQueryData;",
            ),
        ]);
        assert_eq!(
            pdh_sources(&bad),
            [
                "a.rs:1: PDH is banned (#24)",
                "b.rs:1: PDH is banned (#24)",
                "c.rs:1: PDH is banned (#24)"
            ]
        );
    }

    #[test]
    fn words_containing_pdh_pass() {
        let good = files(&[
            (
                "a.rs",
                "// Never use PDH: its counters exceed 100 %.\nlet updhx = 1;",
            ),
            ("b.rs", "fn my_PdhLike() {}\nlet Pdh = 2; let pdhdll = 3;"),
        ]);
        assert_eq!(pdh_sources(&good), Vec::<String>::new());
    }

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
