//! Embeds the manifest and the version resource into `peekbuddy.exe` (#59).
//!
//! The manifest comes from embed-manifest, which needs no external tools and
//! also works when cross-compiling. VERSIONINFO comes from winresource, which
//! needs `rc.exe` (the Windows SDK, present on hosted runners); where that
//! tool is missing the exe is built without it and a warning says so.

use std::error::Error;
use std::io::Write as _;
use std::path::Path;

use embed_manifest::manifest::{
    ActiveCodePage, DpiAwareness, ExecutionLevel, MaxVersionTested, Setting, SupportedOS,
};
use embed_manifest::{embed_manifest, new_manifest};

fn directive(line: &str) -> std::io::Result<()> {
    writeln!(std::io::stdout(), "{line}")
}

/// Whether `rc.exe` can be found on `PATH` or in a Windows SDK install.
fn rc_available() -> bool {
    let on_path = std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join("rc.exe").is_file()));
    let kits = std::env::var_os("ProgramFiles(x86)").is_some_and(|dir| {
        Path::new(&dir)
            .join("Windows Kits")
            .join("10")
            .join("bin")
            .is_dir()
    });
    on_path || kits
}

/// Runs `program args` and returns its trimmed standard output.
fn output(program: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(program)
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .filter(|text| !text.is_empty())
}

/// Versions of the dependencies `--diagnostics` reports, from `Cargo.lock`
/// as `name=version,name=version`.
fn dependency_versions(lock: &str) -> String {
    const KEY: [&str; 6] = [
        "windows-sys",
        "rusqlite",
        "libsqlite3-sys",
        "regex-automata",
        "eframe",
        "wgpu",
    ];
    let mut found = Vec::new();
    let mut name = None;
    for line in lock.lines() {
        if let Some(value) = line.strip_prefix("name = ") {
            name = Some(value.trim_matches('"').to_owned());
        } else if let (Some(value), Some(current)) = (line.strip_prefix("version = "), &name) {
            if KEY.contains(&current.as_str()) {
                found.push(format!("{current}={}", value.trim_matches('"')));
            }
            name = None;
        }
    }
    found.join(",")
}

/// Build facts for `--diagnostics`: git SHA, compiler, key dependencies.
fn build_facts() -> std::io::Result<()> {
    let sha =
        output("git", &["rev-parse", "--short=12", "HEAD"]).unwrap_or_else(|| "unknown".into());
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let rustc = output(&rustc, &["-V"]).unwrap_or_else(|| "unknown".into());
    let lock = std::fs::read_to_string(Path::new("../../Cargo.lock")).unwrap_or_default();
    directive(&format!("cargo:rustc-env=PEEKBUDDY_GIT_SHA={sha}"))?;
    directive(&format!("cargo:rustc-env=PEEKBUDDY_RUSTC={rustc}"))?;
    directive(&format!(
        "cargo:rustc-env=PEEKBUDDY_DEPS={}",
        dependency_versions(&lock)
    ))?;
    directive("cargo:rerun-if-changed=../../Cargo.lock")?;
    directive("cargo:rerun-if-changed=../../.git/HEAD")
}

fn main() -> Result<(), Box<dyn Error>> {
    directive("cargo:rerun-if-changed=build.rs")?;
    build_facts()?;
    if std::env::var_os("CARGO_CFG_WINDOWS").is_none() {
        return Ok(());
    }
    // PerMonitorV2, long paths, UTF-8 ANSI code page, no elevation, Common
    // Controls v6 (a default of the builder) and the supported-OS GUIDs of
    // ADR 0013. Exactly one RT_MANIFEST ends up in the exe.
    embed_manifest(
        new_manifest("NoBuddy.PeekBuddy")
            .supported_os(SupportedOS::Windows10..=SupportedOS::Windows10)
            .max_version_tested(MaxVersionTested::Windows11Version22H2)
            .dpi_awareness(DpiAwareness::PerMonitorV2)
            .long_path_aware(Setting::Enabled)
            .active_code_page(ActiveCodePage::Utf8)
            .requested_execution_level(ExecutionLevel::AsInvoker),
    )?;

    if cfg!(windows) && rc_available() {
        let mut resource = winresource::WindowsResource::new();
        resource.set("FileDescription", "PeekBuddy");
        resource.set("ProductName", "PeekBuddy");
        resource.set("CompanyName", "NoBuddy");
        resource.compile()?;
    } else {
        directive("cargo:warning=rc.exe not found: building peekbuddy without VERSIONINFO")?;
    }
    Ok(())
}
