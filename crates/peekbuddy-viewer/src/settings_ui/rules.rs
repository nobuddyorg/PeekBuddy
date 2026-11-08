//! The rule editor of the settings screen (#81).
//!
//! Every field is text, so a wrong value can be typed and refused with the
//! key it belongs to (`rule[2].threshold`). The screen checks numbers before
//! it asks the engine; everything else (globs, kinds, watch ids, the program
//! path) is the engine's validator's to refuse, with the same words as for a
//! hand-edited file.

use peekbuddy_core::rules::{Command, Rule};

/// What a command asks the user to accept.
pub const COMMAND_WARNING: &str = "File names are chosen by whoever creates the files. They are \
     passed to the program as single arguments and never through a shell, but the program \
     decides what to do with them. Only start programs you trust with the names in your folders.";

/// A command, as text.
#[derive(Debug, Clone)]
pub(super) struct CommandForm {
    program: String,
    /// One argument per line, after the program.
    args: String,
    timeout: String,
    confirmed: bool,
}

impl Default for CommandForm {
    fn default() -> Self {
        Self {
            program: String::new(),
            args: String::new(),
            timeout: Command::default().timeout_s.to_string(),
            confirmed: false,
        }
    }
}

/// A rule, as text.
#[derive(Debug, Clone)]
pub(super) struct RuleForm {
    name: String,
    enabled: bool,
    /// Watch ids separated by commas.
    watches: String,
    glob: String,
    /// Kind names separated by commas.
    kinds: String,
    threshold: String,
    window: String,
    cooldown: String,
    toast: bool,
    command: Option<CommandForm>,
}

fn list(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

fn whole(text: &str) -> Option<u32> {
    text.trim().parse().ok()
}

impl RuleForm {
    /// A new rule with the defaults of the file.
    pub(super) fn blank() -> Self {
        Self::from_rule(&Rule {
            name: String::from("New rule"),
            ..Rule::default()
        })
    }

    pub(super) fn from_rule(rule: &Rule) -> Self {
        Self {
            name: rule.name.clone(),
            enabled: rule.enabled,
            watches: rule.watches.join(", "),
            glob: rule.glob.clone(),
            kinds: rule.kinds.join(", "),
            threshold: rule.threshold.to_string(),
            window: rule.window_s.to_string(),
            cooldown: rule.cooldown_s.to_string(),
            toast: rule.toast,
            command: rule.command.as_ref().map(|command| CommandForm {
                program: command.argv.first().cloned().unwrap_or_default(),
                args: command.argv.get(1..).unwrap_or_default().join("\n"),
                timeout: command.timeout_s.to_string(),
                confirmed: command.confirmed,
            }),
        }
    }

    /// The rule, or the keys of the numbers that are not whole numbers.
    pub(super) fn to_rule(&self) -> Result<Rule, Vec<&'static str>> {
        let threshold = whole(&self.threshold);
        let window = whole(&self.window);
        let cooldown = whole(&self.cooldown);
        let timeout = self
            .command
            .as_ref()
            .map_or(Some(Command::default().timeout_s), |c| whole(&c.timeout));
        let mut bad = Vec::new();
        for (key, ok) in [
            ("threshold", threshold.is_some()),
            ("window_s", window.is_some()),
            ("cooldown_s", cooldown.is_some()),
            ("command.timeout_s", timeout.is_some()),
        ] {
            if !ok {
                bad.push(key);
            }
        }
        let (Some(threshold), Some(window_s), Some(cooldown_s), Some(timeout_s)) =
            (threshold, window, cooldown, timeout)
        else {
            return Err(bad);
        };
        Ok(Rule {
            name: self.name.trim().to_owned(),
            enabled: self.enabled,
            watches: list(&self.watches),
            glob: self.glob.trim().to_owned(),
            kinds: list(&self.kinds),
            threshold,
            window_s,
            cooldown_s,
            toast: self.toast,
            command: self.command.as_ref().map(|command| {
                let mut argv = vec![command.program.trim().to_owned()];
                argv.extend(
                    command
                        .args
                        .lines()
                        .map(str::trim_end)
                        .filter(|line| !line.is_empty())
                        .map(str::to_owned),
                );
                Command {
                    argv,
                    timeout_s,
                    confirmed: command.confirmed,
                }
            }),
        })
    }
}

fn text(ui: &mut egui::Ui, number: usize, label: &str, value: &mut String, width: f32) {
    ui.horizontal(|ui| {
        let name = ui.label(format!("Rule {number}: {label}"));
        ui.add(egui::TextEdit::singleline(value).desired_width(width))
            .labelled_by(name.id);
    });
}

/// Draws one rule. Returns whether the user asked to remove it.
pub(super) fn show_rule(
    ui: &mut egui::Ui,
    index: usize,
    form: &mut RuleForm,
    problems: &[&str],
) -> bool {
    let number = index + 1;
    let mut remove = false;
    ui.group(|ui| {
        ui.horizontal(|ui| {
            ui.checkbox(&mut form.enabled, format!("Rule {number}: enabled"));
            if ui.button(format!("Remove rule {number}")).clicked() {
                remove = true;
            }
        });
        text(ui, number, "name", &mut form.name, 220.0);
        text(
            ui,
            number,
            "watch ids (empty: every watch)",
            &mut form.watches,
            200.0,
        );
        text(
            ui,
            number,
            "path pattern (empty: every path)",
            &mut form.glob,
            200.0,
        );
        text(
            ui,
            number,
            "kinds (empty: every kind)",
            &mut form.kinds,
            200.0,
        );
        ui.horizontal(|ui| {
            text(ui, number, "matches needed", &mut form.threshold, 50.0);
            text(ui, number, "within (s)", &mut form.window, 50.0);
            text(ui, number, "quiet for (s)", &mut form.cooldown, 60.0);
        });
        ui.checkbox(&mut form.toast, format!("Rule {number}: tell me"));
        let mut runs = form.command.is_some();
        if ui
            .checkbox(&mut runs, format!("Rule {number}: start a program"))
            .changed()
        {
            form.command = runs.then(CommandForm::default);
        }
        if let Some(command) = &mut form.command {
            ui.colored_label(ui.visuals().warn_fg_color, COMMAND_WARNING);
            text(
                ui,
                number,
                "program (full path)",
                &mut command.program,
                300.0,
            );
            ui.horizontal(|ui| {
                let name = ui.label(format!(
                    "Rule {number}: arguments, one per line (%path%, %kind%, %root%, %ts%)"
                ));
                ui.add(egui::TextEdit::multiline(&mut command.args).desired_rows(2))
                    .labelled_by(name.id);
            });
            text(ui, number, "stop it after (s)", &mut command.timeout, 60.0);
            ui.checkbox(
                &mut command.confirmed,
                format!("Rule {number}: I accept the risk above"),
            );
        }
        for problem in problems {
            ui.colored_label(ui.visuals().error_fg_color, *problem);
        }
    });
    remove
}
