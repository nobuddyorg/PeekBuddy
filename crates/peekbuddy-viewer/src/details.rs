//! The details pane (#69): everything PeekBuddy stored about one row, with the
//! raw OS values in words.
//!
//! [`fields`] is pure and returns `(label, value)` pairs; [`show`] draws them.
//! Values can be selected and copied. Nothing here reads the file: the pane
//! shows what the log says, and the raw evidence behind it (the mask the OS
//! reported, decoded; the exact UTF-16 of a name on demand).

use std::fmt::Write as _;

use peekbuddy_core::decode;
use peekbuddy_core::event::{Flags, Source};

use crate::rows::{GridRow, root_text};

/// The names of the row flags in `bits`.
fn flag_names(bits: u16) -> String {
    let all = [
        (Flags::CLOUD_CHURN, "cloud sync churn"),
        (Flags::SELF_CHURN, "PeekBuddy's own data"),
        (Flags::TEMP, "temporary file"),
        (Flags::SHORT_DUP, "8.3 short name duplicate"),
        (Flags::NAME_UNRESOLVED, "name unresolved"),
        (Flags::GAP_FOLLOWS, "a gap row follows"),
    ];
    let names: Vec<&str> = all
        .iter()
        .filter(|(flag, _)| bits & flag.bits() != 0)
        .map(|(_, name)| *name)
        .collect();
    if names.is_empty() {
        String::from("none")
    } else {
        names.join(", ")
    }
}

/// The reason mask in words, by what the source reports.
fn raw_mask_text(row: &GridRow) -> String {
    let mask = row.raw.raw_mask;
    let words = match row.raw.source {
        Source::Rdcw => decode::rdcw_action(mask),
        Source::Usn => decode::usn_reasons(mask).join(" | "),
        // The reconciler and the engine write their own values.
        Source::Reconcile | Source::System => String::new(),
    };
    if words.is_empty() {
        format!("{mask:#010x}")
    } else {
        format!("{mask:#010x}  {words}")
    }
}

/// The fields of the pane for `row`, in reading order. Empty values are left
/// out, except the ones whose absence is itself information.
#[must_use]
pub fn fields(row: &GridRow) -> Vec<(&'static str, String)> {
    let raw = &row.raw;
    let mut out: Vec<(&'static str, String)> = vec![
        ("Id", row.id.to_string()),
        ("Time (local)", row.time.clone()),
        ("Time (UTC)", row.time_utc.clone()),
        ("Time (FILETIME)", raw.ts.to_string()),
        ("Kind", row.label.to_owned()),
        ("Source", row.source.to_owned()),
    ];
    let root = root_text(raw);
    if !root.is_empty() {
        out.push(("Watch", root));
    }
    out.push(("Path", row.path.clone()));
    if !row.old_path.is_empty() {
        out.push(("Old path", row.old_path.clone()));
    }
    out.push((
        "Type",
        String::from(if row.is_dir {
            "folder"
        } else {
            "file or unknown"
        }),
    ));
    out.push((
        "Size",
        raw.size.map_or_else(
            || String::from("not reported"),
            |size| format!("{size} bytes ({})", row.size),
        ),
    ));
    match raw.attrs {
        Some(attrs) => {
            let names = decode::attributes(attrs).join(" | ");
            out.push((
                "Attributes",
                if names.is_empty() {
                    format!("{attrs:#010x}")
                } else {
                    format!("{attrs:#010x}  {names}")
                },
            ));
        }
        None => out.push(("Attributes", String::from("not reported"))),
    }
    if raw.reparse_tag != 0 {
        out.push(("Reparse tag", decode::reparse_tag(raw.reparse_tag)));
    }
    if raw.file_id != 0 {
        out.push(("File id", format!("{:032x}", raw.file_id)));
    }
    out.push(("Raw value", raw_mask_text(row)));
    out.push(("Flags", flag_names(raw.flags)));
    if row.escaped {
        out.push((
            "Name",
            String::from(
                "The name has characters that cannot be drawn safely (invisible, \
                 direction-changing or unpaired); they are shown as \\u{...}.",
            ),
        ));
    }
    out
}

/// The exact UTF-16 code units of a path, in hex, four digits each.
#[must_use]
pub fn utf16_hex(units: &[u16]) -> String {
    let mut out = String::with_capacity(units.len() * 5);
    for (at, unit) in units.iter().enumerate() {
        if at > 0 {
            out.push(' ');
        }
        let _ = write!(out, "{unit:04X}");
    }
    out
}

/// Draws the pane for `row`.
pub fn show(ui: &mut egui::Ui, row: &GridRow) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Grid::new("details")
                .num_columns(2)
                .spacing([12.0, 4.0])
                .show(ui, |ui| {
                    for (label, value) in fields(row) {
                        ui.label(egui::RichText::new(label).strong());
                        ui.add(egui::Label::new(value).wrap().selectable(true));
                        ui.end_row();
                    }
                });
            egui::CollapsingHeader::new("UTF-16 code units of the path").show(ui, |ui| {
                ui.add(
                    egui::Label::new(utf16_hex(&row.raw.path))
                        .wrap()
                        .selectable(true),
                );
            });
        });
}

