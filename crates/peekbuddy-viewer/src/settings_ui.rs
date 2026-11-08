//! The settings screen (#70).
//!
//! The screen edits a form and returns [`SettingsAction::Apply`]; the app
//! sends it to the engine, which validates, writes `settings.toml` atomically
//! and answers. The screen shows the engine's answer: a refusal appears as
//! text under the form and next to the field it names, and nothing changes.
//! The screen never writes the file itself. It knows nothing about eframe, so
//! it runs under `egui_kittest` on any host.

use std::collections::BTreeSet;

use peekbuddy_core::rules::Rule;
use peekbuddy_core::settings::{COALESCE_MS, Settings, Theme};

use self::rules::{RuleForm, show_rule};

mod rules;

/// What the About section shows besides the version.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct About {
    /// The package version.
    pub version: String,
    /// The git commit the exe was built from.
    pub git_sha: String,
    /// `x64` or `arm64`.
    pub target: String,
    /// The graphics adapter in use, once the first frame has been drawn.
    pub adapter: Option<String>,
}

/// The values the form can change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsEdit {
    /// Start with Windows.
    pub autostart: bool,
    /// The theme.
    pub theme: Theme,
    /// The coalescing window of the compact view.
    pub coalesce_ms: u32,
    /// Retention by age.
    pub max_age_days: Option<u32>,
    /// Retention by size.
    pub max_total_mb: Option<u64>,
    /// Retention by row count.
    pub max_rows: Option<u64>,
    /// The view presets in use.
    pub presets: Vec<String>,
    /// The rules that notify (#81).
    pub rules: Vec<Rule>,
}

impl SettingsEdit {
    /// Puts the values on `settings`.
    pub fn apply_to(&self, settings: &mut Settings) {
        settings.general.autostart = self.autostart;
        settings.general.theme = self.theme;
        settings.view.coalesce_ms = self.coalesce_ms;
        settings.store.max_age_days = self.max_age_days;
        settings.store.max_total_mb = self.max_total_mb;
        settings.store.max_rows = self.max_rows;
        settings.filters.presets.clone_from(&self.presets);
        settings.rules.clone_from(&self.rules);
    }
}

/// What the user asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsAction {
    /// Save these values through the engine.
    Apply(SettingsEdit),
    /// Open the data folder in Explorer.
    OpenDataFolder,
    /// Collect the diagnostics text for the preview.
    ShowDiagnostics,
    /// Show the first-run screens again.
    ShowOnboarding,
}

/// A view preset the screen offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetChoice {
    /// The id stored in `[filters] presets`.
    pub id: String,
    /// The title shown.
    pub title: String,
}

/// The presets that ship in the exe. Every one of them hides rows in the view
/// and records them anyway (the filter engine's `HideInView`, see
/// `docs/reference/filters.md`); none drops an event at capture.
#[must_use]
pub fn preset_choices() -> Vec<PresetChoice> {
    peekbuddy_core::filter::presets::all()
        .unwrap_or_default()
        .into_iter()
        .map(|preset| PresetChoice {
            id: preset.name,
            title: preset.title,
        })
        .collect()
}

/// The words after a preset's title.
pub const PRESET_NOTE: &str = "hidden in view (recorded)";

/// The form, as text so that a wrong value can be typed and refused.
#[derive(Debug, Clone, Default)]
struct Form {
    age: String,
    total: String,
    rows: String,
    coalesce: String,
    theme: Theme,
    autostart: bool,
    presets: BTreeSet<String>,
    rules: Vec<RuleForm>,
}

/// The settings screen.
#[derive(Debug, Default)]
pub struct SettingsScreen {
    form: Form,
    /// What the engine refused with, one problem per line.
    errors: Vec<String>,
    /// Problems found before asking the engine.
    local: Vec<(&'static str, String)>,
    /// Number fields of rules that are not numbers, by `rule[i].key`.
    local_rule: Vec<(String, String)>,
    status: Option<(bool, String)>,
    usage: Option<u64>,
    data_dir: String,
    about: About,
    choices: Vec<PresetChoice>,
    diagnostics: Option<String>,
    collecting: bool,
}

/// Bytes as megabytes with one decimal, for the disk usage line.
#[must_use]
pub fn megabytes(bytes: u64) -> String {
    // Integer arithmetic: tenths of a megabyte, rounded down.
    let tenths = bytes / 100_000;
    format!("{}.{} MB", tenths / 10, tenths % 10)
}

fn number<T: std::str::FromStr>(text: &str) -> Result<Option<T>, ()> {
    let text = text.trim();
    if text.is_empty() {
        Ok(None)
    } else {
        text.parse().map(Some).map_err(|_| ())
    }
}

impl SettingsScreen {
    /// A screen for the data folder `data_dir`.
    #[must_use]
    pub fn new(data_dir: String, about: About) -> Self {
        Self {
            data_dir,
            about,
            choices: preset_choices(),
            ..Self::default()
        }
    }

