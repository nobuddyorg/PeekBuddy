//! The watch list and its management (#68).
//!
//! The panel draws what the engine reported ([`WatchInfo`]) and returns what
//! the user asked for ([`WatchAction`]); the app sends it. It knows nothing
//! about eframe, so it runs under `egui_kittest` on any host. Every state of
//! the watch lifecycle (F19) has a label, a painted icon and a tooltip that
//! says what it means and what PeekBuddy does next, so no state is told by
//! colour alone.

use peekbuddy_core::event::WatchState;
use peekbuddy_core::ipc::WatchInfo;

/// Which screen is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    /// The event log.
    #[default]
    Log,
    /// The watches.
    Watches,
    /// The settings and About.
    Settings,
}

/// The shape in front of a state's label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Icon {
    /// Three dots: getting ready.
    Dots,
    /// A filled dot: recording.
    Dot,
    /// A triangle: attention, something was missed.
    Triangle,
    /// Two arrows in a circle: scanning.
    Cycle,
    /// Two bars: paused.
    Pause,
    /// A padlock: held by the system.
    Lock,
    /// A cross: lost.
    Cross,
    /// A check mark: back again.
    Check,
    /// A clock: waits for the next engine start.
    Clock,
}

/// How a watch is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Presentation {
    /// The state as a word.
    pub label: &'static str,
    /// The shape.
    pub icon: Icon,
    /// What it means and what PeekBuddy does next.
    pub tooltip: &'static str,
}

/// The presentation of a lifecycle state of an armed watch.
#[must_use]
pub const fn present_state(state: WatchState) -> Presentation {
    match state {
        WatchState::Arming => Presentation {
            label: "Arming",
            icon: Icon::Dots,
            tooltip: "PeekBuddy is opening this folder. It becomes Live as soon as changes \
                      are being received.",
        },
        WatchState::Live => Presentation {
            label: "Live",
            icon: Icon::Dot,
            tooltip: "Changes in this folder are being recorded.",
        },
        WatchState::Overflowed => Presentation {
            label: "Overflowed",
            icon: Icon::Triangle,
            tooltip: "Windows reported more changes than PeekBuddy's buffer could hold. A gap \
                      is recorded and a scan of the folder follows to find what was missed.",
        },
        WatchState::Reconciling => Presentation {
            label: "Reconciling",
            icon: Icon::Cycle,
            tooltip: "PeekBuddy is scanning the folder to find what changed during a gap. \
                      Rows found by the scan are marked as reconcile results.",
        },
        WatchState::SuspendedByUser => Presentation {
            label: "Paused",
            icon: Icon::Pause,
            tooltip: "Paused by you. The watch stays armed, changes are counted but not \
                      recorded. Resume to record again.",
        },
        WatchState::SuspendedQueryRemove => Presentation {
            label: "Suspended (drive removal)",
            icon: Icon::Lock,
            tooltip: "The drive is being removed. PeekBuddy let go of the folder and watches \
                      it again when the drive is back.",
        },
        WatchState::SuspendedLocked => Presentation {
            label: "Suspended (drive locked)",
            icon: Icon::Lock,
            tooltip: "The drive is locked or being checked. PeekBuddy waits and arms the watch \
                      again when the drive is available.",
        },
        WatchState::Lost => Presentation {
            label: "Lost",
            icon: Icon::Cross,
            tooltip: "The folder is gone or cannot be reached. PeekBuddy keeps trying and \
                      records when it is back.",
        },
        WatchState::Restored => Presentation {
            label: "Restored",
            icon: Icon::Check,
            tooltip: "The folder is back. PeekBuddy scans it to find what changed while it \
                      was gone; then the watch is Live again.",
        },
    }
}

/// The presentation of the watch `info`: its state, or "waiting" for a
/// watch the engine has saved but not armed.
#[must_use]
pub const fn present(info: &WatchInfo) -> Presentation {
    if info.armed {
        present_state(info.state)
    } else {
        Presentation {
            label: "Starts after restart",
            icon: Icon::Clock,
            tooltip: "Saved. This folder is recorded once PeekBuddy's engine has restarted.",
        }
    }
}

