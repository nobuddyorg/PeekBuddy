//! Compares the ledger with what PeekBuddy logged.
//!
//! An expected record is **matched** by a logged row with the same path and
//! kind whose time is at or after the operation's start. One row may match
//! several operations: the file system can merge repeated identical
//! notifications, so same-kind repeats count as "at least one". An operation
//! with an unmatched record is **covered** when a gap (overflow, F2) holds
//! every unmatched path in its subtree and the gap's reconciliation finished
//! after the operation ended. Anything else is an **undetected loss**: the
//! number that must be 0.
//!
//! Inputs are OS-independent (the export of #56 or segment DBs are mapped
//! to [`Row`] and [`Gap`] by the caller), so this runs on Linux and under
//! Miri.

use std::collections::{BTreeMap, BTreeSet};

use crate::ledger::Record;
use crate::ops::{RawKind, WidePath};
use crate::plan::is_within;

/// A raw row PeekBuddy logged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// Path relative to the churn root.
    pub path: WidePath,
    /// Record kind.
    pub kind: RawKind,
    /// When the watcher received it, UTC FILETIME ticks.
    pub time: u64,
}

/// A gap event: notifications under `subtree` were lost and a
/// reconciliation scan finished at `reconciled`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gap {
    /// Subtree relative to the churn root; empty for the whole root.
    pub subtree: WidePath,
    /// End of the reconciliation scan, UTC FILETIME ticks.
    pub reconciled: u64,
}

/// The comparison result.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// Operations with every expected record matched (failed operations
    /// expect nothing and count here).
    pub matched: u64,
    /// Operations with an unmatched record covered by a gap.
    pub covered_by_gap: u64,
    /// Sequence numbers of operations with an undetected loss.
    pub undetected: Vec<u64>,
    /// Rows no operation expects (reported, not failed).
    pub unexpected_rows: u64,
    /// Records the ledger could not decode (a broken ledger fails the
    /// comparison like a loss).
    pub malformed: Vec<u64>,
}

impl Report {
    /// `true` when nothing was lost undetected and the ledger was readable.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.undetected.is_empty() && self.malformed.is_empty()
    }
}

/// Compares `records` with the logged `rows` and `gaps`.
#[must_use]
pub fn compare(records: &[Record], rows: &[Row], gaps: &[Gap]) -> Report {
    // Latest row time per (path, kind): a row at or after `start` exists
    // exactly when the latest one is.
    let mut latest: BTreeMap<(&[u16], RawKind), u64> = BTreeMap::new();
    for row in rows {
        let time = latest
            .entry((row.path.as_slice(), row.kind))
            .or_insert(row.time);
        *time = (*time).max(row.time);
    }
    let seen_after = |path: &[u16], kind: RawKind, start: u64| {
        latest.get(&(path, kind)).is_some_and(|last| *last >= start)
    };

    let mut report = Report::default();
    let mut wanted: BTreeSet<(WidePath, RawKind)> = BTreeSet::new();
    for record in records {
        let Some(expected) = record.expected_records() else {
            report.malformed.push(record.seq);
            continue;
        };
        let missing: Vec<&WidePath> = expected
            .iter()
            .filter(|e| !seen_after(&e.path, e.kind, record.t_start))
            .map(|e| &e.path)
            .collect();
        if missing.is_empty() {
            report.matched += 1;
        } else if gaps.iter().any(|gap| {
            gap.reconciled > record.t_end && missing.iter().all(|p| is_within(p, &gap.subtree))
        }) {
            report.covered_by_gap += 1;
        } else {
            report.undetected.push(record.seq);
        }
        wanted.extend(expected.into_iter().map(|e| (e.path, e.kind)));
    }
    report.unexpected_rows = rows
        .iter()
        .filter(|row| !wanted.contains(&(row.path.clone(), row.kind)))
        .count()
        .try_into()
        .unwrap_or(u64::MAX);
    report
}

#[cfg(test)]
#[expect(
    clippy::indexing_slicing,
    reason = "tests index generated data of known size; a panic is a test failure"
)]
mod tests {
    use super::{Gap, Report, Row, compare};
    use crate::ledger::{ExpectedText, Record, encode_path};
    use crate::ops::{Op, OpKind, RawKind, WidePath};
    use crate::plan::{Planner, Shape, is_within};
    use crate::profile::profile;
    use proptest::prelude::{any, prop, prop_assert, prop_assert_eq, proptest};

    /// Planned operations per synthetic ledger (after the setup); small
    /// under Miri, which runs these tests too (#25).
    const OPS: usize = if cfg!(miri) { 3 } else { 120 };

    fn w(text: &str) -> WidePath {
        text.encode_utf16().collect()
    }

    /// A ledger of `ops` planned operations, op `i` running from `10 i` to
    /// `10 i + 5`.
    fn ledger(seed: u64, ops: usize) -> Vec<Record> {
        let mut planner = Planner::new(seed, 0, profile("edge-mix").unwrap());
        let mut plan = planner.setup(Shape::default());
        plan.extend((0..ops).map(|_| planner.next_op()));
        plan.iter()
            .zip(0u64..)
            .map(|(op, seq)| Record::new(seq, 0, op, seq * 10, seq * 10 + 5, 0))
            .collect()
    }

