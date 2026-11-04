//! The event grid (#66): a virtualized table over a log of millions of rows,
//! with live tail and follow mode.
//!
//! [`LogView`] ties the pieces together and draws them:
//!
//! * [`crate::timeline`] decides which pages the visible rows need;
//! * [`crate::worker`] reads them on its own thread and follows the log;
//! * [`crate::follow`] and [`crate::selection`] are the two small state
//!   machines;
//! * `egui_table` lays out the rows. It draws only the visible ones, so the
//!   cost of a frame does not depend on the size of the log.
//!
//! The **compact** toggle folds renames, saves and bursts behind their newest
//! row ([`crate::compact`]); folded rows keep their place in the scrollbar
//! but have no height.
//!
//! A page that has not arrived is drawn as an ellipsis row; the UI thread
//! never waits for the database.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::io;
use std::ops::Range;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use egui_table::{
    AutoSizeMode, CellInfo, Column, HeaderCellInfo, HeaderRow, PrefetchInfo, Table, TableDelegate,
};
use peekbuddy_core::event::Flags;
use peekbuddy_core::query::{CancelFlag, Filter};

use crate::compact::{Fold, Folds};
use crate::debounce::{DELAY, Debounce};
use crate::filterspec::{Chip, Field, FilterSpec, PatternKind, Span, SpecError};
use crate::follow::Follow;
use crate::rows::{GridRow, OffsetFn, RowClass};
use crate::selection::{Selection, Step};
use crate::source::PageSource;
use crate::timeline::{Extent, Fetch, Tail, Timeline};
use crate::worker::{Job, Reply, Worker};

/// Height of a row in points. Fixed, so the scroll maths is O(1).
pub const ROW_HEIGHT: f32 = 22.0;
/// Height of the header row.
pub const HEADER_HEIGHT: f32 = 26.0;

/// A column of the grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Col {
    /// When the event was recorded, local time.
    Time,
    /// What happened.
    Kind,
    /// Where, middle-elided.
    Path,
    /// Where it was before a rename.
    OldPath,
    /// How big, when known.
    Size,
    /// Which mechanism saw it.
    Source,
}

impl Col {
    /// Every column, in the order the chooser lists them.
    pub const ALL: [Self; 6] = [
        Self::Time,
        Self::Kind,
        Self::Path,
        Self::OldPath,
        Self::Size,
        Self::Source,
    ];

    /// The name in `settings.toml` (`[view] columns`).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Time => "time",
            Self::Kind => "kind",
            Self::Path => "path",
            Self::OldPath => "old_path",
            Self::Size => "size",
            Self::Source => "source",
        }
    }

    /// The column for a settings name; the reserved names (`process`,
    /// `volume`, `raw`) are not columns of this build.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|col| col.name() == name)
    }

    /// The header text.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Time => "Time",
            Self::Kind => "Kind",
            Self::Path => "Path",
            Self::OldPath => "Old path",
            Self::Size => "Size",
            Self::Source => "Source",
        }
    }

    /// What the header says when the pointer rests on it.
    #[must_use]
    pub const fn tip(self) -> &'static str {
        match self {
            Self::Time => {
                "Local time the event was recorded; hover a row for UTC. Rows are in the order \
                 they were stored. Rows found by a reconcile scan carry the time they were \
                 detected, so times can step back."
            }
            Self::Kind => "What happened. A symbol and a word, never a colour alone.",
            Self::Path => "The full path; hover a row to read it all.",
            Self::OldPath => "The path before a rename.",
            Self::Size => "Size after the change, when Windows reported it.",
            Self::Source => {
                "RDCW and USN are live changes; RECONCILE rows were found by a scan after a gap."
            }
        }
    }

    const fn width(self) -> f32 {
        match self {
            Self::Time => 180.0,
            Self::Kind => 170.0,
            Self::Path => 520.0,
            Self::OldPath => 320.0,
            Self::Size => 90.0,
            Self::Source => 100.0,
        }
    }
}

/// The visible columns for the names in `settings.toml`; never none.
#[must_use]
pub fn columns_from(names: &[String]) -> Vec<Col> {
    let mut out: Vec<Col> = Vec::new();
    for col in names.iter().filter_map(|name| Col::from_name(name)) {
        if !out.contains(&col) {
            out.push(col);
        }
    }
    if out.is_empty() {
        out = vec![Col::Time, Col::Kind, Col::Path];
    }
    out
}

/// Where the grid stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// The extent of the log is being read.
    Loading,
    /// Rows can be shown (maybe none).
    Ready,
    /// Reading failed; browsing is not possible.
    Failed(String),
}

/// What the user did in the grid that the app has to carry out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewEvent {
    /// The visible columns changed; these are the new names for
    /// `[view] columns`.
    ColumnsChanged(Vec<String>),
    /// The details pane was opened or closed (`true`: open).
    DetailsToggled(bool),
    /// Show a path in Explorer: select the item (`select`), or open the
    /// folder it is in. The app does it off the UI thread.
    Open {
        /// The path as stored.
        path: String,
        /// Select the item instead of opening its folder.
        select: bool,
    },
}

/// What a row's context menu asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RowAction {
    CopyPath(String),
    CopyRows,
    FilterFolder(String),
    FilterFile(String),
    FilterKind(Chip),
    Open { path: String, select: bool },
}

/// The current time as FILETIME ticks (100 ns since 1601, UTC).
#[must_use]
pub fn filetime_now() -> u64 {
    const UNIX_TO_FILETIME: u64 = 11_644_473_600;
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            (since.as_secs() + UNIX_TO_FILETIME) * 10_000_000
                + u64::from(since.subsec_nanos() / 100)
        })
}