#[cfg(test)]
mod tests {
    use peekbuddy_core::event::{Kind, Source};
    use peekbuddy_core::query::Row;

    use super::{fields, utf16_hex};
    use crate::rows::GridRow;

    fn row(source: Source, raw_mask: u32) -> GridRow {
        GridRow::new(
            Row {
                id: 42,
                ts: 134_374_896_000_000_000,
                kind: Kind::Modified,
                source,
                raw_mask,
                is_dir: false,
                size: Some(2048),
                attrs: Some(0x20 | 0x1000),
                reparse_tag: 0x9000_001A,
                file_id: 0xAB,
                name: Vec::new(),
                path: "C:\\w\\a.txt".encode_utf16().collect(),
                old_path: Vec::new(),
                root: "C:\\w".encode_utf16().collect(),
                flags: 1,
            },
            &|_| 0,
        )
    }

    fn get<'a>(fields: &'a [(&'static str, String)], label: &str) -> &'a str {
        fields
            .iter()
            .find(|(name, _)| *name == label)
            .map_or("<missing>", |(_, value)| value.as_str())
    }

    #[test]
    fn the_old_path_is_shown_only_for_a_row_that_has_one() {
        let plain = fields(&row(Source::Rdcw, 1));
        assert!(
            plain.iter().all(|(name, _)| *name != "Old path"),
            "a row without an old path has no such field"
        );
        let mut renamed = row(Source::Rdcw, 4);
        renamed.raw.old_path = "C:\\w\\before.txt".encode_utf16().collect();
        let renamed = fields(&GridRow::new(renamed.raw, &|_| 0));
        assert_eq!(get(&renamed, "Old path"), "C:\\w\\before.txt");
    }

    #[test]
    fn every_stored_field_is_shown_with_the_raw_values_in_words() {
        let fields = fields(&row(Source::Usn, 0x100 | 0x8000_0000));
        assert_eq!(get(&fields, "Id"), "42");
        assert_eq!(get(&fields, "Time (FILETIME)"), "134374896000000000");
        assert_eq!(get(&fields, "Watch"), "C:\\w");
        assert_eq!(get(&fields, "Size"), "2048 bytes (2.0 KB)");
        assert_eq!(
            get(&fields, "Attributes"),
            "0x00001020  OFFLINE | ARCHIVE".replace("OFFLINE | ARCHIVE", "ARCHIVE | OFFLINE")
        );
        assert!(get(&fields, "Reparse tag").starts_with("IO_REPARSE_TAG_CLOUD"));
        assert_eq!(get(&fields, "File id"), "000000000000000000000000000000ab");
        assert_eq!(get(&fields, "Raw value"), "0x80000100  FILE_CREATE | CLOSE");
        assert_eq!(get(&fields, "Flags"), "cloud sync churn");
    }

    #[test]
    fn rdcw_rows_show_the_action_and_other_sources_the_number() {
        assert_eq!(
            get(&fields(&row(Source::Rdcw, 3)), "Raw value"),
            "0x00000003  FILE_ACTION_MODIFIED"
        );
        assert_eq!(
            get(&fields(&row(Source::System, 5)), "Raw value"),
            "0x00000005"
        );
    }

    #[test]
    fn unknown_values_are_said_not_hidden() {
        let mut quiet = row(Source::Rdcw, 3);
        quiet.raw.size = None;
        quiet.raw.attrs = None;
        quiet.raw.reparse_tag = 0;
        quiet.raw.file_id = 0;
        quiet.raw.root = Vec::new();
        let fields = fields(&quiet);
        assert_eq!(get(&fields, "Size"), "not reported");
        assert_eq!(get(&fields, "Attributes"), "not reported");
        assert_eq!(get(&fields, "Reparse tag"), "<missing>");
        assert_eq!(get(&fields, "File id"), "<missing>");
        assert_eq!(get(&fields, "Watch"), "<missing>");
    }

    #[test]
    fn an_escaped_name_carries_a_warning_and_its_exact_units_are_available() {
        let mut spoof = row(Source::Rdcw, 1);
        spoof.raw.path = "C:\\w\\a\u{202E}b".encode_utf16().collect();
        let spoof = GridRow::new(spoof.raw, &|_| 0);
        let fields = fields(&spoof);
        assert!(get(&fields, "Name").contains("cannot be drawn safely"));
        assert_eq!(utf16_hex(&[0x43, 0x3A, 0x202E]), "0043 003A 202E");
        assert_eq!(utf16_hex(&[]), "");
    }
}