    /// Fills the form from the saved settings: when the tab opens and after a
    /// successful save.
    pub fn load(&mut self, settings: &Settings) {
        self.form = Form {
            age: settings
                .store
                .max_age_days
                .map_or_else(String::new, |v| v.to_string()),
            total: settings
                .store
                .max_total_mb
                .map_or_else(String::new, |v| v.to_string()),
            rows: settings
                .store
                .max_rows
                .map_or_else(String::new, |v| v.to_string()),
            coalesce: settings.view.coalesce_ms.to_string(),
            theme: settings.general.theme,
            autostart: settings.general.autostart,
            presets: settings.filters.presets.iter().cloned().collect(),
            rules: settings.rules.iter().map(RuleForm::from_rule).collect(),
        };
        self.errors.clear();
        self.local.clear();
        self.local_rule.clear();
    }

    /// The disk the segments use now, from the app.
    pub fn set_usage(&mut self, bytes: u64) {
        self.usage = Some(bytes);
    }

    /// The adapter line of the About section.
    pub fn set_adapter(&mut self, adapter: String) {
        self.about.adapter = Some(adapter);
    }

    /// The diagnostics text for the preview; `None` closes it.
    pub fn set_diagnostics(&mut self, text: Option<String>) {
        self.collecting = false;
        self.diagnostics = text;
    }

    /// The engine's answer to [`SettingsAction::Apply`]. A refusal keeps the
    /// form as typed so it can be corrected.
    pub fn on_result(&mut self, ok: bool, message: &str, saved: Option<&Settings>) {
        if ok {
            self.errors.clear();
            self.status = Some((true, message.to_owned()));
            if let Some(saved) = saved {
                self.load(saved);
            }
        } else {
            self.errors = message.lines().map(str::to_owned).collect();
            self.status = Some((false, String::from("Nothing was changed.")));
        }
    }

    /// The values typed, or what is wrong with them.
    fn edit(&mut self) -> Option<SettingsEdit> {
        self.local.clear();
        self.local_rule.clear();
        let age = number::<u32>(&self.form.age);
        let total = number::<u64>(&self.form.total);
        let rows = number::<u64>(&self.form.rows);
        let coalesce = number::<u32>(&self.form.coalesce);
        for (key, result_is_err) in [
            ("store.max_age_days", age.is_err()),
            ("store.max_total_mb", total.is_err()),
            ("store.max_rows", rows.is_err()),
            (
                "view.coalesce_ms",
                coalesce.is_err() || matches!(coalesce, Ok(None)),
            ),
        ] {
            if result_is_err {
                self.local.push((key, String::from("enter a whole number")));
            }
        }
        let mut rules = Vec::new();
        for (index, form) in self.form.rules.iter().enumerate() {
            match form.to_rule() {
                Ok(rule) => rules.push(rule),
                Err(keys) => {
                    for key in keys {
                        self.local_rule.push((
                            format!("rule[{index}].{key}"),
                            String::from("enter a whole number"),
                        ));
                    }
                }
            }
        }
        if !self.local.is_empty() || !self.local_rule.is_empty() {
            return None;
        }
        Some(SettingsEdit {
            autostart: self.form.autostart,
            theme: self.form.theme,
            coalesce_ms: coalesce.ok().flatten()?,
            max_age_days: age.ok()?,
            max_total_mb: total.ok()?,
            max_rows: rows.ok()?,
            presets: self.form.presets.iter().cloned().collect(),
            rules,
        })
    }

