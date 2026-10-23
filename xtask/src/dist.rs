//! `cargo xtask dist-extras`: supply-chain files shipped in the release zip
//! (#12): `THIRD_PARTY_LICENSES.html` (cargo-about) and a `CycloneDX` SBOM
//! (cargo-cyclonedx). Output goes to `target/dist-extras/`.

use std::path::Path;

use crate::metadata;

const OUT: &str = "target/dist-extras";

pub fn run() -> Result<String, String> {
    std::fs::create_dir_all(OUT).map_err(|error| format!("{OUT}: {error}"))?;
    let licenses = Path::new(OUT).join("THIRD_PARTY_LICENSES.html");
    let licenses_arg = licenses.to_string_lossy();
    metadata::cargo(&[
        "about",
        "generate",
        "--locked",
        "--workspace",
        "--output-file",
        &licenses_arg,
        "dist/about.hbs",
    ])?;
    metadata::cargo(&[
        "cyclonedx",
        "--manifest-path",
        "crates/peekbuddy/Cargo.toml",
        "--format",
        "json",
        "--spec-version",
        "1.5",
        "--target",
        "x86_64-pc-windows-msvc",
        "--override-filename",
        "peekbuddy.cdx",
    ])?;
    let sbom = Path::new("crates/peekbuddy/peekbuddy.cdx.json");
    let sbom_out = Path::new(OUT).join("peekbuddy.cdx.json");
    std::fs::rename(sbom, &sbom_out).map_err(|error| format!("{}: {error}", sbom.display()))?;
    // cargo-cyclonedx writes one file per workspace member; only the
    // shipped bin's SBOM is kept.
    for krate in metadata::workspace_crates()? {
        for root in &krate.roots {
            if let Some(dir) = root.parent().and_then(Path::parent) {
                let _ = std::fs::remove_file(dir.join("peekbuddy.cdx.json"));
            }
        }
    }
    Ok(format!(
        "dist-extras: {} and {}",
        licenses.display(),
        sbom_out.display()
    ))
}
