//! Command line of `peekbuddy.exe`.
//!
//! Role dispatch happens before any heavy initialisation. The engine role
//! never initialises eframe, winit, wgpu or glow (F1); the viewer is only
//! reached through `--viewer`.

use std::path::PathBuf;

use peekbuddy_engine::Options;

/// The role this process runs.
#[derive(Debug, PartialEq, Eq)]
pub enum Role {
    /// The tray-resident engine (default).
    Engine,
    /// The on-demand viewer window (`--viewer`).
    Viewer,
}

/// A parsed command line.
#[derive(Debug, PartialEq, Eq)]
pub struct Invocation {
    /// Which role to run.
    pub role: Role,
    /// Engine options; default for the viewer.
    pub engine: Options,
}

/// Why the command line was refused.
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// An argument nobody knows.
    Unknown(String),
    /// `--data-dir` without a value.
    MissingValue(&'static str),
    /// `--data-dir` is for tests and the perf harness: it needs `--headless`.
    DataDirNeedsHeadless,
    /// Engine flags together with `--viewer`.
    EngineFlagsWithViewer,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(arg) => write!(f, "unknown argument {arg}"),
            Self::MissingValue(flag) => write!(f, "{flag} needs a value"),
            Self::DataDirNeedsHeadless => write!(f, "--data-dir is only accepted with --headless"),
            Self::EngineFlagsWithViewer => {
                write!(f, "--viewer cannot be combined with engine flags")
            }
        }
    }
}

/// Parses the arguments after the program name.
///
/// # Errors
///
/// Returns the first problem found.
pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Invocation, Error> {
    let mut viewer = false;
    let mut options = Options::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--viewer" => viewer = true,
            "--headless" => options.headless = true,
            "--data-dir" => {
                let value = args.next().ok_or(Error::MissingValue("--data-dir"))?;
                options.data_dir = Some(PathBuf::from(value));
            }
            #[cfg(feature = "test-panic")]
            "--test-panic" => options.panic_after_ready = true,
            _ => return Err(Error::Unknown(arg)),
        }
    }
    if options.data_dir.is_some() && !options.headless {
        return Err(Error::DataDirNeedsHeadless);
    }
    if viewer {
        if options != Options::default() {
            return Err(Error::EngineFlagsWithViewer);
        }
        return Ok(Invocation {
            role: Role::Viewer,
            engine: options,
        });
    }
    Ok(Invocation {
        role: Role::Engine,
        engine: options,
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{Error, Role, parse};

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn engine_is_the_default_role() {
        let invocation = parse(args(&[])).unwrap();
        assert_eq!(invocation.role, Role::Engine);
        assert!(!invocation.engine.headless);
    }

    #[test]
    fn viewer_flag_selects_the_viewer() {
        assert_eq!(parse(args(&["--viewer"])).unwrap().role, Role::Viewer);
    }

    #[test]
    fn headless_with_a_data_dir_is_accepted() {
        let invocation = parse(args(&["--headless", "--data-dir", r"C:\t\pb"])).unwrap();
        assert!(invocation.engine.headless);
        assert_eq!(invocation.engine.data_dir, Some(PathBuf::from(r"C:\t\pb")));
    }

    #[test]
    fn data_dir_alone_is_refused() {
        assert_eq!(
            parse(args(&["--data-dir", r"C:\t"])),
            Err(Error::DataDirNeedsHeadless)
        );
    }

    #[test]
    fn data_dir_without_a_value_is_refused() {
        assert_eq!(
            parse(args(&["--headless", "--data-dir"])),
            Err(Error::MissingValue("--data-dir"))
        );
    }

    #[test]
    fn unknown_and_mixed_flags_are_refused() {
        assert_eq!(
            parse(args(&["--frobnicate"])),
            Err(Error::Unknown(String::from("--frobnicate")))
        );
        assert_eq!(
            parse(args(&["--viewer", "--headless"])),
            Err(Error::EngineFlagsWithViewer)
        );
    }
}
