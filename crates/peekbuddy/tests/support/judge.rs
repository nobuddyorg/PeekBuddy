//! Judging a run with the churn comparator.
#![cfg(windows)]

use std::path::PathBuf;

use peekbuddy_churn::compare::{Gap, Row as ChurnRow, compare};
use peekbuddy_churn::ops::RawKind;
use peekbuddy_core::event::{Kind, Source};

use crate::engine::all_rows;

pub fn raw_kind(action: u32) -> Option<RawKind> {
    Some(match action {
        1 => RawKind::Added,
        2 => RawKind::Removed,
        3 => RawKind::Modified,
        4 => RawKind::RenamedOld,
        5 => RawKind::RenamedNew,
        _ => return None,
    })
}

/// `path` relative to `root` (both without the `\\?\` prefix), as UTF-16.
pub fn relative(path: &[u16], root: &[u16]) -> Option<Vec<u16>> {
    let head = path.get(..root.len())?;
    let same = String::from_utf16_lossy(head).eq_ignore_ascii_case(&String::from_utf16_lossy(root));
    let rest = path.get(root.len()..)?;
    if same && rest.first() == Some(&0x5C) {
        rest.get(1..).map(<[u16]>::to_vec)
    } else {
        None
    }
}

pub struct Run {
    pub data: PathBuf,
    pub root: PathBuf,
}

/// What the comparator saw.
pub struct Verdict {
    pub report: peekbuddy_churn::compare::Report,
    pub gaps: usize,
    pub unfinished_gaps: usize,
    pub summary: String,
}

pub fn judge(run: &Run, outcome: &peekbuddy_churn::exec::Outcome) -> Verdict {
    let root_units: Vec<u16> = run
        .root
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .encode_utf16()
        .collect();
    let rows = all_rows(&run.data);

    let raw: Vec<ChurnRow> = rows
        .iter()
        .filter(|row| row.source == Source::Rdcw)
        .filter_map(|row| {
            Some(ChurnRow {
                path: relative(&row.path, &root_units)?,
                kind: raw_kind(row.raw_mask)?,
                time: row.ts,
            })
        })
        .collect();

    // A gap covers the whole root until the reconcile that follows it ends.
    let gap_rows = rows.iter().filter(|row| row.kind.is_gap()).count();
    let gaps: Vec<Gap> = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| row.kind.is_gap())
        .filter_map(|(at, _)| {
            rows.iter()
                .skip(at)
                .find(|row| row.kind == Kind::ReconcileFinished)
                .map(|finish| Gap {
                    subtree: Vec::new(),
                    reconciled: finish.ts,
                })
        })
        .collect();

    let report = compare(&outcome.records, &raw, &gaps);
    let mut kinds: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for row in rows.iter().filter(|row| row.source != Source::Rdcw) {
        *kinds.entry(row.kind.name()).or_default() += 1;
    }
    let summary = format!(
        "ops {} matched {} covered by a gap {} undetected {} gap rows {gap_rows} rows {}; other rows {kinds:?}",
        outcome.records.len(),
        report.matched,
        report.covered_by_gap,
        report.undetected.len(),
        rows.len()
    );
    let detail: Vec<String> = report
        .undetected
        .iter()
        .take(12)
        .filter_map(|seq| outcome.records.iter().find(|r| r.seq == *seq))
        .map(|r| {
            let wanted: Vec<String> = r
                .expected_records()
                .unwrap_or_default()
                .iter()
                .map(|e| {
                    let seen = raw
                        .iter()
                        .any(|row| row.path == e.path && row.kind == e.kind);
                    format!(
                        "{:?} {} seen={seen}",
                        e.kind,
                        String::from_utf16_lossy(&e.path)
                    )
                })
                .collect();
            format!("#{} {:?} [{}]", r.seq, r.op, wanted.join("; "))
        })
        .collect();
    Verdict {
        report,
        gaps: gap_rows,
        unfinished_gaps: gap_rows - gaps.len(),
        summary: format!("{summary}\n{}", detail.join("\n")),
    }
}