/// A number with thin grouping: `1 234 567`.
#[must_use]
pub fn group_digits(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (at, digit) in digits.chars().enumerate() {
        if at > 0 && (digits.len() - at).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(digit);
    }
    out
}

/// The grid and everything it needs.
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent facts of the view: watching, reloading, first page, advanced row open, focus request"
)]
pub struct LogView {
    worker: Worker,
    filter: Filter,
    generation: u64,
    cancel: CancelFlag,
    timeline: Timeline<GridRow>,
    follow: Follow,
    selection: Selection,
    columns: Vec<Col>,
    phase: Phase,
    watching: bool,
    visible: Range<u64>,
    /// Rows scanned so far by a filtered read, while one runs.
    scanned: u64,
    scroll_to: Option<(u64, egui::Align)>,
    status: Status,
    first_page: bool,
    /// The details pane is open.
    details: bool,
    /// Where exports read from and go to: the segments and the exports folder.
    export_dirs: Option<(std::path::PathBuf, std::path::PathBuf)>,
    /// The export that runs or has just ended.
    export: Option<crate::exporter::Export>,
    wake: Arc<dyn Fn() + Send + Sync>,
    /// Set by a row's context menu, handled after the table.
    action: Option<RowAction>,
    /// What the filter bar shows and the user is editing.
    spec: FilterSpec,
    /// Why the spec cannot be applied; the previous results stay.
    spec_error: Option<SpecError>,
    debounce: Debounce,
    offset: Arc<OffsetFn>,
    /// The watch roots to choose from.
    roots: Vec<String>,
    /// A new filter waits for its extent: the old island is shown but not
    /// extended (its cursors belong to the old filter).
    reloading: bool,
    /// How many rows the view hides, when counted.
    hidden: Option<u64>,
    /// The advanced row of the bar is open.
    more: bool,
    /// The text form being edited, and what is wrong with it.
    text: String,
    text_error: Option<String>,
    focus_search: bool,
    /// The compact view is on: groups are folded inside the held rows.
    compact: bool,
    /// The heads of the groups the user opened.
    expanded: BTreeSet<i64>,
    /// Bumped when `expanded` changes.
    expanded_gen: u64,
    folds: Folds,
    /// What `folds` was made from: timeline revision, mode, `expanded_gen`.
    folds_key: Option<(u64, bool, u64)>,
}

/// The status line, rebuilt only when what it says changes.
#[derive(Default)]
struct Status {
    key: (u64, bool, u64, u64, u64),
    text: String,
}

impl std::fmt::Debug for LogView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogView")
            .field("phase", &self.phase)
            .field("rows", &self.timeline.len())
            .finish_non_exhaustive()
    }
}

/// Whether `filter` leaves the whole id sequence to browse (hidden rows
/// aside): then a scrollbar position is an id by arithmetic.
fn is_seekable(filter: &Filter) -> bool {
    let plain = Filter {
        hide: filter.hide,
        ..Filter::default()
    };
    // `Filter` has no `PartialEq`: compare the fields that matter.
    filter.kinds.as_ref().is_none_or(|kinds| kinds.is_all())
        && filter.root.is_none()
        && filter.time == plain.time
        && filter.subtree.is_none()
        && filter.name.is_none()
        && filter.pattern.is_none()
}

impl LogView {
    /// Starts reading `source` with `filter`; `wake` runs when something
    /// arrived for the UI.
    ///
    /// # Errors
    ///
    /// The error from starting the worker thread.
    pub fn new(
        source: Arc<dyn PageSource>,
        offset: Arc<OffsetFn>,
        wake: Arc<dyn Fn() + Send + Sync>,
        spec: FilterSpec,
        roots: Vec<String>,
        columns: &[String],
    ) -> io::Result<Self> {
        let now = filetime_now();
        let (filter, spec_error) = match spec.to_filter(now, offset(now)) {
            Ok(filter) => (filter, None),
            Err(error) => (Filter::default_view(), Some(error)),
        };
        let worker = Worker::start(source, Arc::clone(&offset), Arc::clone(&wake))?;
        let view = Self {
            worker,
            filter,
            generation: 1,
            cancel: CancelFlag::new(),
            timeline: Timeline::new(None, true, true),
            follow: Follow::default(),
            selection: Selection::default(),
            columns: columns_from(columns),
            phase: Phase::Loading,
            watching: false,
            visible: 0..0,
            scanned: 0,
            scroll_to: None,
            status: Status::default(),
            first_page: false,
            details: true,
            export_dirs: None,
            export: None,
            wake,
            action: None,
            text: spec.to_text(),
            spec,
            spec_error,
            debounce: Debounce::new(DELAY),
            offset,
            roots,
            reloading: false,
            hidden: None,
            more: false,
            text_error: None,
            focus_search: false,
            compact: false,
            expanded: BTreeSet::new(),
            expanded_gen: 0,
            folds: Folds::none(),
            folds_key: None,
        };
        view.worker.send(Job::Extent { generation: 1 });
        Ok(view)
    }

    /// Whether the first page (or the knowledge that there is none) has
    /// arrived: the viewer's first frame counts from here (F7).
    #[must_use]
    pub const fn first_page_ready(&self) -> bool {
        self.first_page
    }

    /// Rows held, for tests and diagnostics.
    #[must_use]
    pub fn held_rows(&self) -> usize {
        self.timeline.len()
    }

    /// The visible columns.
    #[must_use]
    pub fn columns(&self) -> &[Col] {
        &self.columns
    }

    /// The phase.
    #[must_use]
    pub const fn phase(&self) -> &Phase {
        &self.phase
    }

    /// The rows the selection covers, newest first, that are held.
    pub fn selected_rows(&self) -> impl Iterator<Item = &GridRow> {
        let range = self.selection.range();
        range
            .into_iter()
            .flatten()
            .filter_map(|index| self.timeline.row_at(index))
    }

