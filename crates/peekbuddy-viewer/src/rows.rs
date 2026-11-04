//! Rows as the grid shows them (#66).
//!
//! A [`GridRow`] is built once, on the query worker, when its page arrives:
//! the strings the grid draws are ready, so a frame formats nothing. Names
//! are display-escaped (F17, #39): a lone surrogate or an invisible format
//! character such as a right-to-left override shows as `\u{202E}`, and the
//! row says it was escaped, because a spoofed name must look spoofed.

use peekbuddy_core::event::{Flags, GapReason, Kind, Source, WatchState};
use peekbuddy_core::export::{iso_local, iso_utc};
use peekbuddy_core::name::{WideStr, escape_for_display, needs_escape};
use peekbuddy_core::query::Row;

use crate::timeline::Identified;

/// How the UTC offset of an instant is found (seconds east of UTC).
pub type OffsetFn = dyn Fn(u64) -> i32 + Send + Sync;

/// What kind of line a row is, for its background and its marker column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowClass {
    /// A change to a file.
    Normal,
    /// A gap in the stream, or a failure: something may be missing.
    Gap,
    /// Found by the reconciler after a gap.
    Reconcile,
    /// PeekBuddy's own lifecycle: started, stopped, rotated, settings.
    System,
}

impl RowClass {
    /// The word for the marker column. Always text: colour alone is not
    /// enough.
    #[must_use]
    pub const fn marker(self) -> &'static str {
        match self {
            Self::Normal => "",
            Self::Gap => "gap",
            Self::Reconcile => "reconciled",
            Self::System => "engine",
        }
    }
}

/// The columns of a copied row, tab separated.
pub const TSV_HEADER: &str =
    "id\ttime_utc\ttime_local\tkind\tsource\tpath\told_path\tis_dir\tsize\tattrs\traw_mask\troot";

/// The rows as tab-separated text with a header line: what Ctrl+C copies.
/// Tabs, line ends and control characters in a name are written out
/// (`\t`, `\n`, ...), so every row has exactly the columns of the header.
#[must_use]
pub fn tsv<'a>(rows: impl IntoIterator<Item = &'a GridRow>) -> String {
    let mut out = String::from(TSV_HEADER);
    for row in rows {
        out.push('\n');
        row.write_tsv(&mut out);
    }
    out.push('\n');
    out
}

/// A stored row with the text the grid draws.
#[derive(Debug, Clone)]
pub struct GridRow {
    /// The event id.
    pub id: i64,
    /// Local time, `YYYY-MM-DD HH:MM:SS.mmm`.
    pub time: String,
    /// The same instant in UTC with 100 ns precision (the tooltip).
    pub time_utc: String,
    /// One ASCII character in front of the label, so the kind is told by
    /// shape and word, never by colour.
    pub glyph: &'static str,
    /// The kind in words.
    pub label: &'static str,
    /// The glyph and the words together, ready to draw.
    pub kind: String,
    /// The line class.
    pub class: RowClass,
    /// The path, display-escaped.
    pub path: String,
    /// Where the last component starts in `path` (for middle elision).
    pub name_start: usize,
    /// The path before a rename, display-escaped; empty when there is none.
    pub old_path: String,
    /// The size in words (`1.5 MB`), empty when unknown.
    pub size: String,
    /// Where the event came from.
    pub source: &'static str,
    /// The name needed escaping: show a warning badge.
    pub escaped: bool,
    /// The row is a directory.
    pub is_dir: bool,
    /// The stored row, for the details pane and for copying.
    pub raw: Row,
}

impl Identified for GridRow {
    fn id(&self) -> i64 {
        self.id
    }
}

fn gap_text(reason: GapReason) -> &'static str {
    match reason {
        GapReason::BufferOverflow => "Gap: buffer overflow",
        GapReason::NotifyEnumDir => "Gap: directory must be listed again",
        GapReason::JournalEntryDeleted => "Gap: journal entries deleted",
        GapReason::JournalIdChanged => "Gap: journal replaced",
        GapReason::ParserError => "Gap: unreadable change record",
        GapReason::EngineNotRunning => "Gap: engine was not running",
        GapReason::RingFull => "Gap: writer too slow",
    }
}