/// A path for people: the verbatim prefix is not shown.
#[must_use]
pub fn display_path(stored: &str) -> String {
    if let Some(rest) = stored.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else {
        stored.strip_prefix(r"\\?\").unwrap_or(stored).to_owned()
    }
}

/// What the user asked for in the watches screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchAction {
    /// Add a watch on `path`.
    Add {
        /// The path as typed or picked.
        path: String,
        /// The user confirmed the engine's warning.
        confirmed: bool,
    },
    /// Remove the watch `id`.
    Remove(String),
    /// Pause or resume one watch.
    SetPaused {
        /// The watch.
        id: String,
        /// The new state.
        paused: bool,
    },
    /// Open the folder picker.
    Browse,
    /// Restart the engine so saved watches start.
    RestartEngine,
}

/// A question the user has to answer before something happens.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Ask {
    /// A whole drive, or whatever else the engine wants confirmed.
    Confirm { path: String, text: String },
    /// Remove this watch?
    Remove { id: String, path: String },
}

/// The watches screen.
#[derive(Debug, Default)]
pub struct WatchPanel {
    /// What the engine last reported.
    pub watches: Vec<WatchInfo>,
    /// The path being typed.
    input: String,
    /// The path of the add the engine has not answered yet.
    last_add: Option<String>,
    /// The outcome of the last command: whether it worked, and what the
    /// engine said.
    status: Option<(bool, String)>,
    /// The open question.
    ask: Option<Ask>,
    /// The engine is restarting.
    pub restarting: bool,
}

impl WatchPanel {
    /// Takes a fresh list from the engine.
    pub fn set_watches(&mut self, watches: Vec<WatchInfo>) {
        self.watches = watches;
    }

    /// How many watches need a look: lost, overflowed or not recording.
    #[must_use]
    pub fn attention(&self) -> usize {
        self.watches
            .iter()
            .filter(|watch| {
                watch.armed
                    && matches!(
                        watch.state,
                        WatchState::Lost | WatchState::Overflowed | WatchState::SuspendedLocked
                    )
            })
            .count()
    }

    /// How many saved watches wait for an engine start.
    #[must_use]
    pub fn waiting(&self) -> usize {
        self.watches.iter().filter(|watch| !watch.armed).count()
    }

    /// Shows the engine's answer to the last add or remove.
    pub fn on_result(&mut self, ok: bool, message: &str) {
        if let (false, Some(text), Some(path)) =
            (ok, message.strip_prefix("confirm: "), self.last_add.take())
        {
            self.ask = Some(Ask::Confirm {
                path,
                text: text.to_owned(),
            });
            self.status = None;
            return;
        }
        if ok && message.starts_with("added") {
            self.input.clear();
        }
        self.last_add = None;
        self.status = Some((ok, message.to_owned()));
    }

    /// Adds a path the folder picker returned; the engine decides.
    #[must_use]
    pub fn picked(&mut self, path: String) -> WatchAction {
        self.last_add = Some(path.clone());
        WatchAction::Add {
            path,
            confirmed: false,
        }
    }

    /// Draws only the part that adds folders (the path field, Browse, the
    /// engine's answer and the question about a whole drive): the first-run
    /// screen uses it.
    pub fn show_adder(&mut self, ui: &mut egui::Ui) -> Vec<WatchAction> {
        let mut actions = Vec::new();
        self.add_row(ui, &mut actions);
        self.result_line(ui);
        self.question(ui, &mut actions);
        actions
    }

    fn result_line(&self, ui: &mut egui::Ui) {
        if let Some((ok, text)) = &self.status {
            ui.horizontal_wrapped(|ui| {
                let color = if *ok {
                    ui.visuals().text_color()
                } else {
                    ui.visuals().error_fg_color
                };
                ui.colored_label(color, if *ok { "Done:" } else { "Not done:" });
                ui.label(text);
            });
        }
    }

    /// Draws the screen and returns what the user asked for.
    pub fn show(&mut self, ui: &mut egui::Ui) -> Vec<WatchAction> {
        let mut actions = Vec::new();
        self.add_row(ui, &mut actions);
        self.result_line(ui);
        let waiting = self.waiting();
        if waiting > 0 && !self.restarting {
            ui.horizontal_wrapped(|ui| {
                ui.label(format!(
                    "{waiting} saved folder{} start{} recording when PeekBuddy's engine restarts.",
                    if waiting == 1 { "" } else { "s" },
                    if waiting == 1 { "s" } else { "" },
                ));
                if ui.button("Restart engine now").clicked() {
                    actions.push(WatchAction::RestartEngine);
                }
            });
        }
        if self.restarting {
            ui.label("Restarting the engine...");
        }
        ui.separator();
        self.list(ui, &mut actions);
        self.question(ui, &mut actions);
        actions
    }