    /// What is wrong with the field `key`, from the form or the engine.
    fn problem(&self, key: &str) -> Option<&str> {
        self.local
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, text)| text.as_str())
            .or_else(|| {
                self.errors
                    .iter()
                    .find(|line| line.contains(key))
                    .map(String::as_str)
            })
    }

    fn field(&mut self, ui: &mut egui::Ui, label: &str, key: &str, which: Field, hint: &str) {
        ui.horizontal(|ui| {
            let name = ui.label(label);
            let text = match which {
                Field::Age => &mut self.form.age,
                Field::Total => &mut self.form.total,
                Field::Rows => &mut self.form.rows,
                Field::Coalesce => &mut self.form.coalesce,
            };
            let edit = ui.add(
                egui::TextEdit::singleline(text)
                    .id(egui::Id::unique(key))
                    .hint_text(hint)
                    .desired_width(90.0),
            );
            edit.labelled_by(name.id);
        });
        if let Some(problem) = self.problem(key) {
            ui.colored_label(ui.visuals().error_fg_color, problem);
        }
    }

    /// Draws the screen and returns what the user asked for.
    pub fn show(&mut self, ui: &mut egui::Ui) -> Vec<SettingsAction> {
        let mut actions = Vec::new();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                self.retention(ui);
                self.viewing(ui);
                self.notifications(ui);
                self.general(ui);
                if ui.button("Save").clicked() {
                    if let Some(edit) = self.edit() {
                        actions.push(SettingsAction::Apply(edit));
                    } else {
                        self.status = Some((false, String::from("Nothing was changed.")));
                    }
                }
                self.outcome(ui);
                ui.separator();
                self.data(ui, &mut actions);
                self.about_section(ui, &mut actions);
            });
        self.preview(ui);
        actions
    }

    fn retention(&mut self, ui: &mut egui::Ui) {
        ui.heading("Retention");
        ui.label(
            "The log keeps growing until a cap is reached; then the oldest segments are \
             deleted. Leave a field empty for no cap of that kind.",
        );
        if let Some(bytes) = self.usage {
            ui.label(format!("The log uses {} on disk now.", megabytes(bytes)));
        }
        self.field(
            ui,
            "Keep at most (days)",
            "store.max_age_days",
            Field::Age,
            "no cap",
        );
        self.field(
            ui,
            "Keep at most (MB)",
            "store.max_total_mb",
            Field::Total,
            "no cap",
        );
        self.field(
            ui,
            "Keep at most (rows)",
            "store.max_rows",
            Field::Rows,
            "no cap",
        );
        ui.label(
            "Lowering a cap deletes the oldest segments above it at the next retention pass. \
             Deleted history cannot be brought back.",
        );
    }

    fn viewing(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.heading("Viewing");
        self.field(
            ui,
            "Group rapid events of one file within (ms)",
            "view.coalesce_ms",
            Field::Coalesce,
            "150",
        );
        ui.label(format!(
            "From {} to {} ms. This only changes how the compact view groups rows; the \
             stored rows never change.",
            COALESCE_MS.start(),
            COALESCE_MS.end()
        ));
        ui.label("Presets that hide rows in the view:");
        for choice in &self.choices {
            let mut on = self.form.presets.contains(&choice.id);
            let label = format!("{} - {PRESET_NOTE}", choice.title);
            if ui.checkbox(&mut on, label).changed() {
                if on {
                    self.form.presets.insert(choice.id.clone());
                } else {
                    self.form.presets.remove(&choice.id);
                }
            }
        }
    }

    /// What is wrong with rule `index`, from the form or the engine.
    fn rule_problems(&self, index: usize) -> Vec<&str> {
        let prefix = format!("rule[{index}]");
        self.local_rule
            .iter()
            .filter(|(key, _)| key.starts_with(&prefix))
            .map(|(_, text)| text.as_str())
            .chain(
                self.errors
                    .iter()
                    .filter(|line| line.contains(&prefix))
                    .map(String::as_str),
            )
            .collect()
    }

    fn notifications(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.heading("Notifications");
        ui.label(
            "A rule tells you when a pattern shows up in the changes, for example three              deleted files in a folder within ten seconds. It shows a notification on this              PC and can start a program you choose. Nothing is sent anywhere.",
        );
        let mut remove = None;
        for index in 0..self.form.rules.len() {
            let problems: Vec<String> = self
                .rule_problems(index)
                .into_iter()
                .map(str::to_owned)
                .collect();
            let problems: Vec<&str> = problems.iter().map(String::as_str).collect();
            if let Some(form) = self.form.rules.get_mut(index)
                && show_rule(ui, index, form, &problems)
            {
                remove = Some(index);
            }
        }
        if let Some(index) = remove {
            self.form.rules.remove(index);
        }
        if ui.button("Add a rule").clicked() {
            self.form.rules.push(RuleForm::blank());
        }
    }

    fn general(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.heading("General");
        ui.horizontal(|ui| {
            ui.label("Theme");
            for (theme, name) in [
                (Theme::System, "System"),
                (Theme::Light, "Light"),
                (Theme::Dark, "Dark"),
            ] {
                ui.selectable_value(&mut self.form.theme, theme, name);
            }
        });
        ui.checkbox(
            &mut self.form.autostart,
            "Start PeekBuddy when I sign in to Windows",
        );
        ui.label(
            "Off by default. If Task Manager (Startup apps) has switched PeekBuddy off, \
             Windows keeps it off: switch it back on there, then save here again.",
        );
        ui.add_space(4.0);
    }

    fn outcome(&self, ui: &mut egui::Ui) {
        if let Some((ok, text)) = &self.status {
            let color = if *ok {
                ui.visuals().text_color()
            } else {
                ui.visuals().error_fg_color
            };
            ui.colored_label(color, if *ok { "Saved." } else { "Not saved." });
            ui.label(text);
        }
        // Problems that name a field of the form are shown next to it.
        for line in self.errors.iter().filter(|line| {
            !FIELD_KEYS.iter().any(|key| line.contains(key)) && !line.contains("rule[")
        }) {
            ui.colored_label(ui.visuals().error_fg_color, line);
        }
    }

    fn data(&self, ui: &mut egui::Ui, actions: &mut Vec<SettingsAction>) {
        ui.heading("Data");
        ui.label(
            "Everything PeekBuddy records stays in this folder, which cannot be moved. \
             Paths and names are in it; file contents never are.",
        );
        ui.horizontal(|ui| {
            ui.label(&self.data_dir);
            if ui.button("Open folder").clicked() {
                actions.push(SettingsAction::OpenDataFolder);
            }
        });
    }

    fn about_section(&mut self, ui: &mut egui::Ui, actions: &mut Vec<SettingsAction>) {
        ui.add_space(8.0);
        ui.heading("About");
        ui.horizontal(|ui| {
            crate::brand::wordmark(ui, 22.0);
            ui.label(&self.about.version);
        });
        ui.label(format!(
            "Commit {} - {} - {}",
            self.about.git_sha,
            self.about.target,
            self.about
                .adapter
                .as_deref()
                .unwrap_or("renderer not started yet")
        ));
        ui.label("PeekBuddy is free software under the MIT licence.");
        ui.label(
            "The licences of the libraries inside it are listed in THIRD_PARTY_LICENSES.html \
             in the release download.",
        );
        ui.label(crate::onboarding::PRIVACY_STATEMENT);
        ui.horizontal(|ui| {
            let label = if self.collecting {
                "Collecting diagnostics..."
            } else {
                "Copy diagnostics..."
            };
            if ui
                .add_enabled(!self.collecting, egui::Button::new(label))
                .clicked()
            {
                self.collecting = true;
                actions.push(SettingsAction::ShowDiagnostics);
            }
            if ui.button("Show the welcome screens again").clicked() {
                actions.push(SettingsAction::ShowOnboarding);
            }
        });
    }

    /// The diagnostics preview: it holds watched paths, so it is shown before
    /// anything is copied, and nothing leaves the screen by itself.
    fn preview(&mut self, ui: &mut egui::Ui) {
        let Some(text) = self.diagnostics.clone() else {
            return;
        };
        let mut close = false;
        egui::Window::new("Diagnostics")
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .default_size([560.0, 360.0])
            .show(ui.ctx(), |ui| {
                ui.label(
                    "This text contains the folders you watch and your user name. Read it, \
                     then copy it if you want to share it. PeekBuddy sends it nowhere.",
                );
                egui::ScrollArea::vertical()
                    .max_height(260.0)
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(egui::RichText::new(&text).monospace())
                                .selectable(true),
                        );
                    });
                ui.horizontal(|ui| {
                    if ui.button("Copy to clipboard").clicked() {
                        ui.ctx().copy_text(text.clone());
                    }
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                });
            });
        if close {
            self.diagnostics = None;
        }
    }
}

/// The settings keys the form has a field for.
const FIELD_KEYS: [&str; 4] = [
    "store.max_age_days",
    "store.max_total_mb",
    "store.max_rows",
    "view.coalesce_ms",
];

#[derive(Clone, Copy)]
enum Field {
    Age,
    Total,
    Rows,
    Coalesce,
}

#[cfg(test)]
mod tests;