    /// Every expected record logged once, just after its operation ended.
    fn perfect_rows(records: &[Record]) -> Vec<Row> {
        records
            .iter()
            .flat_map(|record| {
                record
                    .expected_records()
                    .unwrap()
                    .into_iter()
                    .map(move |e| Row {
                        path: e.path,
                        kind: e.kind,
                        time: record.t_end + 1,
                    })
            })
            .collect()
    }

    /// Brute-force oracle: sequence numbers whose records are not all
    /// matched and no gap covers them.
    fn oracle(records: &[Record], rows: &[Row], gaps: &[Gap]) -> Vec<u64> {
        records
            .iter()
            .filter(|record| {
                let missing: Vec<WidePath> = record
                    .expected_records()
                    .unwrap()
                    .into_iter()
                    .filter(|e| {
                        !rows.iter().any(|row| {
                            row.path == e.path && row.kind == e.kind && row.time >= record.t_start
                        })
                    })
                    .map(|e| e.path)
                    .collect();
                !missing.is_empty()
                    && !gaps.iter().any(|gap| {
                        gap.reconciled > record.t_end
                            && missing.iter().all(|p| is_within(p, &gap.subtree))
                    })
            })
            .map(|record| record.seq)
            .collect()
    }

    proptest! {
        /// Against a brute-force oracle, for random drops, duplicates,
        /// unrelated rows and gaps.
        #[test]
        fn agrees_with_the_oracle(
            seed in 0u64..1000,
            drops in prop::collection::vec(any::<prop::sample::Index>(), 0..6),
            dups in prop::collection::vec(any::<prop::sample::Index>(), 0..6),
            gap_at in prop::option::of((any::<prop::sample::Index>(), 0u64..3_000)),
            noise in 0usize..5,
        ) {
            let records = ledger(seed, OPS);
            let mut rows = perfect_rows(&records);
            for drop in &drops {
                if !rows.is_empty() {
                    rows.remove(drop.index(rows.len()));
                }
            }
            for dup in &dups {
                if !rows.is_empty() {
                    let row = rows[dup.index(rows.len())].clone();
                    rows.push(row);
                }
            }
            for i in 0..noise {
                rows.push(Row { path: w(&format!("elsewhere\\n{i}")), kind: RawKind::Modified, time: 1 });
            }
            let gaps: Vec<Gap> = gap_at
                .map(|(at, reconciled)| {
                    let record = &records[at.index(records.len())];
                    let path = record.expected_records().unwrap().first().map(|e| e.path.clone()).unwrap_or_default();
                    let parent = path.iter().rposition(|&u| u == u16::from(b'\\')).map_or(Vec::new(), |at| path[..at].to_vec());
                    Gap { subtree: parent, reconciled }
                })
                .into_iter()
                .collect();
            let report = compare(&records, &rows, &gaps);
            prop_assert_eq!(&report.undetected, &oracle(&records, &rows, &gaps));
            prop_assert_eq!(report.unexpected_rows, u64::try_from(noise).unwrap());
            let total = report.matched + report.covered_by_gap + u64::try_from(report.undetected.len()).unwrap();
            prop_assert_eq!(total, u64::try_from(records.len()).unwrap());
        }

        /// Removing a row that is the only witness of an expectation, with no
        /// gap, is always reported (no false negatives).
        #[test]
        fn removing_a_sole_witness_is_a_loss(seed in 0u64..1000, pick in any::<prop::sample::Index>()) {
            let records = ledger(seed, OPS);
            let rows = perfect_rows(&records);
            let at = pick.index(rows.len());
            let removed = rows[at].clone();
            let mut rest = rows.clone();
            rest.remove(at);
            let report = compare(&records, &rest, &[]);
            let still_seen = |record: &Record| rest.iter().any(|row| row.path == removed.path && row.kind == removed.kind && row.time >= record.t_start);
            let owners: Vec<u64> = records
                .iter()
                .filter(|record| record.expected_records().unwrap().iter().any(|e| e.path == removed.path && e.kind == removed.kind))
                .filter(|record| !still_seen(record))
                .map(|record| record.seq)
                .collect();
            prop_assert!(!owners.is_empty() || rest.iter().any(|row| row.path == removed.path && row.kind == removed.kind));
            prop_assert_eq!(&report.undetected, &owners);
            // A whole-root gap reconciled after the run covers it.
            let gap = Gap { subtree: Vec::new(), reconciled: u64::MAX };
            let covered = compare(&records, &rest, &[gap]);
            prop_assert!(covered.passed());
            prop_assert_eq!(covered.covered_by_gap, u64::try_from(owners.len()).unwrap());
        }
    }

    fn one(kind: OpKind, paths: &[&str], t: u64) -> Record {
        let op = Op {
            kind,
            paths: paths.iter().map(|p| w(p)).collect(),
            removed_below: Vec::new(),
        };
        Record::new(t, 0, &op, t, t + 5, 0)
    }

