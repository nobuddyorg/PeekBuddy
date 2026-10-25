//! `peekbuddy.exe`: one exe with two roles (ADR 0001).
//!
//! Without arguments it runs the engine; with `--viewer` it runs the viewer.
#![forbid(unsafe_code)]

use std::process::ExitCode;

#[cfg(feature = "profile-tracy")]
mod tracy;

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

/// Runs `role` to completion; in profiling builds inside one Tracy zone.
fn run(role: &Role) {
    #[cfg(feature = "profile-tracy")]
    let _zone = tracy::start(role);
    match role {
        Role::Engine => peekbuddy_engine::run(),
        Role::Viewer => peekbuddy_viewer::run(),
    }
}

/// Throwaway: nine megabytes of non-zero data, to prove the size gate (#28).
static BLOB: [u8; 9_000_000] = [1; 9_000_000];

fn main() -> ExitCode {
    std::hint::black_box(&BLOB);
    run(&role_from_args(std::env::args()));
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::{Role, role_from_args, run};

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

    #[test]
    fn both_roles_run_to_completion() {
        run(&Role::Engine);
        run(&Role::Viewer);
        #[cfg(feature = "profile-tracy")]
        assert!(tracy_client::Client::is_running());
    }
}