    fn add_row(&mut self, ui: &mut egui::Ui, actions: &mut Vec<WatchAction>) {
        ui.horizontal(|ui| {
            let label = ui.label("Watch a folder");
            let edit = ui.add(
                egui::TextEdit::singleline(&mut self.input)
                    .id(egui::Id::unique("watch-path"))
                    .hint_text(r"C:\Users\you\Documents")
                    .desired_width(360.0),
            );
            let edit = edit.labelled_by(label.id);
            let enter = edit.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            let can_add = !self.input.trim().is_empty();
            if (ui.add_enabled(can_add, egui::Button::new("Add")).clicked() || (enter && can_add))
                && self.ask.is_none()
            {
                let path = self.input.trim().to_owned();
                self.last_add = Some(path.clone());
                actions.push(WatchAction::Add {
                    path,
                    confirmed: false,
                });
            }
            if ui.button("Browse...").clicked() {
                actions.push(WatchAction::Browse);
            }
        });
    }

    fn list(&mut self, ui: &mut egui::Ui, actions: &mut Vec<WatchAction>) {
        if self.watches.is_empty() {
            ui.label(
                "Nothing is watched yet. Add a folder above; PeekBuddy records every change \
                 inside it.",
            );
            return;
        }
        egui::ScrollArea::vertical()
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for watch in &self.watches {
                    let shown = present(watch);
                    ui.horizontal(|ui| {
                        let mark = paint_icon(ui, shown.icon, 14.0);
                        mark.on_hover_text(shown.tooltip);
                        ui.label(shown.label).on_hover_text(shown.tooltip);
                        ui.label(display_path(&watch.path));
                        if !watch.recursive {
                            ui.label("(this folder only)");
                        }
                        if watch.armed
                            && watch.state == WatchState::SuspendedByUser
                            && watch.missed_while_paused > 0
                        {
                            ui.label(format!(
                                "{} changes missed while paused",
                                crate::view::group_digits(watch.missed_while_paused)
                            ));
                        }
                        let shown_path = display_path(&watch.path);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            // Every row has the same words on its buttons; a
                            // screen reader hears which folder each is for.
                            let remove = ui.button("Remove");
                            remove.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::Role::Button,
                                    true,
                                    format!("Remove the watch on {shown_path}"),
                                )
                            });
                            if remove.clicked() {
                                self.ask = Some(Ask::Remove {
                                    id: watch.id.clone(),
                                    path: shown_path.clone(),
                                });
                            }
                            if watch.armed {
                                let paused = watch.state == WatchState::SuspendedByUser;
                                let label = if paused { "Resume" } else { "Pause" };
                                let toggle = ui.button(label);
                                toggle.widget_info(|| {
                                    egui::WidgetInfo::labeled(
                                        egui::Role::Button,
                                        true,
                                        format!("{label} the watch on {shown_path}"),
                                    )
                                });
                                if toggle.clicked() {
                                    actions.push(WatchAction::SetPaused {
                                        id: watch.id.clone(),
                                        paused: !paused,
                                    });
                                }
                            }
                        });
                    });
                }
            });
    }

    fn question(&mut self, ui: &mut egui::Ui, actions: &mut Vec<WatchAction>) {
        let Some(ask) = self.ask.clone() else {
            return;
        };
        let mut close = false;
        let (title, text, yes, path_or_id) = match &ask {
            Ask::Confirm { path, text } => ("Watch this folder?", text.clone(), "Add anyway", path),
            Ask::Remove { path, .. } => (
                "Remove this watch?",
                format!(
                    "{path} is no longer watched. The changes already recorded stay in the log."
                ),
                "Remove",
                path,
            ),
        };
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ui.ctx(), |ui| {
                ui.set_max_width(420.0);
                ui.label(path_or_id);
                ui.label(text);
                ui.horizontal(|ui| {
                    if ui.button(yes).clicked() {
                        match &ask {
                            Ask::Confirm { path, .. } => {
                                self.last_add = Some(path.clone());
                                actions.push(WatchAction::Add {
                                    path: path.clone(),
                                    confirmed: true,
                                });
                            }
                            Ask::Remove { id, .. } => {
                                actions.push(WatchAction::Remove(id.clone()));
                            }
                        }
                        close = true;
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                });
            });
        if close || ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.ask = None;
        }
    }
}