    /// Starts over with another filter: the old reads are cancelled and the
    /// first page of the new view is fetched. Until its extent arrives the old
    /// rows stay on screen, so typing does not flicker.
    fn set_filter(&mut self, filter: Filter) {
        self.cancel.cancel();
        self.cancel = CancelFlag::new();
        self.generation += 1;
        self.filter = filter;
        self.watching = false;
        self.scanned = 0;
        self.hidden = None;
        self.selection.clear();
        self.follow = Follow::default();
        self.reloading = true;
        self.worker.send(Job::StopTail);
        self.worker.send(Job::Extent {
            generation: self.generation,
        });
    }

    /// The spec being edited changed: the filter text follows, and the filter
    /// is applied after the pause.
    fn spec_changed(&mut self) {
        self.text = self.spec.to_text();
        self.text_error = None;
        self.debounce.touch(Instant::now());
    }

    /// Applies the spec: compiles it, and on success reads the new view. An
    /// invalid spec keeps the previous results and says why.
    fn apply_spec(&mut self) {
        let now = filetime_now();
        match self.spec.to_filter(now, (self.offset)(now)) {
            Ok(filter) => {
                self.spec_error = None;
                self.set_filter(filter);
            }
            Err(error) => self.spec_error = Some(error),
        }
    }

    /// Enables exporting: `segments` is where the rows are read, `exports`
    /// where the files go (inside the data directory, which is never watched).
    pub fn set_export_dirs(&mut self, segments: std::path::PathBuf, exports: std::path::PathBuf) {
        self.export_dirs = Some((segments, exports));
    }

    /// Cancels a running export; the partial file is removed. Called when
    /// the window closes.
    pub fn cancel_export(&self) {
        if let Some(export) = &self.export {
            export.cancel();
        }
    }

    /// Starts an export of the current view in `format`.
    fn start_export(&mut self, format: peekbuddy_core::export::Format) {
        let Some((segments, exports)) = &self.export_dirs else {
            return;
        };
        let now = filetime_now();
        let stamp = crate::rows::format_time(now, (self.offset)(now));
        let target =
            crate::exporter::free_target(exports, &crate::exporter::file_name(&stamp, format));
        let job = peekbuddy_store::export::Job {
            filter: self.filter.clone(),
            format,
            options: crate::exporter::default_options(),
        };
        // A folder that cannot be made is shown as a failed export.
        self.export = crate::exporter::Export::start(
            segments,
            target,
            job,
            Arc::clone(&self.offset),
            Arc::clone(&self.wake),
        )
        .ok();
    }

