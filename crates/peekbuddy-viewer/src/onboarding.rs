//! The first-run screens (#70).
//!
//! Shown when the settings hold no watches and the first-run flag is still
//! set, and again on request from the settings. Five steps: welcome and the
//! privacy statement, folders (the same add flow as the Watches tab, with the
//! native picker), where the tray icon went, the autostart choice (unticked)
//! and done. Skipping at any step counts as done: the screens appear once.
//! Everything here is a plain button or checkbox, so it works by keyboard.

use std::collections::BTreeSet;

use peekbuddy_core::ipc::WatchInfo;

use crate::settings_ui::{PRESET_NOTE, PresetChoice, preset_choices};
use crate::watches::{WatchAction, WatchPanel, display_path};

/// The privacy statement. The README says the same in its own wording; a
/// test keeps the two from drifting apart.
pub const PRIVACY_STATEMENT: &str = "No login, no server, no tracking; the log never leaves your disk. PeekBuddy makes no \
     network connections: no telemetry, no update checks, no accounts.";

/// The sentence of the privacy statement the README and PRIVACY.md repeat
/// word for word.
pub const PRIVACY_HEADLINE: &str =
    "No login, no server, no tracking; the log never leaves your disk.";

/// Where the first-run screens stand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Step {
    /// Welcome and the privacy statement.
    #[default]
    Welcome,
    /// Choose folders and presets.
    Folders,
    /// Where the tray icon is.
    Tray,
    /// Start with Windows, or not.
    Autostart,
    /// All set.
    Done,
}

impl Step {
    const fn next(self) -> Self {
        match self {
            Self::Welcome => Self::Folders,
            Self::Folders => Self::Tray,
            Self::Tray => Self::Autostart,
            Self::Autostart | Self::Done => Self::Done,
        }
    }

    const fn back(self) -> Self {
        match self {
            Self::Welcome | Self::Folders => Self::Welcome,
            Self::Tray => Self::Folders,
            Self::Autostart => Self::Tray,
            Self::Done => Self::Autostart,
        }
    }

    /// The step's position, for "Step 2 of 5".
    #[must_use]
    pub const fn number(self) -> u8 {
        match self {
            Self::Welcome => 1,
            Self::Folders => 2,
            Self::Tray => 3,
            Self::Autostart => 4,
            Self::Done => 5,
        }
    }
}

/// What the screens ask the app to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OnboardingAction {
    /// A folder action, handled like the Watches tab's.
    Watch(WatchAction),
    /// Leave the screens and remember the choices.
    Finish {
        /// Start with Windows.
        autostart: bool,
        /// The view presets chosen.
        presets: Vec<String>,
    },
}

/// The first-run screens.
#[derive(Debug)]
pub struct Onboarding {
    /// Where the user is.
    pub step: Step,
    /// The autostart choice; unticked unless the user ticks it.
    pub autostart: bool,
    presets: BTreeSet<String>,
    choices: Vec<PresetChoice>,
    /// The add-folder flow, shared with the Watches tab.
    pub adder: WatchPanel,
}

impl Default for Onboarding {
    fn default() -> Self {
        Self {
            step: Step::default(),
            autostart: false,
            presets: BTreeSet::new(),
            choices: preset_choices(),
            adder: WatchPanel::default(),
        }
    }
}

impl Onboarding {
    /// The screens, reopened with the choices already made.
    #[must_use]
    pub fn reopened(autostart: bool, presets: &[String]) -> Self {
        Self {
            autostart,
            presets: presets.iter().cloned().collect(),
            ..Self::default()
        }
    }

    /// The choices to remember when the screens end: unchanged defaults mean
    /// no preset and no autostart.
    #[must_use]
    pub fn finish(&self) -> OnboardingAction {
        OnboardingAction::Finish {
            autostart: self.autostart,
            presets: self.presets.iter().cloned().collect(),
        }
    }

