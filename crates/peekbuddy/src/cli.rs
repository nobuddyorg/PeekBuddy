//! Command line of `peekbuddy.exe`.
//!
//! Role dispatch happens before any heavy initialisation. The engine role
//! never initialises eframe, winit, wgpu or glow (F1); the viewer is only
//! reached through `--viewer`.

use std::path::PathBuf;

use peekbuddy_engine::{Options, ProbeOptions};

/// The role this process runs.
#[derive(Debug, PartialEq, Eq)]
pub enum Role {
    /// The tray-resident engine (default).
    Engine,
    /// The on-demand viewer window (`--viewer`).
    Viewer,
    /// Print the diagnostics text and exit (`--diagnostics`).
    Diagnostics,
    /// Connect to the running engine's pipe and say hello (`--ipc-ping`);
    /// exits with the Windows error code on failure.
    Ping,
}

/// A parsed command line.
#[derive(Debug, PartialEq, Eq)]
pub struct Invocation {
    /// Which role to run.
    pub role: Role,
    /// Engine options; default for the other roles.
    pub engine: Options,
    /// `--full`: show watched paths and user folders in the diagnostics.
    pub full: bool,
}

/// Why the command line was refused.
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// An argument nobody knows.
    Unknown(String),
    /// A flag without its value.
    MissingValue(&'static str),
    /// A value that is not a whole number of seconds.
    BadSeconds(String),
    /// `--data-dir` is for tests and the perf harness: it needs `--headless`
    /// or `--perf-probe`.
    DataDirNeedsHeadless,
    /// `--perf-probe-out` without `--perf-probe`, or `--full` without
    /// `--diagnostics`.
    OrphanFlag(&'static str),
    /// Flags of one role together with another role.
    RoleFlagsMixed,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(arg) => write!(f, "unknown argument {arg}"),
            Self::MissingValue(flag) => write!(f, "{flag} needs a value"),
            Self::BadSeconds(value) => write!(f, "{value} is not a number of seconds"),
            Self::DataDirNeedsHeadless => write!(
                f,
                "--data-dir is only accepted with --headless or --perf-probe"
            ),
            Self::OrphanFlag(flag) => write!(f, "{flag} needs its command"),
            Self::RoleFlagsMixed => write!(
                f,
                "--viewer, --diagnostics and the engine flags exclude each other"
            ),
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
    let mut diagnostics = false;
    let mut ping = false;
    let mut full = false;
    let mut probe_out = None;
    let mut options = Options::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--viewer" => viewer = true,
            "--headless" => options.headless = true,
            "--tray" => options.tray = true,
            "--diagnostics" => diagnostics = true,
            "--ipc-ping" => ping = true,
            "--full" => full = true,
            "--data-dir" => {
                let value = args.next().ok_or(Error::MissingValue("--data-dir"))?;
                options.data_dir = Some(PathBuf::from(value));
            }
            "--perf-probe" => {
                let value = args.next().ok_or(Error::MissingValue("--perf-probe"))?;
                let seconds = value.parse().map_err(|_| Error::BadSeconds(value))?;
                options.probe = Some(ProbeOptions { seconds, out: None });
            }
            "--perf-probe-out" => {
                let value = args.next().ok_or(Error::MissingValue("--perf-probe-out"))?;
                probe_out = Some(PathBuf::from(value));
            }
            #[cfg(feature = "test-panic")]
            "--test-panic" => options.panic_after_ready = true,
            _ => return Err(Error::Unknown(arg)),
        }
    }
    if let Some(out) = probe_out {
        let probe = options
            .probe
            .as_mut()
            .ok_or(Error::OrphanFlag("--perf-probe-out"))?;
        probe.out = Some(out);
    }
    if options.tray && !options.headless {
        return Err(Error::OrphanFlag("--tray"));
    }
    if full && !diagnostics {
        return Err(Error::OrphanFlag("--full"));
    }
    if options.data_dir.is_some() && !options.headless && options.probe.is_none() && !ping {
        return Err(Error::DataDirNeedsHeadless);
    }
    let role = match (viewer, diagnostics, ping) {
        (true, false, false) => Role::Viewer,
        (false, true, false) => Role::Diagnostics,
        (false, false, true) => Role::Ping,
        (false, false, false) => Role::Engine,
        _ => return Err(Error::RoleFlagsMixed),
    };
    // `--data-dir` selects which engine to ping; nothing else is allowed.
    let ping_options = Options {
        data_dir: options.data_dir.clone(),
        ..Options::default()
    };
    if role == Role::Ping && options != ping_options {
        return Err(Error::RoleFlagsMixed);
    }
    if role != Role::Engine && role != Role::Ping && options != Options::default() {
        return Err(Error::RoleFlagsMixed);
    }
    Ok(Invocation {
        role,
        engine: options,
        full,
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
            Err(Error::RoleFlagsMixed)
        );
    }

    #[test]
    fn the_probe_takes_seconds_and_an_optional_file() {
        let invocation = parse(args(&["--perf-probe", "5"])).unwrap();
        let probe = invocation.engine.probe.unwrap();
        assert_eq!((probe.seconds, probe.out), (5, None));

        let invocation = parse(args(&[
            "--perf-probe",
            "0",
            "--perf-probe-out",
            r"C:\t\p.json",
        ]))
        .unwrap();
        let probe = invocation.engine.probe.unwrap();
        assert_eq!(probe.out, Some(PathBuf::from(r"C:\t\p.json")));
    }

    #[test]
    fn the_probe_accepts_a_data_dir_without_headless() {
        let invocation = parse(args(&["--perf-probe", "0", "--data-dir", r"C:	"])).unwrap();
        assert_eq!(invocation.role, Role::Engine);
    }

    #[test]
    fn bad_probe_arguments_are_refused() {
        assert_eq!(
            parse(args(&["--perf-probe", "soon"])),
            Err(Error::BadSeconds(String::from("soon")))
        );
        assert_eq!(
            parse(args(&["--perf-probe"])),
            Err(Error::MissingValue("--perf-probe"))
        );
        assert_eq!(
            parse(args(&["--perf-probe-out", "x"])),
            Err(Error::OrphanFlag("--perf-probe-out"))
        );
    }

    #[test]
    fn tray_is_an_addition_to_headless() {
        let invocation = parse(args(&["--headless", "--tray"])).unwrap();
        assert!(invocation.engine.tray);
        assert_eq!(parse(args(&["--tray"])), Err(Error::OrphanFlag("--tray")));
    }

    #[test]
    fn ping_takes_only_a_data_dir() {
        assert_eq!(parse(args(&["--ipc-ping"])).unwrap().role, Role::Ping);
        let pinged = parse(args(&["--ipc-ping", "--data-dir", r"C:	"])).unwrap();
        assert_eq!(pinged.engine.data_dir, Some(PathBuf::from(r"C:	")));
        assert_eq!(
            parse(args(&["--ipc-ping", "--headless"])),
            Err(Error::RoleFlagsMixed)
        );
        assert_eq!(
            parse(args(&["--ipc-ping", "--viewer"])),
            Err(Error::RoleFlagsMixed)
        );
    }

    #[test]
    fn diagnostics_is_its_own_role_and_full_belongs_to_it() {
        let invocation = parse(args(&["--diagnostics", "--full"])).unwrap();
        assert_eq!(invocation.role, Role::Diagnostics);
        assert!(invocation.full);
        assert_eq!(parse(args(&["--full"])), Err(Error::OrphanFlag("--full")));
        assert_eq!(
            parse(args(&["--diagnostics", "--headless"])),
            Err(Error::RoleFlagsMixed)
        );
        assert_eq!(
            parse(args(&["--diagnostics", "--viewer"])),
            Err(Error::RoleFlagsMixed)
        );
    }
}
