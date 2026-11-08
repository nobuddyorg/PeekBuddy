//! The stats panel (#82): where the churn comes from.
//!
//! Events per hour and watch root for the last day, drawn as a sparkline
//! with the egui painter (no chart crate, F20), and the busiest paths and
//! extensions of an hour from the writer's sketches. Both come from small
//! tables the writer keeps as it commits, never from counting the log, so the
//! panel costs the same on a 10-million-row log as on an empty one. The
//! sketch counts are approximate by design and the panel says so.
//!
//! Hourly, not per minute: the per-minute live series needs the engine to
//! push its counters (the live tail, #52 and #65), which it does not yet.
//! Top processes need `proc_id` (#80), which nothing fills yet.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use peekbuddy_store::read::Reader;
use peekbuddy_store::read::stats::{HourChurn, HourlyCount};
use peekbuddy_store::schema::TICKS_PER_HOUR;

/// Hours the sparklines span.
pub const HOURS: usize = 24;

/// The same, for tick arithmetic.
const SPAN: i64 = 24;

/// Rows of each top list.
pub const TOP: usize = 10;

/// One root's events per hour, oldest hour first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootSeries {
    /// The root, as stored.
    pub root: String,
    /// [`HOURS`] counts, the last one for the hour that contains `now`.
    pub per_hour: Vec<u64>,
}

impl RootSeries {
    /// The busiest hour: its index and count.
    #[must_use]
    pub fn peak(&self) -> (usize, u64) {
        self.per_hour
            .iter()
            .copied()
            .enumerate()
            .fold(
                (0, 0),
                |best, (i, n)| if n > best.1 { (i, n) } else { best },
            )
    }

    /// The events of all the hours.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.per_hour.iter().sum()
    }
}

/// The hour (ticks) that starts at or before `now`.
#[must_use]
pub const fn hour_of(now: i64) -> i64 {
    now.div_euclid(TICKS_PER_HOUR) * TICKS_PER_HOUR
}

/// The first hour a series for `now` covers.
#[must_use]
pub const fn first_hour(now: i64) -> i64 {
    hour_of(now) - (SPAN - 1) * TICKS_PER_HOUR
}

/// Groups the counts per root into [`HOURS`] slots ending with the hour of
/// `now`; counts outside the window are ignored.
#[must_use]
pub fn series(counts: &[HourlyCount], now: i64) -> Vec<RootSeries> {
    let first = first_hour(now);
    let mut by_root: BTreeMap<&str, Vec<u64>> = BTreeMap::new();
    for count in counts {
        let slot = (count.hour_ts - first).div_euclid(TICKS_PER_HOUR);
        let Ok(slot) = usize::try_from(slot) else {
            continue;
        };
        if slot >= HOURS || (count.hour_ts - first).rem_euclid(TICKS_PER_HOUR) != 0 {
            continue;
        }
        let values = by_root
            .entry(count.root.as_str())
            .or_insert_with(|| vec![0; HOURS]);
        if let Some(value) = values.get_mut(slot) {
            *value += count.events;
        }
    }
    by_root
        .into_iter()
        .map(|(root, per_hour)| RootSeries {
            root: root.to_owned(),
            per_hour,
        })
        .collect()
}

/// The points of a sparkline in a box: x spread evenly, y from the bottom
/// (zero) to the top (the busiest hour). A flat zero series is a line along
/// the bottom.
#[must_use]
#[expect(
    clippy::cast_precision_loss,
    reason = "pixel positions of at most a few hundred points from hour counts"
)]
pub fn spark_points(values: &[u64], width: f32, height: f32) -> Vec<(f32, f32)> {
    let top = values.iter().copied().max().unwrap_or(0).max(1);
    let steps = values.len().saturating_sub(1).max(1);
    values
        .iter()
        .enumerate()
        .map(|(i, &value)| {
            let x = width * i as f32 / steps as f32;
            let y = height - height * value as f32 / top as f32;
            (x, y)
        })
        .collect()
}

/// `HH:00` of an hour, shifted by `offset` seconds from UTC.
#[must_use]
pub fn hour_label(hour_ts: i64, offset: i32) -> String {
    let seconds = hour_ts.div_euclid(10_000_000) + i64::from(offset);
    let hour = seconds.div_euclid(3_600).rem_euclid(24);
    format!("{hour:02}:00")
}

/// What the loading thread leaves.
#[derive(Default)]
struct Shared {
    counts: Vec<HourlyCount>,
    churn: Vec<HourChurn>,
    /// The hour `churn` is for.
    churn_hour: Option<i64>,
    loading: bool,
    error: Option<String>,
}

/// The panel.
pub struct StatsPanel {
    segments: PathBuf,
    waker: Arc<dyn Fn() + Send + Sync>,
    now: fn() -> u64,
    offset: Arc<crate::rows::OffsetFn>,
    shared: Arc<Mutex<Shared>>,
    /// The hour whose top lists are shown; `None` is the latest.
    chosen: Option<i64>,
    asked: bool,
}

