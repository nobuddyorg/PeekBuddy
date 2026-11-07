//! The PeekBuddy viewer role: the on-demand egui window (`--viewer`).
//!
//! The window is a separate process so that the always-running engine never
//! loads a GPU stack (ADR 0001). The code splits along what each part needs:
//!
//! | Module | Needs | Builds and tests on |
//! | --- | --- | --- |
//! | [`link`] | nothing (a trait for the pipe) | every host |
//! | [`geometry`], [`timeline`], [`compact`] | nothing | every host |
//! | [`shell`], [`theme`] | `egui` (no window) | every host, under `egui_kittest` |
//! | `app`, `gpu`, `link_win` | `eframe`, Win32 | Windows |
//!
//! [`run`] is the only entry point; the `peekbuddy` bin calls it for
//! `--viewer`.
#![forbid(unsafe_code)]

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use peekbuddy_core::perf::MilestoneTable;

pub mod brand;
pub mod colwidth;
pub mod compact;
pub mod debounce;
pub mod details;
pub mod exporter;
pub mod filterspec;
pub mod follow;
pub mod geometry;
pub mod link;
pub mod onboarding;
pub mod rows;
pub mod selection;
pub mod settings_ui;
pub mod shell;
pub mod shellpath;
pub mod source;
pub mod theme;
pub mod timeline;
pub mod view;
pub mod watches;
pub mod worker;

#[cfg(test)]
mod layout_snapshots;
#[cfg(test)]
mod paint_dump;

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod gpu;
#[cfg(windows)]
mod link_win;

/// The title of the window of a viewer that belongs to an isolated engine
/// (`--data-dir`, tests and the perf harness): the hash of its directory
/// makes it findable, and keeps it apart from the user's own window.
#[must_use]
pub fn isolated_title(hash: u64) -> String {
    format!("PeekBuddy [{hash:016x}]")
}

/// `--viewer-probe <seconds>`: open, stay idle, report, close.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    /// Seconds to leave the window alone after it settled.
    pub seconds: u64,
    /// Write the report here instead of standard output.
    pub out: Option<PathBuf>,
}

/// Everything the viewer needs from the process that starts it.
#[derive(Debug, Clone)]
pub struct Options {
    /// The data directory (`settings.toml`, `logs\`).
    pub data_dir: PathBuf,
    /// The day segments (`catalog.db` lives here).
    pub segments: PathBuf,
    /// The engine's pipe.
    pub pipe_name: String,
    /// This exe, which is also the engine's.
    pub exe: PathBuf,
    /// The arguments that start the matching engine ("Start engine").
    pub engine_args: Vec<OsString>,
    /// Name of the event a second launch raises to bring this window up.
    pub show_event: String,
    /// The window title: `PeekBuddy`, or [`isolated_title`].
    pub title: String,
    /// The milestones table of this process (F7).
    pub table: Arc<MilestoneTable>,
    /// Measure instead of stay open.
    pub probe: Option<Probe>,
    /// What the About section shows about this build.
    pub about: settings_ui::About,
}

/// Runs the viewer window until it is closed.
///
/// # Errors
///
/// The window or the renderer could not start. The user has been told in a
/// message box, and the reason is in `logs\viewer.log`.
pub fn run(options: Options) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        app::run(options)
    }
    #[cfg(not(windows))]
    {
        drop(options);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use peekbuddy_core::perf::MilestoneTable;

    use super::{Options, run};

    #[test]
    fn an_isolated_viewer_has_a_title_of_its_own() {
        assert_eq!(super::isolated_title(0xAB), "PeekBuddy [00000000000000ab]");
        assert_ne!(super::isolated_title(1), super::isolated_title(2));
    }

    #[test]
    fn options_are_plain_data() {
        let options = Options {
            data_dir: "d".into(),
            segments: "d/segments".into(),
            pipe_name: String::from("pipe"),
            exe: "peekbuddy.exe".into(),
            engine_args: Vec::new(),
            show_event: String::from("event"),
            title: String::from("PeekBuddy"),
            table: Arc::new(MilestoneTable::new()),
            probe: None,
            about: super::settings_ui::About::default(),
        };
        assert_eq!(options.clone().pipe_name, "pipe");
        // Off Windows there is no window: the role returns at once.
        #[cfg(not(windows))]
        run(options).unwrap();
        #[cfg(windows)]
        let _ = run;
    }
}

/// Accessible-name enforcement for the kittest tests (#71): a control that a
/// screen reader would announce without a name fails the test that draws it.
#[cfg(test)]
pub(crate) mod a11y {
    use egui::accesskit::Role;
    use egui_kittest::Harness;
    use egui_kittest::kittest::NodeT as _;

    /// The roles of things a user can operate.
    const CONTROLS: &[Role] = &[
        Role::Button,
        Role::CheckBox,
        Role::ComboBox,
        Role::Link,
        Role::MenuItem,
        Role::RadioButton,
        Role::Slider,
        Role::SpinButton,
        Role::Switch,
        Role::Tab,
        Role::TextInput,
    ];

    /// Every operable node on screen, named or not: how many controls the
    /// check looked at.
    pub(crate) fn controls<S>(harness: &Harness<'_, S>) -> usize {
        harness
            .root()
            .children_recursive()
            .filter(|node| CONTROLS.contains(&node.accesskit_node().role()))
            .count()
    }

    /// Fails when a control on screen has no name, and when there is no
    /// control at all (a harness that drew nothing would pass otherwise).
    pub(crate) fn assert_every_control_is_named<S>(harness: &Harness<'_, S>) {
        let unnamed: Vec<String> = harness
            .root()
            .children_recursive()
            .filter(|node| CONTROLS.contains(&node.accesskit_node().role()))
            .filter(|node| {
                node.accesskit_node()
                    .label()
                    .is_none_or(|label| label.trim().is_empty())
            })
            .map(|node| format!("{node:?}"))
            .collect();
        assert_eq!(unnamed, Vec::<String>::new(), "controls without a name");
        assert!(controls(harness) > 0, "no control was drawn");
    }
}
