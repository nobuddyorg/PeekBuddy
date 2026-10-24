//! `cargo xtask check-deps`: the always-running engine crates never pull in a
//! GPU or UI stack (ADR 0001, F1), nothing uses PDH (#24), and the per-record
//! capture path has no `tracing` macro (#21).

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
    let sources = product_sources()?;
    problems.extend(pdh_sources(&sources));
    problems.extend(tracing_sources(&sources));
    if problems.is_empty() {
        Ok(format!(
            "check-deps: ok ({} free of {}; no PDH; no tracing in the capture path)",
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

/// Directories of the per-record capture path: parse, name, filter, classify.
/// Release builds must not spend a span or an event per file event there (#21);
/// subsystem spans live in the engine.
const CAPTURE_DIRS: &[&str] = &[
    "crates/peekbuddy-core/src/derive/",
    "crates/peekbuddy-core/src/event/",
    "crates/peekbuddy-core/src/filter/",
    "crates/peekbuddy-core/src/name/",
    "crates/peekbuddy-core/src/notify/",
];

/// Macros of the `tracing` crate, spelled in parts so this file does not
/// match its own source scan.
const TRACING_MACROS: &[&str] = &[
    concat!("tracing", "::"),
    concat!("trace", "!("),
    concat!("debug", "!("),
    concat!("info", "!("),
    concat!("warn", "!("),
    concat!("error", "!("),
    concat!("span", "!("),
    concat!("#[", "instrument"),
    concat!("#[profiling", "::function"),
];

/// Problems in `(path, text)` source files of the capture path: a `tracing`
/// or `profiling` use per record.
pub fn tracing_sources(files: &[(String, String)]) -> Vec<String> {
    let mut problems = Vec::new();
    for (path, text) in files {
        let normalised = path.replace('\\', "/");
        if !CAPTURE_DIRS.iter().any(|dir| normalised.starts_with(dir)) {
            continue;
        }
        for (number, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            if TRACING_MACROS.iter().any(|needle| code.contains(needle)) {
                problems.push(format!(
                    "{path}:{}: no tracing macro in the capture path (#21)",
                    number + 1
                ));
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
    use super::{FORBIDDEN, PDH_FEATURE, check, pdh_features, pdh_sources, tracing_sources};

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
    #[test]
    fn tracing_in_the_capture_path_is_reported() {
        let hot = "crates/peekbuddy-core/src/notify/mod.rs";
        let found = tracing_sources(&files(&[(hot, "fn f() {\n    tracing::info!(\"x\");\n}")]));
        assert_eq!(found.len(), 1);
        assert!(found[0].contains(":2:"), "{found:?}");
        let attr = tracing_sources(&files(&[(hot, "#[instrument]\nfn f() {}")]));
        assert_eq!(attr.len(), 1);
    }

    #[test]
    fn tracing_elsewhere_and_in_comments_is_fine() {
        let engine = "crates/peekbuddy-engine/src/lib.rs";
        assert_eq!(
            tracing_sources(&files(&[(engine, "tracing::info!(\"x\");")])),
            Vec::<String>::new()
        );
        let hot = "crates/peekbuddy-core/src/filter/mod.rs";
        assert_eq!(
            tracing_sources(&files(&[(hot, "// tracing::info! is banned here")])),
            Vec::<String>::new()
        );
        assert_eq!(
            tracing_sources(&files(&[(hot, "let a = 1; // debug!(x)")])),
            Vec::<String>::new()
        );
    }

    #[test]
    fn windows_separators_are_normalised() {
        let hot = "crates\\peekbuddy-core\\src\\name\\mod.rs";
        assert_eq!(tracing_sources(&files(&[(hot, "warn!(\"x\");")])).len(), 1);
    }
}