    /// The export menu, or the state of the export.
    fn export_ui(&mut self, ui: &mut egui::Ui, events: &mut Vec<ViewEvent>) {
        use crate::exporter::State;
        use peekbuddy_core::export::Format;

        let Some(export) = &self.export else {
            if self.export_dirs.is_some() {
                let mut chosen = None;
                ui.menu_button("Export", |ui| {
                    if ui.button("CSV").clicked() {
                        chosen = Some(Format::Csv);
                        ui.close();
                    }
                    if ui.button("JSON Lines").clicked() {
                        chosen = Some(Format::JsonLines);
                        ui.close();
                    }
                });
                if let Some(format) = chosen {
                    self.start_export(format);
                }
            }
            return;
        };
        let mut dismiss = false;
        match export.state() {
            State::Running => {
                ui.label(format!(
                    "Exporting {} of about {} rows",
                    group_digits(export.written()),
                    group_digits(export.estimated())
                ));
                if ui.button("Cancel export").clicked() {
                    export.cancel();
                }
                // Progress at most ten times a second.
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(100));
            }
            State::Done { rows, path } => {
                ui.label(format!(
                    "Exported {} rows to {}",
                    group_digits(rows),
                    path.display()
                ));
                if ui.button("Show in Explorer").clicked() {
                    events.push(ViewEvent::Open {
                        path: path.to_string_lossy().into_owned(),
                        select: true,
                    });
                }
                dismiss = ui.button("Dismiss").clicked();
            }
            State::Cancelled => {
                ui.label("Export cancelled; nothing was kept.");
                dismiss = ui.button("Dismiss").clicked();
            }
            State::Failed(error) => {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    format!("Export failed: {error}"),
                );
                dismiss = ui.button("Dismiss").clicked();
            }
        }
        if dismiss {
            self.export = None;
        }
    }

    /// Opens or closes the details pane (from `[viewer] details_hidden`).
    pub const fn set_details(&mut self, open: bool) {
        self.details = open;
    }

    /// Whether the details pane is open.
    #[must_use]
    pub const fn details_open(&self) -> bool {
        self.details
    }

    /// The text form of the filter, for `[viewer] filter`.
    #[must_use]
    pub fn filter_text(&self) -> String {
        self.spec.to_text()
    }

    /// Whether a new filter waits for the pause or for its first answer.
    #[must_use]
    pub fn is_applying(&self) -> bool {
        self.debounce.pending() || self.reloading
    }

    /// Stops the scan that is running; what was found stays.
    pub fn stop_scan(&mut self) {
        self.cancel.cancel();
        self.cancel = CancelFlag::new();
        self.timeline.abandon_pending();
        self.scanned = 0;
    }

    /// Drains what the worker found. Called once per frame, before drawing.
    pub fn logic(&mut self) {
        while let Some(reply) = self.worker.poll() {
            self.take(reply);
        }
        if self.debounce.due(Instant::now()) {
            self.apply_spec();
        }
        // A view with no row yet asks for its first page without waiting for
        // the table to be laid out (a streaming view has zero rows to lay out).
        let complete = self.timeline.at_newest() && self.timeline.at_oldest();
        if self.phase == Phase::Ready && !self.reloading && self.timeline.is_empty() && !complete {
            for fetch in self.timeline.plan(&self.visible) {
                self.send_page(fetch);
            }
        }
    }

    fn take(&mut self, reply: Reply) {
        match reply {
            Reply::Extent { generation, result } if generation == self.generation => {
                self.on_extent(result);
            }
            Reply::Page {
                generation,
                fetch,
                result,
            } if generation == self.generation => match result {
                Ok(fetched) => {
                    self.timeline.apply(&fetch, fetched);
                    if matches!(fetch, Fetch::Seek { .. }) {
                        self.first_page = true;
                        self.watch_tail();
                    }
                }
                Err(error) => {
                    self.timeline.failed(&fetch);
                    self.first_page = true;
                    self.phase = Phase::Failed(error);
                }
            },
            Reply::Hidden {
                generation,
                count: Some(count),
            } if generation == self.generation => {
                self.hidden = Some(count);
                self.timeline.note_hidden(count);
            }
            Reply::Progress {
                generation,
                scanned,
            } if generation == self.generation => self.scanned = scanned,
            Reply::Tail {
                generation,
                fetched,
            } if generation == self.generation => match self.timeline.ingest_tail(fetched) {
                Tail::Merged(count) => self.selection.shift_down(count),
                Tail::Counted(count) => self.follow.arrived(count),
                Tail::Hole => self.watching = false,
                Tail::Nothing => {}
            },
            // Replies of an older generation.
            Reply::Extent { .. }
            | Reply::Page { .. }
            | Reply::Hidden { .. }
            | Reply::Progress { .. }
            | Reply::Tail { .. } => {}
        }
        if !self.watching && self.first_page {
            self.watch_tail();
        }
    }

    fn on_extent(&mut self, result: Result<Option<Extent>, String>) {
        let extent = match result {
            Ok(extent) => extent,
            Err(error) => {
                self.first_page = true;
                self.phase = Phase::Failed(error);
                return;
            }
        };
        let seekable = is_seekable(&self.filter);
        self.timeline = Timeline::new(extent, seekable, self.filter.is_empty());
        self.phase = Phase::Ready;
        self.reloading = false;
        if extent.is_none() {
            // Nothing recorded yet: the empty view is the first page, and
            // the tail brings the first rows.
            self.timeline.empty_log();
            self.first_page = true;
        } else if self.filter.hide != Flags::NONE {
            self.worker.send(Job::Hidden {
                generation: self.generation,
                filter: self.filter.clone(),
                cancel: self.cancel.clone(),
            });
        }
        self.watch_tail();
    }

    fn watch_tail(&mut self) {
        if self.watching {
            return;
        }
        self.watching = true;
        self.worker.send(Job::WatchTail {
            generation: self.generation,
            filter: self.filter.clone(),
            cursor: self.timeline.tail_cursor(),
        });
    }

    /// The status text, rebuilt when it changes.
    fn status_text(&mut self) -> &str {
        let selected = self
            .selection
            .range()
            .map_or(0, |r| r.end() - r.start() + 1);
        let folded = self.folds.folded();
        let key = (
            self.timeline.total(),
            self.timeline.is_estimate() || self.compact,
            selected,
            self.scanned,
            folded,
        );
        if self.status.key != key || self.status.text.is_empty() {
            self.status.key = key;
            let count = group_digits(self.timeline.total());
            let mut text = if key.1 {
                format!("about {count} events")
            } else {
                format!("{count} events")
            };
            if folded > 0 {
                let _ = write!(text, ", {} folded into groups", group_digits(folded));
            }
            if selected > 0 {
                let _ = write!(text, ", {} selected", group_digits(selected));
            }
            self.status.text = text;
        }
        &self.status.text
    }

    /// Handles the keys of the grid: arrows, pages, Home and End, with Shift
    /// to extend the selection.
    fn keys(&mut self, ctx: &egui::Context) {
        // Ctrl+F focuses the search box and Esc clears it, wherever the focus is.
        let (find, escape) = ctx.input(|input| {
            (
                input.modifiers.command && input.key_pressed(egui::Key::F),
                input.key_pressed(egui::Key::Escape),
            )
        });
        if find {
            self.focus_search = true;
        }
        if escape && !self.spec.name.is_empty() {
            self.spec.name.clear();
            self.spec_changed();
        }
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        if ctx.input(|input| {
            input
                .events
                .iter()
                .any(|event| matches!(event, egui::Event::Copy))
        }) {
            ctx.copy_text(crate::rows::tsv(self.selected_rows()));
        }
        let page = self
            .visible
            .end
            .saturating_sub(self.visible.start)
            .saturating_sub(1);
        let (step, shift) = ctx.input(|input| {
            let step = if input.key_pressed(egui::Key::ArrowDown) {
                Some(Step::Down)
            } else if input.key_pressed(egui::Key::ArrowUp) {
                Some(Step::Up)
            } else if input.key_pressed(egui::Key::PageDown) {
                Some(Step::PageDown(page))
            } else if input.key_pressed(egui::Key::PageUp) {
                Some(Step::PageUp(page))
            } else if input.key_pressed(egui::Key::Home) {
                Some(Step::Home)
            } else if input.key_pressed(egui::Key::End) {
                Some(Step::End)
            } else {
                None
            };
            (step, input.modifiers.shift)
        });
        let Some(step) = step else { return };
        let total = self.timeline.total();
        let Some(mut cursor) = self.selection.step(step, total, shift) else {
            return;
        };
        if self.compact && self.folds.is_folded(cursor) {
            // A folded row has no height: the cursor goes on to the next one
            // that is shown, the other way when the end is reached.
            let (forward, back) = match step {
                Step::Up | Step::PageUp(_) | Step::Home | Step::End => (Step::Up, Step::Down),
                Step::Down | Step::PageDown(_) => (Step::Down, Step::Up),
            };
            for direction in [forward, back] {
                while self.folds.is_folded(cursor) {
                    match self.selection.step(direction, total, shift) {
                        Some(next) if next != cursor => cursor = next,
                        _ => break,
                    }
                }
            }
        }
        if cursor < self.visible.start {
            self.scroll_to = Some((cursor, egui::Align::TOP));
        } else if cursor + 1 >= self.visible.end {
            self.scroll_to = Some((cursor, egui::Align::BOTTOM));
        }
    }

    /// Draws the filter bar, the grid and the pill. Returns what the app has
    /// to do.
    pub fn show(&mut self, ui: &mut egui::Ui) -> Vec<ViewEvent> {
        let ctx = ui.ctx().clone();
        self.keys(&ctx);
        let mut events = Vec::new();
        self.bar(ui, &mut events);
        if let Some(left) = self.debounce.remaining(Instant::now()) {
            // The pause after the last key: one repaint, no timer thread.
            ctx.request_repaint_after(left);
        }

        match &self.phase {
            Phase::Failed(error) => {
                ui.centered_and_justified(|ui| {
                    ui.label(format!("The log could not be read: {error}"));
                });
                return events;
            }
            Phase::Loading => {
                ui.centered_and_justified(|ui| {
                    ui.label("Reading the log...");
                });
                return events;
            }
            Phase::Ready => {}
        }
        if self.timeline.is_empty()
            && self.timeline.at_newest()
            && self.timeline.at_oldest()
            && !self.reloading
        {
            let text = self.empty_text();
            ui.centered_and_justified(|ui| {
                ui.label(text);
            });
            return events;
        }

        if self.details {
            let cursor = self.selection.cursor();
            egui::Panel::right("details")
                .resizable(true)
                .default_size(340.0)
                .min_size(220.0)
                .show(ui, |ui| {
                    match cursor.and_then(|at| self.timeline.row_at(at)) {
                        Some(row) => crate::details::show(ui, row),
                        None => {
                            ui.label("Select a row to see everything stored about it.");
                        }
                    }
                });
        }
        self.table(ui);
        self.pill(&ctx);
        if let Some(action) = self.action.take() {
            self.act(&ctx, action, &mut events);
        }
        events
    }

    /// Carries out what a row's context menu asked for.
    fn act(&mut self, ctx: &egui::Context, action: RowAction, events: &mut Vec<ViewEvent>) {
        match action {
            RowAction::CopyPath(path) => ctx.copy_text(path),
            RowAction::CopyRows => ctx.copy_text(crate::rows::tsv(self.selected_rows())),
            RowAction::FilterFolder(folder) => {
                self.spec.subtree = Some(folder);
                self.spec_changed();
            }
            RowAction::FilterFile(name) => {
                self.spec.name = name;
                self.spec_changed();
            }
            RowAction::FilterKind(chip) => {
                self.spec.kinds = BTreeSet::from([chip]);
                self.spec_changed();
            }
            RowAction::Open { path, select } => events.push(ViewEvent::Open { path, select }),
        }
    }

    /// What the grid says when there is nothing to show.
    fn empty_text(&self) -> String {
        match self.spec.narrowest() {
            Some(narrowest) if self.timeline.total() == 0 && !self.spec.is_default() => {
                format!("No matching events. {narrowest} narrows the view the most.")
            }
            _ => String::from("No events recorded yet. Changes appear here as they happen."),
        }
    }

    /// The filter bar: search, kind chips, time span, the advanced row, the
    /// hidden counter, the scan progress and the column chooser.
    fn bar(&mut self, ui: &mut egui::Ui, events: &mut Vec<ViewEvent>) {
        let mut changed = false;
        ui.horizontal_wrapped(|ui| {
            let label = ui.label("Search");
            let search = ui.add(
                egui::TextEdit::singleline(&mut self.spec.name)
                    .id(egui::Id::unique("search"))
                    .hint_text("name contains...")
                    .desired_width(200.0),
            );
            let search = search.labelled_by(label.id);
            if self.focus_search {
                self.focus_search = false;
                search.request_focus();
            }
            changed |= search.changed();
            for chip in Chip::ALL {
                let on = self.spec.kinds.contains(&chip);
                if ui.selectable_label(on, chip.label()).clicked() {
                    if on {
                        self.spec.kinds.remove(&chip);
                    } else {
                        self.spec.kinds.insert(chip);
                    }
                    changed = true;
                }
            }
            changed |= span_menu(ui, &mut self.spec.span);
            if ui.selectable_label(self.more, "More").clicked() {
                self.more = !self.more;
            }
            if ui
                .selectable_label(self.compact, "Compact")
                .on_hover_text(
                    "Fold renames, saves and bursts of changes to one file behind \
                     their newest row. Every raw event stays in the log; open a \
                     group to see them. Groups are found among the rows loaded \
                     now, so the counts are approximate.",
                )
                .clicked()
            {
                self.compact = !self.compact;
                self.expanded.clear();
                self.expanded_gen += 1;
            }
            if ui.selectable_label(self.details, "Details").clicked() {
                self.details = !self.details;
                events.push(ViewEvent::DetailsToggled(self.details));
            }
        });
        if self.more {
            changed |= self.advanced(ui);
        }
        ui.horizontal_wrapped(|ui| {
            events.extend(self.columns_menu(ui));
            self.export_ui(ui, events);
            let status = self.status_text().to_owned();
            ui.label(status);
            if let Some(hidden) = self.hidden.filter(|hidden| *hidden > 0) {
                let verb = if self.spec.show_hidden {
                    "Hide"
                } else {
                    "Show"
                };
                ui.label(format!("{} hidden by the view.", group_digits(hidden)));
                if ui.button(verb).clicked() {
                    self.spec.show_hidden = !self.spec.show_hidden;
                    changed = true;
                }
            }
            if self.timeline.busy() && self.scanned > 0 {
                let of = self.timeline.extent().map_or(String::new(), |extent| {
                    format!(" of {}", group_digits(extent.span()))
                });
                ui.label(format!("Scanned {}{of}", group_digits(self.scanned)));
                if ui.button("Stop").clicked() {
                    self.stop_scan();
                }
            }
        });
        if changed {
            self.spec_changed();
        }
    }

    /// The advanced row: glob or regex, watch, folder and the text form.
    /// Returns whether the spec changed.
    fn advanced(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        ui.horizontal_wrapped(|ui| {
            let (mut kind, mut text) = self
                .spec
                .pattern
                .clone()
                .unwrap_or((PatternKind::Glob, String::new()));
            let before = (kind, text.clone());
            egui::ComboBox::from_label("Match")
                .selected_text(match kind {
                    PatternKind::Glob => "Glob",
                    PatternKind::Regex => "Regex",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut kind, PatternKind::Glob, "Glob");
                    ui.selectable_value(&mut kind, PatternKind::Regex, "Regex");
                });
            let field = ui.add(
                egui::TextEdit::singleline(&mut text)
                    .id(egui::Id::unique("pattern"))
                    .hint_text("*.tmp or \\.tmp$")
                    .desired_width(220.0),
            );
            if (kind, text.clone()) != before || field.changed() {
                self.spec.pattern = (!text.is_empty()).then_some((kind, text));
                changed = true;
            }
            if let Some(error) = self
                .spec_error
                .as_ref()
                .filter(|error| error.field == Field::Pattern)
            {
                ui.colored_label(ui.visuals().error_fg_color, format!("x {error}"));
            }
            if !self.roots.is_empty() {
                let current = self.spec.root.clone();
                egui::ComboBox::from_label("Watch")
                    .selected_text(current.as_deref().unwrap_or("All watches"))
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(current.is_none(), "All watches")
                            .clicked()
                        {
                            self.spec.root = None;
                            changed = true;
                        }
                        for root in &self.roots {
                            let chosen = current.as_deref() == Some(root.as_str());
                            if ui.selectable_label(chosen, root).clicked() {
                                self.spec.root = Some(root.clone());
                                changed = true;
                            }
                        }
                    });
            }
            if let Some(folder) = self.spec.subtree.clone() {
                ui.label(format!("In {folder}"));
                if ui.button("Clear folder").clicked() {
                    self.spec.subtree = None;
                    changed = true;
                }
            }
            if let Some(error) = self
                .spec_error
                .as_ref()
                .filter(|error| error.field != Field::Pattern)
            {
                ui.colored_label(ui.visuals().error_fg_color, format!("x {error}"));
            }
        });
        ui.horizontal_wrapped(|ui| {
            let label = ui.label("Filter text");
            let field = ui.add(
                egui::TextEdit::singleline(&mut self.text)
                    .id(egui::Id::unique("filter-text"))
                    .desired_width(480.0),
            );
            let field = field.labelled_by(label.id);
            if field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                match FilterSpec::parse(&self.text) {
                    Ok(spec) => {
                        self.spec = spec;
                        self.text_error = None;
                        changed = true;
                    }
                    Err(error) => self.text_error = Some(error.to_string()),
                }
            }
            if ui.button("Copy filter").clicked() {
                ui.ctx().copy_text(self.spec.to_text());
            }
            if ui.button("Clear filter").clicked() {
                self.spec = FilterSpec::default();
                changed = true;
            }
            if let Some(error) = &self.text_error {
                ui.colored_label(ui.visuals().error_fg_color, format!("x {error}"));
            }
        });
        changed
    }

    /// The column chooser; returns the new column names when they changed.
    fn columns_menu(&mut self, ui: &mut egui::Ui) -> Option<ViewEvent> {
        let mut event = None;
        ui.menu_button("Columns", |ui| {
            let mut changed = false;
            for col in Col::ALL {
                let mut on = self.columns.contains(&col);
                if ui.checkbox(&mut on, col.title()).changed() {
                    changed = true;
                    if on {
                        self.columns.push(col);
                    } else if self.columns.len() > 1 {
                        self.columns.retain(|c| *c != col);
                    }
                }
            }
            if changed {
                // The chooser lists them in a fixed order.
                self.columns.sort_by_key(|col| {
                    Col::ALL.iter().position(|c| c == col).unwrap_or(usize::MAX)
                });
                event = Some(ViewEvent::ColumnsChanged(
                    self.columns.iter().map(|c| c.name().to_owned()).collect(),
                ));
            }
        });
        event
    }

    /// Makes the folds again when the held rows, the mode or the opened groups
    /// changed since they were made.
    fn refresh_folds(&mut self) {
        let key = (self.timeline.revision(), self.compact, self.expanded_gen);
        if self.folds_key == Some(key) {
            return;
        }
        self.folds = if self.compact {
            Folds::build(self.timeline.iter(), self.timeline.top(), &self.expanded)
        } else {
            Folds::none()
        };
        // Only the heads that are still there stay opened.
        let folds = &self.folds;
        self.expanded.retain(|id| folds.is_head(*id));
        self.folds_key = Some(key);
    }

    fn table(&mut self, ui: &mut egui::Ui) {
        ui.style_mut().interaction.selectable_labels = false;
        self.refresh_folds();
        let columns: Vec<Column> = self
            .columns
            .iter()
            .map(|col| {
                Column::new(col.width())
                    .resizable(true)
                    .range(48.0..=2400.0)
            })
            .collect();
        let mut table = Table::new()
            .id_salt("events")
            .num_rows(self.timeline.total())
            .columns(columns)
            .headers(vec![HeaderRow::new(HEADER_HEIGHT)])
            .auto_size_mode(AutoSizeMode::OnParentResize);
        if let Some((row, align)) = self.scroll_to.take() {
            table = table.scroll_to_row(row, Some(align));
        }
        let mut toggle = None;
        let mut grid = Grid {
            timeline: &mut self.timeline,
            worker: &self.worker,
            generation: self.generation,
            filter: &self.filter,
            cancel: &self.cancel,
            selection: &mut self.selection,
            follow: &mut self.follow,
            columns: &self.columns,
            visible: &mut self.visible,
            frozen: self.reloading,
            action: &mut self.action,
            folds: &self.folds,
            toggle: &mut toggle,
        };
        table.show(ui, &mut grid);
        if let Some(head) = toggle
            && !self.expanded.remove(&head)
        {
            self.expanded.insert(head);
        }
        if toggle.is_some() {
            self.expanded_gen += 1;
            ui.ctx().request_repaint();
        } else if self
            .folds_key
            .is_some_and(|key| key.0 != self.timeline.revision())
        {
            // The grid moved the island while drawing: draw again with folds
            // made for the rows it holds now.
            ui.ctx().request_repaint();
        }
    }

    /// The "N new events" pill, over the top of the grid.
    fn pill(&mut self, ctx: &egui::Context) {
        let Some(text) = self.follow.pill() else {
            return;
        };
        let mut clicked = false;
        egui::Area::new(egui::Id::unique("new-events"))
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 72.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                clicked = ui.button(text).clicked();
            });
        if clicked {
            self.resume();
        }
    }

    /// Back to the newest rows, following again.
    pub fn resume(&mut self) {
        self.follow.resume();
        self.selection.clear();
        let fetch = self.timeline.jump_to_newest();
        self.send_page(fetch);
        self.scroll_to = Some((0, egui::Align::TOP));
    }

    fn send_page(&self, fetch: Fetch) {
        self.worker.send(Job::Page {
            generation: self.generation,
            fetch,
            filter: self.filter.clone(),
            cancel: self.cancel.clone(),
        });
    }
}