    /// Draws the current step. `watches` are the folders added so far.
    pub fn show(&mut self, ui: &mut egui::Ui, watches: &[WatchInfo]) -> Vec<OnboardingAction> {
        let mut actions = Vec::new();
        ui.horizontal(|ui| {
            crate::brand::wordmark(ui, 28.0);
            ui.label(format!("Step {} of 5", self.step.number()));
        });
        ui.separator();
        match self.step {
            Step::Welcome => Self::welcome(ui),
            Step::Folders => self.folders(ui, watches, &mut actions),
            Step::Tray => Self::tray(ui),
            Step::Autostart => self.autostart_step(ui),
            Step::Done => Self::done(ui, watches),
        }
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if self.step != Step::Welcome && ui.button("Back").clicked() {
                self.step = self.step.back();
            }
            if self.step == Step::Done {
                if ui.button("Open the log").clicked() {
                    actions.push(self.finish());
                }
            } else {
                if ui.button("Next").clicked() {
                    self.step = self.step.next();
                }
                if ui.button("Skip").clicked() {
                    actions.push(self.finish());
                }
            }
        });
        actions
    }

    fn welcome(ui: &mut egui::Ui) {
        ui.heading("Welcome to PeekBuddy");
        ui.label(
            "PeekBuddy quietly records every change in the folders you choose, so you can \
             see what happened and when.",
        );
        ui.add_space(6.0);
        ui.label(PRIVACY_STATEMENT);
        ui.label(
            "It stores paths, names, sizes, attributes and times of changes; never the \
             contents of your files.",
        );
    }

    fn folders(
        &mut self,
        ui: &mut egui::Ui,
        watches: &[WatchInfo],
        actions: &mut Vec<OnboardingAction>,
    ) {
        ui.heading("Which folders should PeekBuddy watch?");
        ui.label(
            "Pick the folders you care about. Watching a whole drive is possible but noisy; \
             you can change this later in the Watches tab.",
        );
        for action in self.adder.show_adder(ui) {
            actions.push(OnboardingAction::Watch(action));
        }
        for watch in watches {
            ui.label(format!("Watching {}", display_path(&watch.path)));
        }
        ui.add_space(6.0);
        ui.label("Rows to hide in the log (they are still recorded):");
        for choice in &self.choices {
            let mut on = self.presets.contains(&choice.id);
            if ui
                .checkbox(&mut on, format!("{} - {PRESET_NOTE}", choice.title))
                .changed()
            {
                if on {
                    self.presets.insert(choice.id.clone());
                } else {
                    self.presets.remove(&choice.id);
                }
            }
        }
    }

    fn tray(ui: &mut egui::Ui) {
        ui.heading("Where is PeekBuddy when this window is closed?");
        ui.label(
            "PeekBuddy keeps recording in the background and shows an icon in the notification \
             area at the right end of the taskbar. Windows may put a new icon in the overflow \
             flyout (the small arrow next to the clock). Drag the PeekBuddy icon from the \
             flyout onto the taskbar to keep it in view.",
        );
        ui.label(
            "Click the icon to open this window again. Right-click it to pause everything or \
             to quit.",
        );
        ui.label(
            "If you cannot find it, Windows lists the icons under Settings, Personalization, \
             Taskbar (the wording differs a little between Windows 10 and 11).",
        );
    }

    fn autostart_step(&mut self, ui: &mut egui::Ui) {
        ui.heading("Start with Windows?");
        ui.label(
            "PeekBuddy can start when you sign in so that no change is missed while you work. \
             This is off unless you switch it on, here or later in the settings.",
        );
        ui.checkbox(
            &mut self.autostart,
            "Start PeekBuddy when I sign in to Windows",
        );
    }

    fn done(ui: &mut egui::Ui, watches: &[WatchInfo]) {
        ui.heading("You are all set");
        if watches.is_empty() {
            ui.label("No folder is watched yet. Add one in the Watches tab whenever you like.");
        } else {
            ui.label(format!(
                "{} folder{} will be recorded once PeekBuddy's engine has started with them.",
                watches.len(),
                if watches.len() == 1 { "" } else { "s" }
            ));
        }
        ui.label("The log opens next; new changes appear at the top as they happen.");
    }
}

#[cfg(test)]
mod tests;
