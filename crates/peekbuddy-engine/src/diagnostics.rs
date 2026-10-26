//! `peekbuddy --diagnostics [--full]` (#24, #161).
//!
//! The text a bug report pastes. Watched paths and user folders are redacted
//! unless `--full` is given; the default output has no absolute user paths.

use std::fmt::Write as _;

/// Build facts the exe knows at compile time.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facts {
    /// Package version.
    pub version: String,
    /// Short git SHA, or `unknown`.
    pub git_sha: String,
    /// `rustc -V` of the build, or `unknown`.
    pub rustc: String,
    /// Key dependency versions as `(name, version)`.
    pub dependencies: Vec<(String, String)>,
}

impl Facts {
    /// Parses `name=version,name=version` as written by the exe's build
    /// script.
    #[must_use]
    pub fn parse_dependencies(list: &str) -> Vec<(String, String)> {
        list.split(',')
            .filter_map(|pair| pair.split_once('='))
            .map(|(name, version)| (name.to_owned(), version.to_owned()))
            .collect()
    }
}

/// Everything `--diagnostics` prints.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diagnostics {
    /// Compile-time facts.
    pub facts: Facts,
    /// Bundled SQLite version, or why it is not known.
    pub sqlite: String,
    /// `major.minor.build.UBR`.
    pub os: String,
    /// Architecture the process runs as and whether that is native.
    pub arch: String,
    /// Renderer and adapter the viewer recorded last, if it did.
    pub renderer: Option<String>,
    /// Data directory, redacted unless `--full`.
    pub data_dir: String,
    /// How the data directory was chosen.
    pub data_mode: String,
    /// State of `settings.toml`, read without changing anything.
    pub settings: String,
    /// Whether an engine runs in this session.
    pub engine: String,
    /// Watch states, from the engine pipe (#61) when it exists.
    pub watches: String,
    /// Defender real-time protection, when the registry says.
    pub defender: String,
}

impl Diagnostics {
    /// The text, one `key: value` per line.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        let mut line = |key: &str, value: &str| {
            let _ = writeln!(out, "{key}: {value}");
        };
        line(
            "PeekBuddy",
            &format!("{} ({})", self.facts.version, self.facts.git_sha),
        );
        line("rustc", &self.facts.rustc);
        for (name, version) in &self.facts.dependencies {
            line(&format!("dependency {name}"), version);
        }
        line("bundled SQLite", &self.sqlite);
        line("Windows", &self.os);
        line("architecture", &self.arch);
        line(
            "viewer renderer",
            self.renderer.as_deref().unwrap_or("not recorded yet"),
        );
        line("data directory", &self.data_dir);
        line("data mode", &self.data_mode);
        line("settings", &self.settings);
        line("engine", &self.engine);
        line("watches", &self.watches);
        line("Defender real-time protection", &self.defender);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::{Diagnostics, Facts};

    fn diagnostics() -> Diagnostics {
        Diagnostics {
            facts: Facts {
                version: String::from("0.0.0"),
                git_sha: String::from("abcdef123456"),
                rustc: String::from("rustc 1.99.1"),
                dependencies: vec![(String::from("windows-sys"), String::from("0.61.2"))],
            },
            sqlite: String::from("not linked into this build yet"),
            os: String::from("10.0.26100.5039"),
            arch: String::from("x86_64 (native)"),
            renderer: None,
            data_dir: String::from(r"%LOCALAPPDATA%\nobuddy\PeekBuddy"),
            data_mode: String::from("installed"),
            settings: String::from("ok, 0 watch(es)"),
            engine: String::from("not running"),
            watches: String::from("engine not running"),
            defender: String::from("on"),
        }
    }

    #[test]
    fn the_report_has_one_line_per_fact() {
        let text = diagnostics().render();
        insta::assert_snapshot!(text);
    }

    #[test]
    fn a_recorded_renderer_is_shown() {
        let mut data = diagnostics();
        data.renderer = Some(String::from("wgpu DX12 / Intel(R) Graphics"));
        assert!(
            data.render()
                .contains("viewer renderer: wgpu DX12 / Intel(R) Graphics\n")
        );
    }

    #[test]
    fn dependencies_are_parsed_from_the_build_script_format() {
        assert_eq!(
            Facts::parse_dependencies("windows-sys=0.61.2,regex-automata=0.4.18"),
            [
                (String::from("windows-sys"), String::from("0.61.2")),
                (String::from("regex-automata"), String::from("0.4.18")),
            ]
        );
        assert_eq!(Facts::parse_dependencies("").len(), 0);
    }
}
