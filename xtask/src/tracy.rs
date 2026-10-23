//! Tracy profiling build (#22): `cargo xtask profile` and the guards that keep
//! Tracy out of release builds (`cargo xtask check-release`).

use std::path::Path;
use std::process::Command;

use crate::metadata;

/// Tracy GUI release whose protocol matches `tracy-client-sys` 0.30.1.
pub const TRACY_GUI: &str = "0.14.1";

/// The crate whose presence means Tracy code is linked.
const TRACY_SYS: &str = "tracy-client-sys";

/// Byte strings that only a Tracy-enabled binary contains.
pub const MARKERS: &[&[u8]] = &[b"___tracy", b"Tracy"];

/// `cargo xtask check-release [--features <list>] [--exe <path>]`.
///
/// Fails when the feature set (default: the release set, no extra features)
/// pulls in `tracy-client-sys`, or when the exe contains a Tracy marker.
pub fn check_release(args: &[String]) -> Result<String, String> {
    let features = option(args, "--features")?;
    let exe = option(args, "--exe")?;
    let mut tree_args = vec![
        "tree",
        "--package",
        "peekbuddy",
        "--edges",
        "normal,build",
        "--target",
        "all",
        "--invert",
        TRACY_SYS,
    ];
    if let Some(features) = features {
        tree_args.extend(["--features", features]);
    }
    let tree = match metadata::cargo(&tree_args) {
        Ok(tree) => Some(tree),
        Err(error) if error.contains("did not match any packages") => None,
        Err(error) => return Err(error),
    };
    let mut problems = Vec::new();
    if let Some(tree) = tree.filter(|tree| !tree.trim().is_empty()) {
        problems.push(format!(
            "the release feature set links {TRACY_SYS}:\n{tree}"
        ));
    }
    if let Some(exe) = exe {
        let bytes = std::fs::read(exe).map_err(|error| format!("cannot read {exe}: {error}"))?;
        problems.extend(
            markers_in(&bytes)
                .into_iter()
                .map(|marker| format!("{exe} contains {marker:?}")),
        );
    }
    if problems.is_empty() {
        Ok(match exe {
            Some(exe) => format!("check-release: ok (no {TRACY_SYS}, no Tracy strings in {exe})"),
            None => format!("check-release: ok (no {TRACY_SYS})"),
        })
    } else {
        Err(format!(
            "check-release failed:\n  {}",
            problems.join("\n  ")
        ))
    }
}

/// The Tracy markers found in `bytes`, in [`MARKERS`] order.
pub fn markers_in(bytes: &[u8]) -> Vec<String> {
    MARKERS
        .iter()
        .filter(|marker| bytes.windows(marker.len()).any(|window| window == **marker))
        .map(|marker| String::from_utf8_lossy(marker).into_owned())
        .collect()
}

/// `cargo xtask profile --tracy [--alloc] [--role engine|viewer] [--target <triple>]`.
pub fn profile(args: &[String]) -> Result<String, String> {
    if !args.iter().any(|arg| arg == "--tracy") {
        return Err(
            "usage: cargo xtask profile --tracy [--alloc] [--role engine|viewer] \
                    [--target <triple>]"
                .to_owned(),
        );
    }
    let role = option(args, "--role")?.unwrap_or("engine");
    let run_args = match role {
        "engine" => "",
        "viewer" => " --viewer",
        other => return Err(format!("unknown role {other:?} (engine or viewer)")),
    };
    let features = if args.iter().any(|arg| arg == "--alloc") {
        "profile-tracy-alloc"
    } else {
        "profile-tracy"
    };
    let target = option(args, "--target")?;
    let mut build = vec![
        "build",
        "--package",
        "peekbuddy",
        "--profile",
        "profiling",
        "--features",
        features,
        "--locked",
    ];
    if let Some(target) = target {
        build.extend(["--target", target]);
    }
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let status = Command::new(cargo)
        .args(&build)
        .status()
        .map_err(|error| format!("cannot run cargo build: {error}"))?;
    if !status.success() {
        return Err(format!("cargo {} failed", build.join(" ")));
    }
    let dir = match target {
        Some(target) => Path::new("target").join(target).join("profiling"),
        None => Path::new("target").join("profiling"),
    };
    let exe = dir.join("peekbuddy.exe");
    Ok(format!(
        "built {exe} ({features})\n\
         connect Tracy GUI {TRACY_GUI} (other versions cannot connect), then run:\n  \
         {exe}{run_args}\n\
         call stacks and context switches need an elevated process",
        exe = exe.display()
    ))
}

/// The value after `name` in `args`, if `name` is present.
fn option<'a>(args: &'a [String], name: &str) -> Result<Option<&'a str>, String> {
    match args.iter().position(|arg| arg == name) {
        None => Ok(None),
        Some(index) => args
            .get(index + 1)
            .map(|value| Some(value.as_str()))
            .ok_or_else(|| format!("{name} needs a value")),
    }
}

#[cfg(test)]
mod tests {
    use super::{markers_in, option};

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn a_clean_binary_has_no_markers() {
        assert_eq!(
            markers_in(b"MZ\x90\0 just a tray app"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn tracy_symbols_are_found() {
        assert_eq!(
            markers_in(b"\0___tracy_emit_zone_begin\0TracyClient\0"),
            ["___tracy", "Tracy"]
        );
        assert_eq!(markers_in(b"xx___tracy"), ["___tracy"]);
    }

    #[test]
    fn options_take_the_next_argument() {
        let list = args(&["--exe", "a.exe", "--features", "x"]);
        assert_eq!(option(&list, "--exe"), Ok(Some("a.exe")));
        assert_eq!(option(&list, "--features"), Ok(Some("x")));
        assert_eq!(option(&list, "--role"), Ok(None));
        assert!(option(&args(&["--exe"]), "--exe").is_err());
    }
}