/// Draws the tab strip above the content. `attention` is shown next to
/// "Watches" when something needs a look.
pub fn tabs(ui: &mut egui::Ui, current: &mut Tab, watches: usize, attention: usize) {
    ui.horizontal(|ui| {
        ui.selectable_value(current, Tab::Log, "Log");
        let label = if attention > 0 {
            format!("Watches ({watches}, {attention} need attention)")
        } else {
            format!("Watches ({watches})")
        };
        ui.selectable_value(current, Tab::Watches, label);
        ui.selectable_value(current, Tab::Settings, "Settings");
    });
    ui.separator();
}

/// Paints `icon` in a square of `size` points.
fn paint_icon(ui: &mut egui::Ui, icon: Icon, size: f32) -> egui::Response {
    let (response, painter) = ui.allocate_painter(egui::vec2(size, size), egui::Sense::hover());
    let visuals = ui.visuals();
    let color = match icon {
        Icon::Cross => visuals.error_fg_color,
        Icon::Triangle | Icon::Lock => visuals.warn_fg_color,
        _ => visuals.text_color(),
    };
    let stroke = egui::Stroke::new(1.5, color);
    let rect = response.rect.shrink(1.0);
    let at = |x: f32, y: f32| {
        egui::pos2(
            rect.left() + rect.width() * x,
            rect.top() + rect.height() * y,
        )
    };
    match icon {
        Icon::Dots => {
            for x in [0.2, 0.5, 0.8] {
                painter.circle_filled(at(x, 0.5), 1.5, color);
            }
        }
        Icon::Dot => {
            painter.circle_filled(at(0.5, 0.5), rect.width() * 0.3, color);
        }
        Icon::Triangle => {
            let points = vec![at(0.5, 0.05), at(0.95, 0.9), at(0.05, 0.9)];
            painter.add(egui::Shape::closed_line(points, stroke));
            painter.line_segment([at(0.5, 0.35), at(0.5, 0.62)], stroke);
            painter.circle_filled(at(0.5, 0.78), 1.0, color);
        }
        Icon::Cycle => {
            painter.circle_stroke(at(0.5, 0.5), rect.width() * 0.4, stroke);
            painter.line_segment([at(0.5, 0.1), at(0.72, 0.1)], stroke);
            painter.line_segment([at(0.5, 0.9), at(0.28, 0.9)], stroke);
        }
        Icon::Pause => {
            painter.line_segment([at(0.3, 0.15), at(0.3, 0.85)], stroke);
            painter.line_segment([at(0.7, 0.15), at(0.7, 0.85)], stroke);
        }
        Icon::Lock => {
            painter.rect_stroke(
                egui::Rect::from_min_max(at(0.2, 0.45), at(0.8, 0.9)),
                1.0,
                stroke,
                egui::StrokeKind::Inside,
            );
            painter.line_segment([at(0.32, 0.45), at(0.32, 0.25)], stroke);
            painter.line_segment([at(0.68, 0.45), at(0.68, 0.25)], stroke);
            painter.line_segment([at(0.32, 0.2), at(0.68, 0.2)], stroke);
        }
        Icon::Cross => {
            painter.line_segment([at(0.15, 0.15), at(0.85, 0.85)], stroke);
            painter.line_segment([at(0.85, 0.15), at(0.15, 0.85)], stroke);
        }
        Icon::Check => {
            painter.line_segment([at(0.1, 0.55), at(0.4, 0.85)], stroke);
            painter.line_segment([at(0.4, 0.85), at(0.9, 0.15)], stroke);
        }
        Icon::Clock => {
            painter.circle_stroke(at(0.5, 0.5), rect.width() * 0.45, stroke);
            painter.line_segment([at(0.5, 0.5), at(0.5, 0.2)], stroke);
            painter.line_segment([at(0.5, 0.5), at(0.7, 0.6)], stroke);
        }
    }
    response
}

#[cfg(test)]
mod mapping_tests;

#[cfg(test)]
mod tests;