fn watch_text(state: WatchState) -> &'static str {
    match state {
        WatchState::Arming => "Watch: arming",
        WatchState::Live => "Watch: live",
        WatchState::Overflowed => "Watch: overflowed",
        WatchState::Reconciling => "Watch: reconciling",
        WatchState::SuspendedByUser => "Watch: paused",
        WatchState::SuspendedQueryRemove => "Watch: suspended (volume removal)",
        WatchState::Lost => "Watch: lost",
        WatchState::Restored => "Watch: restored",
        WatchState::SuspendedLocked => "Watch: suspended (volume locked)",
    }
}

/// The glyph, the words and the class of a kind.
#[must_use]
pub fn describe(kind: Kind) -> (&'static str, &'static str, RowClass) {
    use RowClass::{Gap, Normal, Reconcile, System};
    match kind {
        Kind::Created => ("+", "Created", Normal),
        Kind::Modified => ("~", "Modified", Normal),
        Kind::Deleted => ("-", "Deleted", Normal),
        Kind::RenamedFrom => (">", "Renamed from", Normal),
        Kind::RenamedTo => (">", "Renamed to", Normal),
        Kind::AttributesChanged => ("~", "Attributes changed", Normal),
        Kind::SecurityChanged => ("~", "Security changed", Normal),
        Kind::StreamChanged => ("~", "Stream changed", Normal),
        Kind::LinkChanged => ("~", "Link changed", Normal),
        Kind::Overflow(reason) => ("!", gap_text(reason), Gap),
        Kind::WatchStateChanged(state) => ("*", watch_text(state), System),
        Kind::ReconciledCreated => ("+", "Found created", Reconcile),
        Kind::ReconciledModified => ("~", "Found modified", Reconcile),
        Kind::ReconciledDeleted => ("-", "Found deleted", Reconcile),
        Kind::ReconciledRenamed => (">", "Found renamed", Reconcile),
        Kind::ReconcileStarted => ("*", "Reconcile started", Reconcile),
        Kind::ReconcileFinished => ("*", "Reconcile finished", Reconcile),
        Kind::BaselineEstablished => ("*", "Baseline established", Reconcile),
        Kind::AccessDenied => ("!", "Access denied", Gap),
        Kind::EngineAbnormalExit => ("!", "Engine ended abnormally", Gap),
        Kind::StoreDegraded => ("!", "Store degraded", Gap),
        Kind::StoreRecovered => ("*", "Store recovered", System),
        Kind::Other => ("?", "Other", Normal),
        Kind::EngineStarted => ("*", "Engine started", System),
        Kind::EngineStopped => ("*", "Engine stopped", System),
        Kind::SegmentRotated => ("*", "Day segment started", System),
        Kind::RetentionPurged => ("*", "Old data removed", System),
        Kind::SettingsChanged => ("*", "Settings changed", System),
    }
}

/// The source in words.
#[must_use]
pub const fn source_text(source: Source) -> &'static str {
    match source {
        Source::Rdcw => "RDCW",
        Source::Usn => "USN",
        Source::Reconcile => "RECONCILE",
        Source::System => "SYSTEM",
    }
}

/// A size in words: `812 B`, `1.5 KB`, `3.0 GB` (powers of 1024).
#[must_use]
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["KB", "MB", "GB", "TB", "PB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    // Tenths, rounded down, in integers: no float casts.
    let mut tenths = u128::from(bytes) * 10 / 1024;
    let mut unit = 0;
    while tenths >= 10_240 && unit + 1 < UNITS.len() {
        tenths /= 1024;
        unit += 1;
    }
    format!(
        "{}.{} {}",
        tenths / 10,
        tenths % 10,
        UNITS.get(unit).copied().unwrap_or("PB")
    )
}

/// `YYYY-MM-DD HH:MM:SS.mmm` for an instant shifted by `offset` seconds.
#[must_use]
pub fn format_time(ts: u64, offset: i32) -> String {
    // `iso_local` is `YYYY-MM-DDTHH:MM:SS.fffffff+HH:MM`.
    let iso = iso_local(ts, offset);
    let date = iso.get(..10).unwrap_or_default();
    let clock = iso.get(11..23).unwrap_or_default();
    format!("{date} {clock}")
}

/// The watch root of a row, display-escaped.
#[must_use]
pub fn root_text(row: &Row) -> String {
    escape_for_display(WideStr::new(&row.root))
}

