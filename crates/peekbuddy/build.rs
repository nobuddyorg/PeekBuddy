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

fn main() -> Result<(), Box<dyn Error>> {
    directive("cargo:rerun-if-changed=build.rs")?;
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
