//! Markers and "what changed since" (#82).
//!
//! The panel lists the markers, makes new ones, renames them and answers
//! "what did that installer just do?": the net change of every raw row after
//! a marker (or after some minutes ago), folded per file by
//! [`peekbuddy_core::changes`]. Created and deleted inside the range
//! (transient) is only counted. Every row says how many raw rows it folds
//! and can show them in the log (F3: the diff is a derived view). A diff
//! across an unreconciled gap says it is incomplete and where.
//!
//! The work runs on a thread of its own and reports progress; the panel only
//! draws what the thread left. It knows nothing about eframe, so it runs under
//! `egui_kittest` on any host.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use peekbuddy_core::changes::{Net, NetKind};
use peekbuddy_core::query::CancelFlag;
use peekbuddy_store::markers::{self, Marker};
use peekbuddy_store::read::Reader;
use peekbuddy_store::read::changes::{ChangeSet, Progress};
use peekbuddy_store::segment::DayKey;

use crate::rows::{OffsetFn, format_time};

/// FILETIME ticks per minute.
const MINUTE: u64 = 600_000_000;

/// How far back "the last ..." reaches, in minutes.
pub const RANGES: [(&str, u64); 3] = [("15 minutes", 15), ("hour", 60), ("24 hours", 1_440)];

/// Today's calendar day and the time now (FILETIME ticks), as the panel asks
/// for them.
pub type Clocks = (fn() -> DayKey, fn() -> u64);

/// What the user asked for; the app sends or shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangesAction {
    /// Write a marker row now.
    MarkNow,
    /// Rename a marker.
    Rename {
        /// The id of the marker's event row.
        event_id: u64,
        /// The new name.
        name: String,
    },
    /// Show the raw rows of one path in the log.
    ShowRows {
        /// The folder of the path.
        dir: String,
        /// The file name.
        name: String,
    },
}

/// What the diff is measured from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Since {
    /// The marker with this event id, by name.
    Marker(i64, String),
    /// Some minutes ago.
    Minutes(u64),
}

impl Since {
    /// The sentence in the result header.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Marker(_, name) => format!("since marker \u{201c}{name}\u{201d}"),
            Self::Minutes(60) => String::from("in the last hour"),
            Self::Minutes(minutes) => format!("in the last {minutes} minutes"),
        }
    }
}

/// One row of the result as it is drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// One ASCII glyph, so the kind is not told by colour alone.
    pub glyph: &'static str,
    /// The kind in words.
    pub label: &'static str,
    /// The path now.
    pub path: String,
    /// The path before, for a rename.
    pub from: Option<String>,
    /// Whether a renamed file also changed.
    pub modified: bool,
    /// Raw rows folded into this line.
    pub raw_events: u32,
}

impl Line {
    /// The row as a line of TSV: kind, path, from, raw events.
    #[must_use]
    pub fn tsv(&self) -> String {
        format!(
            "{}\t{}\t{}\t{}",
            self.label,
            self.path,
            self.from.as_deref().unwrap_or(""),
            self.raw_events
        )
    }
}

const fn words(kind: NetKind) -> (&'static str, &'static str) {
    match kind {
        NetKind::Created => ("+", "Created"),
        NetKind::Deleted => ("-", "Deleted"),
        NetKind::Modified => ("~", "Modified"),
        NetKind::Renamed => (">", "Renamed"),
    }
}

/// The rows of `set` as lines, in the order of the fold.
#[must_use]
pub fn lines(set: &ChangeSet) -> Vec<Line> {
    set.summary
        .rows
        .iter()
        .map(|row: &Net<u64>| {
            let (glyph, label) = words(row.kind);
            Line {
                glyph,
                label,
                path: set.path(row.key).to_owned(),
                from: row.from.map(|from| set.path(from).to_owned()),
                modified: row.modified,
                raw_events: row.raw_events,
            }
        })
        .collect()
}

/// The totals line: `3 created, 1 deleted, 12 modified, 2 renamed`, with the
/// transient files as a visible count.
#[must_use]
pub fn totals(set: &ChangeSet) -> String {
    let mut counts = [0_usize; 4];
    for row in &set.summary.rows {
        let slot = match row.kind {
            NetKind::Created => 0,
            NetKind::Deleted => 1,
            NetKind::Modified => 2,
            NetKind::Renamed => 3,
        };
        if let Some(count) = counts.get_mut(slot) {
            *count += 1;
        }
    }
    let [created, deleted, modified, renamed] = counts;
    let mut text =
        format!("{created} created, {deleted} deleted, {modified} modified, {renamed} renamed");
    if set.summary.transient > 0 {
        let _ = write!(
            text,
            "; {} created and removed again (transient, hidden)",
            set.summary.transient
        );
    }
    text
}

