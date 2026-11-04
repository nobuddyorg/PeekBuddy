//! The window's frame (#65): top bar, engine banner and status line.
//!
//! The shell knows nothing about eframe, so it runs under `egui_kittest` on
//! any host. It draws from state and returns at most one [`ShellAction`] per
//! frame; it never allocates a string per frame for text that does not change.

use peekbuddy_core::ipc::PROTOCOL_VERSION;

use crate::link::LinkState;

/// What the user asked for in the shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellAction {
    /// Start the engine (it is not running).
    StartEngine,
    /// Look for the engine again (after restarting PeekBuddy).
    Reconnect,
}

/// What the shell shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shell {
    /// The state of the engine link.
    pub link: LinkState,
}

impl Default for Shell {
    fn default() -> Self {
        Self {
            link: LinkState::Connecting,
        }
    }
}

/// The text of the banner for a state, when it has one.
#[must_use]
pub fn banner_text(state: &LinkState) -> Option<String> {
    match state {
        LinkState::Connecting | LinkState::Connected { .. } => None,
        LinkState::Gone => Some(String::from(
            "PeekBuddy's engine is not running, so no new changes are recorded. \
             You can still browse the log.",
        )),
        LinkState::Mismatch { engine_version } => Some(format!(
            "The running PeekBuddy engine is a different version (protocol {engine_version}, \
             this viewer speaks {PROTOCOL_VERSION}). Quit PeekBuddy from its tray icon and \
             start it again."
        )),
    }
}

/// The short status for the status line.
#[must_use]
pub const fn status_text(state: &LinkState) -> &'static str {
    match state {
        LinkState::Connecting => "Connecting to the engine...",
        LinkState::Connected { .. } => "Engine running",
        LinkState::Gone => "Engine not running",
        LinkState::Mismatch { .. } => "Engine version differs",
    }
}

/// The shape in front of the status text. Text always follows the shape, and
/// every state has its own shape, so colour is never the only signal. Shapes
/// are painted, not typed: the bundled fonts have no check or cross glyph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StatusMark {
    /// Three dots: working on it.
    Dots,
    /// A check mark: all well.
    Check,
    /// A cross: not running.
    Cross,
    /// A triangle: attention.
    Triangle,
}

/// The mark for a state.
#[must_use]
pub const fn status_mark(state: &LinkState) -> StatusMark {
    match state {
        LinkState::Connecting => StatusMark::Dots,
        LinkState::Connected { .. } => StatusMark::Check,
        LinkState::Gone => StatusMark::Cross,
        LinkState::Mismatch { .. } => StatusMark::Triangle,
    }
}

/// Paints `mark` in a square of `size` points and returns its response.
fn paint_mark(ui: &mut egui::Ui, mark: StatusMark, size: f32) -> egui::Response {
    let (response, painter) = ui.allocate_painter(egui::vec2(size, size), egui::Sense::hover());
    let visuals = ui.visuals();
    let color = match mark {
        StatusMark::Dots | StatusMark::Check => visuals.text_color(),
        StatusMark::Cross => visuals.error_fg_color,
        StatusMark::Triangle => visuals.warn_fg_color,
    };
    let stroke = egui::Stroke::new(1.5, color);
    let rect = response.rect.shrink(1.0);
    let at = |x: f32, y: f32| {
        egui::pos2(
            rect.left() + rect.width() * x,
            rect.top() + rect.height() * y,
        )
    };
    match mark {
        StatusMark::Dots => {
            for x in [0.2, 0.5, 0.8] {
                painter.circle_filled(at(x, 0.5), 1.5, color);
            }
        }
        StatusMark::Check => {
            painter.line_segment([at(0.1, 0.55), at(0.4, 0.85)], stroke);
            painter.line_segment([at(0.4, 0.85), at(0.9, 0.15)], stroke);
        }
        StatusMark::Cross => {
            painter.line_segment([at(0.15, 0.15), at(0.85, 0.85)], stroke);
            painter.line_segment([at(0.85, 0.15), at(0.15, 0.85)], stroke);
        }
        StatusMark::Triangle => {
            let points = vec![at(0.5, 0.05), at(0.95, 0.9), at(0.05, 0.9)];
            painter.add(egui::Shape::closed_line(points, stroke));
            painter.line_segment([at(0.5, 0.35), at(0.5, 0.62)], stroke);
            painter.circle_filled(at(0.5, 0.78), 1.0, color);
        }
    }
    response
}