fn show(units: &[u16]) -> (String, bool) {
    let name = WideStr::new(units);
    (escape_for_display(name), needs_escape(name))
}

/// Where the last component of `path` starts (after the last `\` or `/`).
fn last_component(path: &str) -> usize {
    path.rfind(['\\', '/']).map_or(0, |at| at + 1)
}

impl GridRow {
    /// The grid's view of a stored row.
    #[must_use]
    pub fn new(raw: Row, offset: &OffsetFn) -> Self {
        let (glyph, label, class) = describe(raw.kind);
        let (path, escaped) = show(&raw.path);
        let (old_path, old_escaped) = show(&raw.old_path);
        let name_start = last_component(&path);
        Self {
            id: raw.id,
            time: format_time(raw.ts, offset(raw.ts)),
            time_utc: iso_utc(raw.ts),
            glyph,
            label,
            kind: format!("{glyph} {label}"),
            class,
            name_start,
            size: raw.size.map_or_else(String::new, format_size),
            source: source_text(raw.source),
            escaped: escaped || old_escaped,
            is_dir: raw.is_dir,
            path,
            old_path,
            raw,
        }
    }

    /// Appends this row as one tab-separated line (no line end).
    pub fn write_tsv(&self, out: &mut String) {
        use std::fmt::Write as _;

        use peekbuddy_core::decode::tsv_cell;

        let cells = [
            self.id.to_string(),
            self.time_utc.clone(),
            self.time.clone(),
            self.label.to_owned(),
            self.source.to_owned(),
            tsv_cell(&self.path),
            tsv_cell(&self.old_path),
            self.is_dir.to_string(),
            self.raw
                .size
                .map_or_else(String::new, |size| size.to_string()),
            self.raw
                .attrs
                .map_or_else(String::new, |attrs| format!("{attrs:#010x}")),
            format!("{:#010x}", self.raw.raw_mask),
            tsv_cell(&root_text(&self.raw)),
        ];
        for (at, cell) in cells.iter().enumerate() {
            if at > 0 {
                out.push('\t');
            }
            let _ = write!(out, "{cell}");
        }
    }

    /// The directory part of the path, with its trailing separator.
    #[must_use]
    pub fn directory(&self) -> &str {
        self.path.get(..self.name_start).unwrap_or_default()
    }

    /// The last component of the path.
    #[must_use]
    pub fn file_name(&self) -> &str {
        self.path.get(self.name_start..).unwrap_or_default()
    }

    /// Whether the row was hidden by default (a cloud sync churn row that a
    /// view with the preset off shows).
    #[must_use]
    pub fn is_cloud_churn(&self) -> bool {
        self.raw.flags & Flags::CLOUD_CHURN.bits() != 0
    }
}

#[cfg(test)]
mod tests {
    use peekbuddy_core::event::{GapReason, Kind, Source};
    use peekbuddy_core::query::Row;

    use super::{GridRow, RowClass, describe, format_size, format_time};