/// What the gaps in the range say, one line each; empty when there are none.
#[must_use]
pub fn gap_lines(set: &ChangeSet) -> Vec<String> {
    set.summary
        .gaps
        .iter()
        .map(|gap| {
            let root = set.roots.get(&gap.root).map_or("a watch", String::as_str);
            let root = root.strip_prefix(r"\\?\").unwrap_or(root);
            if gap.reconciled {
                format!("{root}: changes were missed, a scan of the folder found them")
            } else {
                format!("{root}: changes were missed and no scan covered them")
            }
        })
        .collect()
}

/// What the worker thread leaves for the panel.
#[derive(Default)]
struct Shared {
    markers: Vec<Marker>,
    loading: bool,
    error: Option<String>,
    progress: Option<Progress>,
    running: bool,
    result: Option<(Since, ChangeSet)>,
}

/// The panel.
pub struct ChangesPanel {
    segments: PathBuf,
    waker: Arc<dyn Fn() + Send + Sync>,
    offset: Arc<OffsetFn>,
    today: fn() -> DayKey,
    now: fn() -> u64,
    max_age_days: Option<u32>,
    shared: Arc<Mutex<Shared>>,
    cancel: CancelFlag,
    renaming: Option<(i64, String)>,
    asked: bool,
    status: Option<(bool, String)>,
}

impl ChangesPanel {
    /// A panel over the segments in `segments`.
    #[must_use]
    pub fn new(
        segments: PathBuf,
        waker: Arc<dyn Fn() + Send + Sync>,
        offset: Arc<OffsetFn>,
        clocks: Clocks,
    ) -> Self {
        Self {
            segments,
            waker,
            offset,
            today: clocks.0,
            now: clocks.1,
            max_age_days: None,
            shared: Arc::new(Mutex::new(Shared::default())),
            cancel: CancelFlag::new(),
            renaming: None,
            asked: false,
            status: None,
        }
    }

    /// The age limit of retention, for the warning before a marker goes.
    pub fn set_max_age_days(&mut self, days: Option<u32>) {
        self.max_age_days = days;
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Reads the markers again, on a thread.
    pub fn refresh(&self) {
        {
            let mut shared = self.lock();
            if shared.loading {
                return;
            }
            shared.loading = true;
        }
        let (dir, shared, waker) = (
            self.segments.clone(),
            Arc::clone(&self.shared),
            Arc::clone(&self.waker),
        );
        std::thread::spawn(move || {
            let found = markers::list(&dir);
            {
                let mut shared = shared.lock().unwrap_or_else(PoisonError::into_inner);
                shared.loading = false;
                match found {
                    Ok(mut list) => {
                        list.reverse();
                        shared.markers = list;
                        shared.error = None;
                    }
                    Err(error) => shared.error = Some(format!("markers: {error}")),
                }
            }
            waker();
        });
    }

    /// Reads the markers again once the engine has had time to commit the
    /// row it was just asked for.
    pub fn refresh_soon(&self) {
        let (dir, shared, waker) = (
            self.segments.clone(),
            Arc::clone(&self.shared),
            Arc::clone(&self.waker),
        );
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(900));
            if let Ok(mut list) = markers::list(&dir) {
                list.reverse();
                shared
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .markers = list;
            }
            waker();
        });
    }

    /// Starts the diff, stopping one that still runs.
    pub fn compute(&mut self, since: Since) {
        self.cancel.cancel();
        self.cancel = CancelFlag::new();
        {
            let mut shared = self.lock();
            shared.running = true;
            shared.progress = None;
            shared.error = None;
        }
        let (dir, shared, waker, cancel, now) = (
            self.segments.clone(),
            Arc::clone(&self.shared),
            Arc::clone(&self.waker),
            self.cancel.clone(),
            self.now,
        );
        std::thread::spawn(move || {
            let reader = Reader::new(&dir);
            let from = match &since {
                Since::Marker(id, _) => Ok(*id),
                Since::Minutes(minutes) => reader.id_before(now().saturating_sub(minutes * MINUTE)),
            };
            let outcome = from.and_then(|id| {
                reader.changes_since(id, &cancel, &mut |progress| {
                    let mut shared = shared.lock().unwrap_or_else(PoisonError::into_inner);
                    shared.progress = Some(progress);
                    drop(shared);
                    waker();
                })
            });
            let mut shared = shared.lock().unwrap_or_else(PoisonError::into_inner);
            // A newer request replaced this one: leave its state alone.
            if !cancel.is_cancelled() || outcome.as_ref().is_ok_and(|set| set.cancelled) {
                shared.running = false;
            }
            match outcome {
                Ok(set) if !cancel.is_cancelled() => shared.result = Some((since, set)),
                Ok(_) => {}
                Err(error) => shared.error = Some(format!("changes: {error}")),
            }
            drop(shared);
            waker();
        });
    }

    /// The engine's answer to a marker command: shown under the buttons, and
    /// the list is read again when a marker was made or renamed.
    pub fn on_result(&mut self, ok: bool, message: &str) {
        self.status = Some((ok, message.to_owned()));
        if ok {
            self.refresh_soon();
        }
    }

    /// Stops the diff that runs.
    pub fn stop(&self) {
        self.cancel.cancel();
        self.lock().running = false;
    }

    /// The finished diff, if any.
    #[must_use]
    pub fn result(&self) -> Option<(Since, ChangeSet)> {
        self.lock().result.clone()
    }

    /// Draws the panel and returns what the user asked for.
    pub fn show(&mut self, ui: &mut egui::Ui) -> Vec<ChangesAction> {
        if !self.asked {
            self.asked = true;
            self.refresh();
        }
        let mut actions = Vec::new();
        ui.horizontal(|ui| {
            let mark = ui.button("Mark now");
            mark.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::Role::Button,
                    true,
                    "Mark now: name this moment in the log",
                )
            });
            if mark.on_hover_text("Name this moment (Ctrl+M)").clicked() {
                actions.push(ChangesAction::MarkNow);
            }
            if ui.button("Refresh markers").clicked() {
                self.refresh();
            }
        });
        if let Some((ok, text)) = &self.status {
            let color = if *ok {
                ui.visuals().text_color()
            } else {
                ui.visuals().error_fg_color
            };
            ui.colored_label(color, text);
        }
        let (markers, error) = {
            let shared = self.lock();
            (shared.markers.clone(), shared.error.clone())
        };
        if let Some(error) = error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        self.marker_list(ui, &markers, &mut actions);
        ui.horizontal(|ui| {
            ui.label("Changes in the last:");
            for (label, minutes) in RANGES {
                let button = ui.button(label);
                button.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::Role::Button,
                        true,
                        format!("What changed in the last {label}"),
                    )
                });
                if button.clicked() {
                    self.compute(Since::Minutes(minutes));
                }
            }
        });
        ui.separator();
        self.result_view(ui, &mut actions);
        actions
    }

    fn marker_list(
        &mut self,
        ui: &mut egui::Ui,
        markers: &[Marker],
        actions: &mut Vec<ChangesAction>,
    ) {
        if markers.is_empty() {
            ui.label("No markers yet. Mark a moment, do something, then ask what changed.");
            return;
        }
        egui::ScrollArea::vertical()
            .id_salt("markers")
            .max_height(150.0)
            .show(ui, |ui| {
                for marker in markers {
                    ui.horizontal(|ui| {
                        self.marker_row(ui, marker, actions);
                    });
                }
            });
    }

    fn marker_row(&mut self, ui: &mut egui::Ui, marker: &Marker, actions: &mut Vec<ChangesAction>) {
        let editing = self
            .renaming
            .as_ref()
            .is_some_and(|(id, _)| *id == marker.event_id);
        if editing {
            let mut done = None;
            if let Some((_, text)) = self.renaming.as_mut() {
                let edit = ui.add(egui::TextEdit::singleline(text).desired_width(220.0));
                edit.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::TextInput, true, "New marker name")
                });
                edit.request_focus();
                if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    done = Some(true);
                }
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    done = Some(false);
                }
            }
            if ui.button("Save name").clicked() {
                done = Some(true);
            }
            if ui.button("Cancel").clicked() {
                done = Some(false);
            }
            match done {
                Some(true) => {
                    if let Some((id, name)) = self.renaming.take() {
                        actions.push(ChangesAction::Rename {
                            event_id: u64::try_from(id).unwrap_or(0),
                            name,
                        });
                    }
                }
                Some(false) => self.renaming = None,
                None => {}
            }
            return;
        }
        ui.label(egui::RichText::new(&marker.name).strong());
        ui.label(format_time(
            u64::try_from(marker.created_ts).unwrap_or(0),
            (self.offset)(u64::try_from(marker.created_ts).unwrap_or(0)),
        ));
        if markers::expires_soon(&marker.segment, (self.today)(), self.max_age_days) {
            ui.colored_label(ui.visuals().warn_fg_color, "! goes with its day file soon")
                .on_hover_text(
                    "Markers are deleted with the day file they are in, by the retention \
                     limits. Export what you need before then.",
                );
        }
        let since = ui.button("Changes since");
        since.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::Role::Button,
                true,
                format!("What changed since marker {}", marker.name),
            )
        });
        if since.clicked() {
            self.compute(Since::Marker(marker.event_id, marker.name.clone()));
        }
        if marker.open_segment {
            let rename = ui.button("Rename");
            rename.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::Role::Button,
                    true,
                    format!("Rename marker {}", marker.name),
                )
            });
            if rename.clicked() {
                self.renaming = Some((marker.event_id, marker.name.clone()));
            }
        }
    }

    fn result_view(&mut self, ui: &mut egui::Ui, actions: &mut Vec<ChangesAction>) {
        let (running, progress, result) = {
            let shared = self.lock();
            (shared.running, shared.progress, shared.result.clone())
        };
        if running {
            ui.horizontal(|ui| {
                ui.spinner();
                let text = progress.map_or_else(
                    || String::from("Reading the log..."),
                    |p| {
                        format!(
                            "Reading the log... {} rows, day file {} of {}",
                            p.scanned,
                            (p.segments_done + 1).min(p.segments),
                            p.segments
                        )
                    },
                );
                ui.label(text);
                if ui.button("Stop").clicked() {
                    self.stop();
                }
            });
        }
        let Some((since, set)) = result else {
            if !running {
                ui.label("Pick a marker, or a time span, to see what changed.");
            }
            return;
        };
        ui.strong(format!("What changed {}", since.describe()));
        if set.cancelled {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                "! Stopped early: this list is partial.",
            );
        }
        let gaps = gap_lines(&set);
        if set.summary.incomplete() {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                "! Incomplete: the log has holes in this range.",
            );
        }
        for line in &gaps {
            ui.label(line);
        }
        ui.label(totals(&set));
        if set.summary.transient > 0 {
            ui.small("Files that only existed inside the range are counted, not listed.");
        }
        let rows = lines(&set);
        if ui.button("Copy list").clicked() {
            let mut text = String::from("change\tpath\tfrom\traw events\n");
            for line in &rows {
                text.push_str(&line.tsv());
                text.push('\n');
            }
            ui.ctx().copy_text(text);
        }
        let height = ui.text_style_height(&egui::TextStyle::Body) + 6.0;
        egui::ScrollArea::vertical()
            .id_salt("changes")
            .auto_shrink([false, false])
            .show_rows(ui, height, rows.len(), |ui, range| {
                for line in rows.iter().skip(range.start).take(range.len()) {
                    ui.horizontal(|ui| {
                        ui.monospace(line.glyph);
                        ui.label(line.label);
                        let shown = match &line.from {
                            Some(from) => format!("{from}  ->  {}", line.path),
                            None => line.path.clone(),
                        };
                        ui.label(shown);
                        if line.modified {
                            ui.label("(also changed)");
                        }
                        let raw = ui.link(format!("raw events: {}", line.raw_events));
                        raw.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::Role::Link,
                                true,
                                format!("Show the {} raw rows of {}", line.raw_events, line.path),
                            )
                        });
                        if raw.clicked() {
                            let (dir, name) = split_path(&line.path);
                            actions.push(ChangesAction::ShowRows { dir, name });
                        }
                    });
                }
            });
    }
}

/// The folder and the file name of a path.
#[must_use]
pub fn split_path(path: &str) -> (String, String) {
    path.rsplit_once('\\').map_or_else(
        || (String::new(), path.to_owned()),
        |(dir, name)| (dir.to_owned(), name.to_owned()),
    )
}

#[cfg(test)]
mod tests;