impl Shell {
    /// Draws the top bar, the banner and the status line around `content`,
    /// which fills the rest.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        content: impl FnOnce(&mut egui::Ui),
    ) -> Option<ShellAction> {
        let mut action = None;
        egui::Panel::top("top-bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                crate::brand::wordmark(ui, 22.0);
            });
        });
        egui::Panel::bottom("status-line").show(ui, |ui| {
            ui.horizontal(|ui| {
                paint_mark(ui, status_mark(&self.link), 14.0);
                ui.label(status_text(&self.link));
            });
        });
        if let Some(text) = banner_text(&self.link) {
            egui::Panel::top("engine-banner").show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(text);
                    let (label, wanted) = match self.link {
                        LinkState::Mismatch { .. } => ("Check again", ShellAction::Reconnect),
                        _ => ("Start engine", ShellAction::StartEngine),
                    };
                    if ui.button(label).clicked() {
                        action = Some(wanted);
                    }
                });
            });
        }
        egui::CentralPanel::default().show(ui, content);
        action
    }
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable as _;

    use crate::link::LinkState;

    use super::{Shell, ShellAction, banner_text, status_mark, status_text};

    fn harness(link: LinkState) -> Harness<'static, (Shell, Option<ShellAction>)> {
        Harness::new_ui_state(
            |ui, (shell, taken)| {
                if let Some(action) = shell.show(ui, |ui| {
                    ui.label("content");
                }) {
                    *taken = Some(action);
                }
            },
            (Shell { link }, None),
        )
    }

    #[test]
    fn a_running_engine_has_no_banner_and_says_so_in_the_status_line() {
        let mut harness = harness(LinkState::Connected {
            build: String::from("peekbuddy 0.0.0"),
        });
        harness.run();
        assert!(harness.query_by_label_contains("not running").is_none());
        harness.get_by_label("Engine running");
        harness.get_by_label("content");
    }

    #[test]
    fn a_missing_engine_offers_to_start_it() {
        let mut harness = harness(LinkState::Gone);
        harness.run();
        harness.get_by_label_contains("engine is not running");
        harness.get_by_label("Engine not running");
        harness.get_by_label("Start engine").click();
        harness.run();
        assert_eq!(harness.state().1, Some(ShellAction::StartEngine));
    }

    #[test]
    fn every_control_of_the_banner_has_a_name() {
        for link in [LinkState::Gone, LinkState::Mismatch { engine_version: 3 }] {
            let mut harness = harness(link);
            harness.run();
            crate::a11y::assert_every_control_is_named(&harness);
        }
    }

    #[test]
    fn a_version_mismatch_explains_itself_and_offers_to_check_again() {
        let mut harness = harness(LinkState::Mismatch { engine_version: 9 });
        harness.run();
        harness.get_by_label_contains("different version (protocol 9");
        harness.get_by_label("Check again").click();
        harness.run();
        assert_eq!(harness.state().1, Some(ShellAction::Reconnect));
    }

    #[test]
    fn every_state_has_a_text_and_a_mark_and_only_two_have_a_banner() {
        let states = [
            LinkState::Connecting,
            LinkState::Connected {
                build: String::new(),
            },
            LinkState::Gone,
            LinkState::Mismatch { engine_version: 2 },
        ];
        let marks: std::collections::HashSet<_> = states.iter().map(status_mark).collect();
        let texts: std::collections::HashSet<_> = states.iter().map(status_text).collect();
        assert_eq!(marks.len(), 4, "state is never told by colour alone");
        assert_eq!(texts.len(), 4);
        assert_eq!(
            states.iter().filter(|s| banner_text(s).is_some()).count(),
            2
        );
    }
}