/// The time span chooser. Returns whether it changed. (A custom range is set
/// with `from:` and `to:` in the filter text.)
fn span_menu(ui: &mut egui::Ui, span: &mut Span) -> bool {
    let before = *span;
    let name = match span {
        Span::Any => "Any time",
        Span::Last5Minutes => "Last 5 minutes",
        Span::LastHour => "Last hour",
        Span::Today => "Today",
        Span::Between { .. } => "Custom range",
    };
    egui::ComboBox::from_label("When")
        .selected_text(name)
        .show_ui(ui, |ui| {
            ui.selectable_value(span, Span::Any, "Any time");
            ui.selectable_value(span, Span::Last5Minutes, "Last 5 minutes");
            ui.selectable_value(span, Span::LastHour, "Last hour");
            ui.selectable_value(span, Span::Today, "Today");
        });
    *span != before
}

/// The table's delegate: borrows what a frame needs from the view.
struct Grid<'a> {
    timeline: &'a mut Timeline<GridRow>,
    worker: &'a Worker,
    generation: u64,
    filter: &'a Filter,
    cancel: &'a CancelFlag,
    selection: &'a mut Selection,
    follow: &'a mut Follow,
    columns: &'a [Col],
    visible: &'a mut Range<u64>,
    /// A new filter waits for its extent: show, but do not extend.
    frozen: bool,
    /// Where a context menu leaves what the user asked for.
    action: &'a mut Option<RowAction>,
    /// The groups folded in the held rows (none in the raw view).
    folds: &'a Folds,
    /// The head whose group the user opened or closed.
    toggle: &'a mut Option<i64>,
}