    fn row(path: &str, kind: RawKind, time: u64) -> Row {
        Row {
            path: w(path),
            kind,
            time,
        }
    }

    #[test]
    fn a_perfect_log_passes() {
        let records = ledger(7, OPS);
        let report = compare(&records, &perfect_rows(&records), &[]);
        assert!(report.passed());
        assert_eq!(report.matched, u64::try_from(records.len()).unwrap());
        assert_eq!(
            report,
            Report {
                matched: report.matched,
                ..Report::default()
            }
        );
    }

    #[test]
    fn a_row_before_the_operation_does_not_match() {
        let records = [one(OpKind::Write, &["t0\\f"], 100)];
        let early = compare(&records, &[row("t0\\f", RawKind::Modified, 99)], &[]);
        assert_eq!(early.undetected, [100]);
        let on_time = compare(&records, &[row("t0\\f", RawKind::Modified, 100)], &[]);
        assert!(on_time.passed());
    }

    #[test]
    fn the_wrong_kind_or_path_does_not_match() {
        let records = [one(OpKind::Write, &["t0\\f"], 0)];
        assert_eq!(
            compare(&records, &[row("t0\\f", RawKind::Added, 9)], &[]).undetected,
            [0]
        );
        assert_eq!(
            compare(&records, &[row("t0\\g", RawKind::Modified, 9)], &[]).undetected,
            [0]
        );
        assert_eq!(
            compare(&records, &[row("T0\\F", RawKind::Modified, 9)], &[]).undetected,
            [0]
        );
    }

    #[test]
    fn a_rename_needs_both_names() {
        let records = [one(OpKind::Rename, &["t0\\a", "t0\\b"], 0)];
        let old_only = [row("t0\\a", RawKind::RenamedOld, 9)];
        assert_eq!(compare(&records, &old_only, &[]).undetected, [0]);
        let both = [
            row("t0\\a", RawKind::RenamedOld, 9),
            row("t0\\b", RawKind::RenamedNew, 9),
        ];
        assert!(compare(&records, &both, &[]).passed());
    }

    #[test]
    fn gaps_cover_only_their_subtree_and_only_after_the_operation() {
        let records = [one(OpKind::Write, &["t0\\d\\f"], 100)];
        let cover = |subtree: &str, reconciled| {
            compare(
                &records,
                &[],
                &[Gap {
                    subtree: w(subtree),
                    reconciled,
                }],
            )
        };
        assert_eq!(cover("t0\\d", 106).covered_by_gap, 1);
        assert_eq!(cover("t0", 106).covered_by_gap, 1);
        assert_eq!(cover("", 106).covered_by_gap, 1);
        // Reconciled before the operation ended: it cannot have seen it.
        assert_eq!(cover("t0\\d", 105).undetected, [100]);
        assert_eq!(cover("t0\\e", 106).undetected, [100]);
        assert_eq!(cover("t0\\d\\f\\x", 106).undetected, [100]);
        assert_eq!(cover("t0\\dd", 106).undetected, [100]);
    }

    #[test]
    fn a_gap_must_cover_every_missing_path() {
        let records = [one(OpKind::Move, &["t0\\a\\f", "t0\\b\\f"], 0)];
        let gap = Gap {
            subtree: w("t0\\a"),
            reconciled: 99,
        };
        assert_eq!(
            compare(&records, &[], std::slice::from_ref(&gap)).undetected,
            [0]
        );
        let half = [row("t0\\b\\f", RawKind::RenamedNew, 9)];
        assert_eq!(compare(&records, &half, &[gap]).covered_by_gap, 1);
    }

    #[test]
    fn one_row_may_match_repeated_operations() {
        let records = [
            one(OpKind::Write, &["t0\\f"], 0),
            one(OpKind::Append, &["t0\\f"], 10),
        ];
        let report = compare(&records, &[row("t0\\f", RawKind::Modified, 20)], &[]);
        assert!(report.passed());
        assert_eq!(report.matched, 2);
    }

    #[test]
    fn failed_operations_and_unexpected_rows_do_not_fail() {
        let op = Op {
            kind: OpKind::Delete,
            paths: vec![w("t0\\f")],
            removed_below: Vec::new(),
        };
        let failed = Record::new(0, 0, &op, 0, 5, 2);
        let rows = [
            row("t0\\other", RawKind::Added, 3),
            row("t0\\f", RawKind::Modified, 3),
        ];
        let report = compare(&[failed], &rows, &[]);
        assert!(report.passed());
        assert_eq!((report.matched, report.unexpected_rows), (1, 2));
    }

    #[test]
    fn a_malformed_ledger_fails() {
        let mut record = one(OpKind::Write, &["t0\\f"], 0);
        record.expected = vec![ExpectedText {
            path: "utf16:zz".into(),
            kind: RawKind::Modified,
        }];
        let report = compare(&[record], &[], &[]);
        assert!(!report.passed());
        assert_eq!(report.malformed, [0]);
        assert_eq!(encode_path(&w("t0\\f")), "t0\\f");
    }
}
