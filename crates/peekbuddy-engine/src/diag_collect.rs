//! Gathers the facts of `--diagnostics` on Windows.

use std::path::Path;

use peekbuddy_win::metrics::{Arch, arch, current_process};
use peekbuddy_win::osinfo;
use peekbuddy_win::single_instance::SingleInstance;

use crate::diagnostics::{Diagnostics, Facts};
use crate::portable::Decision;
use crate::{Names, datadir};

/// File in the data directory where the viewer records its renderer and
/// adapter (#65): one line, `<renderer> / <adapter>`.
pub const RENDERER_FILE: &str = "renderer.txt";

fn redact(root: &Path, decision: Decision) -> String {
    if matches!(decision, Decision::Portable) {
        return String::from(r"<exe folder>\data");
    }
    let local = known_folders::get_known_folder_path(known_folders::KnownFolder::LocalAppData);
    match local.and_then(|local| root.strip_prefix(local).ok().map(Path::to_path_buf)) {
        Some(rest) => format!(r"%LOCALAPPDATA%\{}", rest.display()),
        None => String::from("<custom location>"),
    }
}

fn describe(decision: Decision) -> String {
    match decision {
        Decision::Portable => String::from("portable"),
        Decision::Installed(None) => String::from("installed"),
        Decision::Installed(Some(why)) => format!("installed ({})", why.describe()),
    }
}

/// Collects everything `--diagnostics` prints. `full` shows the real data
/// directory instead of a redacted one.
#[must_use]
pub fn collect(facts: Facts, full: bool) -> Diagnostics {
    let located = std::env::current_exe()
        .map_err(|error| error.to_string())
        .and_then(|exe| datadir::locate(&exe).map_err(|error| error.to_string()));
    let (data_dir, data_mode, renderer) = match located {
        Ok((root, decision)) => {
            let shown = if full {
                root.display().to_string()
            } else {
                redact(&root, decision)
            };
            let renderer = std::fs::read_to_string(root.join(RENDERER_FILE))
                .ok()
                .and_then(|text| text.lines().next().map(str::to_owned));
            (shown, describe(decision), renderer)
        }
        Err(error) => (format!("unknown ({error})"), String::from("unknown"), None),
    };
    let running = matches!(SingleInstance::acquire(&Names::new(None).mutex), Ok(None));
    let arch_text = arch(current_process()).map_or_else(
        |_| String::from("unknown"),
        |arch| {
            format!(
                "{} ({}; machine {})",
                Arch::name(arch.process.max(arch.native)),
                if arch.emulated() {
                    "emulated"
                } else {
                    "native"
                },
                Arch::name(arch.native),
            )
        },
    );
    Diagnostics {
        facts,
        sqlite: String::from("not linked into this build yet (#52)"),
        os: osinfo::update_build_revision().map_or_else(osinfo::os_build, |ubr| {
            format!("{}.{ubr}", osinfo::os_build())
        }),
        arch: arch_text,
        renderer,
        data_dir,
        data_mode,
        engine: String::from(if running { "running" } else { "not running" }),
        watches: String::from(if running {
            "unavailable: the engine pipe does not exist yet (#61)"
        } else {
            "engine not running"
        }),
        defender: match osinfo::defender_realtime_enabled() {
            Some(true) => String::from("on"),
            Some(false) => String::from("off"),
            None => String::from("unknown"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::collect;
    use crate::diagnostics::Facts;

    #[test]
    fn the_default_output_has_no_absolute_user_path() {
        let text = collect(Facts::default(), false).render();
        let user = std::env::var("USERPROFILE").unwrap();
        assert!(!text.contains(&user), "{text}");
        assert!(!text.to_lowercase().contains(r"c:\users"), "{text}");
    }

    #[test]
    fn full_shows_the_real_data_directory() {
        let text = collect(Facts::default(), true).render();
        assert!(
            text.to_lowercase().contains(r"\nobuddy\peekbuddy"),
            "{text}"
        );
    }

    #[test]
    fn the_facts_come_from_this_machine() {
        let text = collect(Facts::default(), false).render();
        assert!(text.contains("Windows: 10.0."), "{text}");
        assert!(text.contains("architecture: "), "{text}");
        assert!(text.contains("data mode: installed"), "{text}");
    }
}