fn row_tint(ui: &egui::Ui, class: RowClass) -> Option<egui::Color32> {
    let visuals = ui.visuals();
    match class {
        RowClass::Normal => None,
        RowClass::Gap => Some(visuals.warn_fg_color.gamma_multiply(0.22)),
        RowClass::Reconcile => Some(visuals.text_color().gamma_multiply(0.07)),
        RowClass::System => Some(visuals.faint_bg_color),
    }
}

impl TableDelegate for Grid<'_> {
    fn prepare(&mut self, info: &PrefetchInfo) {
        *self.visible = info.visible_rows.clone();
        if self.frozen {
            return;
        }
        self.timeline.keep_near(&info.visible_rows);
        for fetch in self.timeline.plan(&info.visible_rows) {
            self.worker.send(Job::Page {
                generation: self.generation,
                fetch,
                filter: self.filter.clone(),
                cancel: self.cancel.clone(),
            });
        }
        if !self.timeline.is_empty() {
            let at_top = info.visible_rows.start == 0 && self.timeline.at_newest();
            self.follow.moved(at_top);
        }
    }

    fn header_cell_ui(&mut self, ui: &mut egui::Ui, cell: &HeaderCellInfo) {
        if let Some(col) = self.columns.get(cell.col_range.start) {
            ui.strong(col.title()).on_hover_text(col.tip());
        }
    }

    fn cell_ui(&mut self, ui: &mut egui::Ui, cell: &CellInfo) {
        let Some(col) = self.columns.get(cell.col_nr).copied() else {
            return;
        };
        let rect = ui.max_rect();
        if self
            .timeline
            .row_at(cell.row_nr)
            .is_some_and(|row| self.folds.of(row.id) == Fold::Folded)
        {
            // Folded away: no height, nothing to draw.
            return;
        }
        if self.selection.contains(cell.row_nr) {
            ui.painter()
                .rect_filled(rect, 0.0, ui.visuals().selection.bg_fill);
        }
        let Some(row) = self.timeline.row_at(cell.row_nr) else {
            if col == Col::Path || cell.col_nr == 0 {
                ui.weak("...");
            }
            return;
        };
        if !self.selection.contains(cell.row_nr)
            && let Some(tint) = row_tint(ui, row.class)
        {
            ui.painter().rect_filled(rect, 0.0, tint);
        }
        // Before the content, so a button in the cell is on top of it.
        let response = ui.interact(
            rect,
            ui.scope_id().with(("row", cell.row_nr)),
            egui::Sense::click(),
        );
        match col {
            Col::Time => {
                ui.add(egui::Label::new(&row.time).truncate())
                    .on_hover_text(&row.time_utc);
            }
            Col::Kind => match self.folds.of(row.id) {
                Fold::Plain | Fold::Folded => {
                    ui.add(egui::Label::new(&row.kind).truncate());
                }
                Fold::Inside => {
                    ui.horizontal(|ui| {
                        ui.add_space(14.0);
                        ui.add(egui::Label::new(&row.kind).truncate());
                    });
                }
                Fold::Head {
                    kind,
                    members,
                    expanded,
                } => {
                    ui.horizontal(|ui| {
                        let arrow = if expanded { "v" } else { ">" };
                        let badge = ui
                            .add(egui::Button::new(format!("{arrow} {members}")).frame(false))
                            .on_hover_ui(|ui| {
                                ui.label(Fold::badge(members, kind));
                            });
                        if badge.clicked() {
                            *self.toggle = Some(row.id);
                        }
                        ui.add(egui::Label::new(&row.kind).truncate());
                    });
                }
            },
            Col::Path => path_cell(ui, row),
            Col::OldPath => {
                ui.add(egui::Label::new(&row.old_path).truncate());
            }
            Col::Size => {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add(egui::Label::new(&row.size).truncate());
                });
            }
            Col::Source => {
                ui.add(egui::Label::new(row.source).truncate());
            }
        }
        if response.clicked() {
            let shift = ui.input(|input| input.modifiers.shift);
            self.selection.click(cell.row_nr, shift);
        }
        if response.secondary_clicked() && !self.selection.contains(cell.row_nr) {
            self.selection.click(cell.row_nr, false);
        }
        let many = self
            .selection
            .range()
            .map_or(1, |range| range.end() - range.start() + 1);
        response.context_menu(|ui| {
            if let Some(action) = row_menu(ui, row, many) {
                *self.action = Some(action);
            }
        });
    }

    fn default_row_height(&self) -> f32 {
        ROW_HEIGHT
    }

    #[expect(
        clippy::cast_precision_loss,
        reason = "a scroll offset; the default of egui_table rounds the same way"
    )]
    fn row_top_offset(&self, _ctx: &egui::Context, _table_id: egui::Id, row_nr: u64) -> f32 {
        // Folded rows have no height. Past the held rows the count stops, so
        // the offset stays a multiple of the row height.
        self.folds.shown_before(row_nr) as f32 * ROW_HEIGHT
    }
}