impl StatsPanel {
    /// A panel over the segments in `segments`.
    #[must_use]
    pub fn new(
        segments: PathBuf,
        waker: Arc<dyn Fn() + Send + Sync>,
        offset: Arc<crate::rows::OffsetFn>,
        now: fn() -> u64,
    ) -> Self {
        Self {
            segments,
            waker,
            now,
            offset,
            shared: Arc::new(Mutex::new(Shared::default())),
            chosen: None,
            asked: false,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn now(&self) -> i64 {
        i64::try_from((self.now)()).unwrap_or(i64::MAX)
    }

    /// Reads the tables again, on a thread.
    pub fn refresh(&self) {
        {
            let mut shared = self.lock();
            if shared.loading {
                return;
            }
            shared.loading = true;
        }
        let (dir, shared, waker, now, chosen) = (
            self.segments.clone(),
            Arc::clone(&self.shared),
            Arc::clone(&self.waker),
            self.now(),
            self.chosen,
        );
        std::thread::spawn(move || {
            let reader = Reader::new(&dir);
            let counts = reader.hourly_counts(first_hour(now));
            // The latest hour that has anything to show.
            let latest = counts
                .as_ref()
                .ok()
                .and_then(|c| c.iter().map(|c| c.hour_ts).max());
            let hour = chosen.or(latest);
            let churn = hour.map(|hour| reader.hour_churn(hour, TOP));
            let mut shared = shared.lock().unwrap_or_else(PoisonError::into_inner);
            shared.loading = false;
            match (counts, churn) {
                (Ok(counts), Some(Ok(churn))) => {
                    shared.counts = counts;
                    shared.churn = churn;
                    shared.churn_hour = hour;
                    shared.error = None;
                }
                (Ok(counts), None) => {
                    shared.counts = counts;
                    shared.churn.clear();
                    shared.churn_hour = None;
                    shared.error = None;
                }
                (Err(error), _) | (_, Some(Err(error))) => {
                    shared.error = Some(format!("stats: {error}"));
                }
            }
            drop(shared);
            waker();
        });
    }

    /// Draws the panel.
    pub fn show(&mut self, ui: &mut egui::Ui) {
        if !self.asked {
            self.asked = true;
            self.refresh();
        }
        let (counts, churn, churn_hour, error) = {
            let shared = self.lock();
            (
                shared.counts.clone(),
                shared.churn.clone(),
                shared.churn_hour,
                shared.error.clone(),
            )
        };
        ui.horizontal(|ui| {
            ui.strong("Events per hour, last 24 hours");
            if ui.button("Refresh stats").clicked() {
                self.refresh();
            }
        });
        if let Some(error) = error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        let all = series(&counts, self.now());
        if all.is_empty() {
            ui.label("No events recorded in the last 24 hours.");
        }
        for one in &all {
            self.spark_row(ui, one);
        }
        ui.separator();
        self.top_lists(ui, &churn, churn_hour);
    }

    fn spark_row(&self, ui: &mut egui::Ui, one: &RootSeries) {
        let (peak_at, peak) = one.peak();
        let first = first_hour(self.now());
        let peak_hour = first + i64::try_from(peak_at).unwrap_or(0) * TICKS_PER_HOUR;
        let hour = hour_label(
            peak_hour,
            (self.offset)(u64::try_from(peak_hour).unwrap_or(0)),
        );
        let root = one.root.strip_prefix(r"\\?\").unwrap_or(&one.root);
        let summary = format!(
            "{root}: {} events, busiest hour {hour} with {peak}",
            one.total()
        );
        ui.horizontal(|ui| {
            let (response, painter) =
                ui.allocate_painter(egui::vec2(240.0, 36.0), egui::Sense::hover());
            let rect = response.rect.shrink(2.0);
            let color = ui.visuals().text_color();
            painter.rect_stroke(
                response.rect,
                2.0,
                egui::Stroke::new(1.0, ui.visuals().weak_text_color()),
                egui::StrokeKind::Inside,
            );
            let points: Vec<egui::Pos2> = spark_points(&one.per_hour, rect.width(), rect.height())
                .into_iter()
                .map(|(x, y)| egui::pos2(rect.left() + x, rect.top() + y))
                .collect();
            painter.add(egui::Shape::line(points, egui::Stroke::new(1.5, color)));
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::Role::Image,
                    true,
                    format!("Events per hour, {summary}"),
                )
            });
            response.on_hover_text(&summary);
            ui.label(&summary);
        });
    }

    fn top_lists(&mut self, ui: &mut egui::Ui, churn: &[HourChurn], hour: Option<i64>) {
        let Some(hour) = hour else {
            ui.label("Nothing to rank yet.");
            return;
        };
        ui.horizontal(|ui| {
            ui.strong(format!(
                "Busiest in the hour from {}",
                hour_label(hour, (self.offset)(u64::try_from(hour).unwrap_or(0)))
            ));
            if ui.button("Earlier hour").clicked() {
                self.chosen = Some(hour - TICKS_PER_HOUR);
                self.refresh();
            }
            if ui.button("Later hour").clicked() {
                self.chosen = Some(hour + TICKS_PER_HOUR).filter(|h| *h <= hour_of(self.now()));
                self.refresh();
            }
        });
        ui.small("Counts are approximate (a sketch of the busiest 100 per hour and watch).");
        if churn.is_empty() {
            ui.label("Nothing was ranked in this hour.");
        }
        for one in churn {
            let root = one.root.strip_prefix(r"\\?\").unwrap_or(&one.root);
            ui.label(egui::RichText::new(root).strong());
            ui.columns(2, |columns| {
                if let [paths, exts] = columns {
                    list(paths, "Paths", &one.paths);
                    list(exts, "File types", &one.exts);
                }
            });
        }
        ui.small("Top processes need the process column (not recorded yet).");
    }
}

fn list(ui: &mut egui::Ui, title: &str, hits: &[peekbuddy_store::read::stats::Hit]) {
    ui.label(title);
    for hit in hits {
        let error = if hit.error > 0 {
            format!(" (up to {} too high)", hit.error)
        } else {
            String::new()
        };
        ui.label(format!("{}  {}{error}", hit.count, hit.label));
    }
}

#[cfg(test)]
mod tests;
