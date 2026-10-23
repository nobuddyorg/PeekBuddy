//! `peekbuddy.exe`: one exe with two roles (ADR 0001).
//!
//! Without arguments it runs the engine; with `--viewer` it runs the viewer.
#![forbid(unsafe_code)]

use std::process::ExitCode;

/// The role this process runs.
#[derive(Debug, PartialEq, Eq)]
enum Role {
    Engine,
    Viewer,
}

fn role_from_args(args: impl IntoIterator<Item = String>) -> Role {
    if args.into_iter().skip(1).any(|arg| arg == "--viewer") {
        Role::Viewer
    } else {
        Role::Engine
    }
}

fn main() -> ExitCode {
    match role_from_args(std::env::args()) {
        Role::Engine => peekbuddy_engine::run(),
        Role::Viewer => peekbuddy_viewer::run(),
    }
}

#[cfg(test)]
mod tests {
    use super::{Role, role_from_args};

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn engine_is_the_default_role() {
        assert_eq!(role_from_args(args(&["peekbuddy.exe"])), Role::Engine);
    }

    #[test]
    fn viewer_flag_selects_the_viewer() {
        assert_eq!(
            role_from_args(args(&["peekbuddy.exe", "--viewer"])),
            Role::Viewer
        );
    }
}