    fn units(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    fn row(kind: Kind, path: &[u16]) -> Row {
        Row {
            id: 7,
            // 2026-10-26 12:00:00 UTC.
            ts: 134_374_896_000_000_000,
            kind,
            source: Source::Rdcw,
            raw_mask: 3,
            is_dir: false,
            size: Some(1_536),
            attrs: None,
            reparse_tag: 0,
            file_id: 0,
            name: Vec::new(),
            path: path.to_vec(),
            old_path: Vec::new(),
            root: units("C:\\x"),
            flags: 0,
        }
    }

    #[test]
    fn a_row_is_ready_to_draw() {
        let grid = GridRow::new(row(Kind::Created, &units(r"C:\work\report.txt")), &|_| 7200);
        assert_eq!(grid.time, "2026-10-26 14:00:00.000");
        assert_eq!(grid.time_utc, "2026-10-26T12:00:00.0000000Z");
        assert_eq!((grid.glyph, grid.label), ("+", "Created"));
        assert_eq!(grid.kind, "+ Created");
        assert_eq!(grid.class, RowClass::Normal);
        assert_eq!(grid.path, r"C:\work\report.txt");
        assert_eq!(grid.directory(), r"C:\work\");
        assert_eq!(grid.file_name(), "report.txt");
        assert_eq!(grid.size, "1.5 KB");
        assert_eq!(grid.source, "RDCW");
        assert!(!grid.escaped);
    }

    #[test]
    fn a_spoofed_or_broken_name_is_escaped_and_flagged() {
        // "invoice\u{202E}fdp.exe": a right-to-left override.
        let grid = GridRow::new(
            row(Kind::Created, &units("C:\\x\\invoice\u{202E}fdp.exe")),
            &|_| 0,
        );
        assert_eq!(grid.path, "C:\\x\\invoice\\u{202E}fdp.exe");
        assert!(grid.escaped);
        // A lone surrogate is shown, not lost.
        let mut broken = units("C:\\x\\a");
        broken.push(0xD800);
        let grid = GridRow::new(row(Kind::Created, &broken), &|_| 0);
        assert_eq!(grid.path, "C:\\x\\a\\u{D800}");
        assert!(grid.escaped);
    }

    #[test]
    fn a_path_without_a_separator_is_all_name() {
        let grid = GridRow::new(row(Kind::Modified, &units("report.txt")), &|_| 0);
        assert_eq!(grid.directory(), "");
        assert_eq!(grid.file_name(), "report.txt");
    }

    #[test]
    fn every_kind_has_a_glyph_a_label_and_the_gap_rows_are_gaps() {
        let (glyph, label, class) = describe(Kind::Overflow(GapReason::BufferOverflow));
        assert_eq!((glyph, class), ("!", RowClass::Gap));
        assert!(label.starts_with("Gap"));
        assert_eq!(RowClass::Gap.marker(), "gap");
        assert_eq!(RowClass::Normal.marker(), "");
        for kind in [
            Kind::Created,
            Kind::Modified,
            Kind::Deleted,
            Kind::RenamedFrom,
            Kind::RenamedTo,
            Kind::Other,
            Kind::EngineStarted,
        ] {
            let (glyph, label, _) = describe(kind);
            assert!(glyph.is_ascii() && glyph.len() == 1, "{kind:?}");
            assert_ne!(label.len(), 0);
        }
    }

    #[test]
    fn a_copied_row_has_the_columns_of_the_header_and_escapes_what_would_break_them() {
        use super::{TSV_HEADER, tsv};

        let mut plain = row(Kind::Created, &units("C:\\x\\a.txt"));
        plain.attrs = Some(0x20);
        let tabbed = row(Kind::Modified, &units("C:\\x\\a\tb\nc.txt"));
        let mut broken = units("C:\\x\\lone");
        broken.push(0xD800);
        let lone = row(Kind::Deleted, &broken);
        let rows: Vec<GridRow> = [plain, tabbed, lone]
            .into_iter()
            .map(|raw| GridRow::new(raw, &|_| 0))
            .collect();
        let text = tsv(&rows);
        for line in text.lines() {
            assert_eq!(
                line.matches('\t').count(),
                TSV_HEADER.matches('\t').count(),
                "{line}"
            );
        }
        assert_eq!(text.lines().count(), 4, "the header and three rows");
        insta::assert_snapshot!(text);
    }

    proptest::proptest! {
        #[test]
        fn any_name_gives_a_row_with_exactly_the_columns(
            name in proptest::collection::vec(proptest::prelude::any::<u16>(), 0..40)
        ) {
            use super::{TSV_HEADER, tsv};
            let grid = GridRow::new(row(Kind::Created, &name), &|_| 0);
            let text = tsv(std::iter::once(&grid));
            let body = text.lines().nth(1).unwrap_or_default();
            proptest::prop_assert_eq!(body.matches('\t').count(), TSV_HEADER.matches('\t').count());
            proptest::prop_assert_eq!(text.lines().count(), 2);
        }
    }

    #[test]
    fn sizes_are_in_words() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(10 * 1024 * 1024), "10.0 MB");
        assert_eq!(format_size(3 * 1024 * 1024 * 1024), "3.0 GB");
        assert_eq!(format_size(u64::MAX), "16383.9 PB", "rounds down, never up");
    }

    #[test]
    fn times_are_local_with_milliseconds() {
        let noon = 134_374_896_000_000_000;
        assert_eq!(format_time(noon, 0), "2026-10-26 12:00:00.000");
        assert_eq!(
            format_time(noon + 1_234_567, -3600),
            "2026-10-26 11:00:00.123"
        );
    }
}