/// The context menu of a row. Every entry has a name; none opens, reads or
/// runs the file: only its folder is shown.
fn row_menu(ui: &mut egui::Ui, row: &GridRow, many: u64) -> Option<RowAction> {
    let mut chosen = None;
    if ui.button("Copy path").clicked() {
        chosen = Some(RowAction::CopyPath(row.path.clone()));
    }
    let label = if many > 1 {
        format!("Copy {} rows", group_digits(many))
    } else {
        String::from("Copy row")
    };
    if ui.button(label).clicked() {
        chosen = Some(RowAction::CopyRows);
    }
    ui.separator();
    if ui.button("Show in Explorer").clicked() {
        chosen = Some(RowAction::Open {
            path: row.path.clone(),
            select: true,
        });
    }
    if ui.button("Open containing folder").clicked() {
        chosen = Some(RowAction::Open {
            path: row.path.clone(),
            select: false,
        });
    }
    ui.separator();
    let folder = if row.is_dir {
        Some(row.path.as_str())
    } else {
        crate::shellpath::parent(&row.path)
    };
    if let Some(folder) = folder
        && ui.button("Filter to this folder").clicked()
    {
        chosen = Some(RowAction::FilterFolder(folder.to_owned()));
    }
    if ui.button("Filter to this file").clicked() {
        chosen = Some(RowAction::FilterFile(row.file_name().to_owned()));
    }
    if let Some(chip) = Chip::of(row.raw.kind)
        && ui
            .button(format!("Filter to {} rows", chip.label()))
            .clicked()
    {
        chosen = Some(RowAction::FilterKind(chip));
    }
    if chosen.is_some() {
        ui.close();
    }
    chosen
}

/// The path with its middle elided: the directory gives way, the file name
/// stays, and the whole path is in the tooltip.
fn path_cell(ui: &mut egui::Ui, row: &GridRow) {
    let name = row.file_name();
    let escaped = row.escaped;
    let name_width = ui
        .painter()
        .layout_no_wrap(
            name.to_owned(),
            egui::TextStyle::Body.resolve(ui.style()),
            ui.visuals().text_color(),
        )
        .size()
        .x;
    let directory_width = (ui.available_width() - name_width - 8.0).max(0.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        if escaped {
            ui.colored_label(ui.visuals().warn_fg_color, "! ")
                .on_hover_text("The name has characters that were made visible");
        }
        if directory_width > 8.0 && !row.directory().is_empty() {
            ui.allocate_ui(egui::vec2(directory_width, ROW_HEIGHT), |ui| {
                ui.add(egui::Label::new(row.directory()).truncate());
            });
        }
        ui.add(egui::Label::new(name).extend())
            .on_hover_text(&row.path);
    });
}

#[cfg(test)]
mod tests;
