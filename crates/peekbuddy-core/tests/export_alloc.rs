//! Exporting a row allocates the same few times however many rows follow
//! (#215, #23, F10): the writer keeps one line buffer and one scratch string.
//! `harness = false`: libtest itself allocates.

use std::alloc::System;
use std::io::Write as _;
use std::process::ExitCode;

use peekbuddy_alloc::{CountingAlloc, answer_nextest_list, measure};
use peekbuddy_core::event::{Kind, Source};
use peekbuddy_core::export::{Format, Options, RowWriter};
use peekbuddy_core::query::Row;

#[global_allocator]
static ALLOC: CountingAlloc<System> = CountingAlloc::new(System);

/// The budget `alloc.export_per_row` in `perf/budgets.toml`.
const BUDGET: u64 = 20;

fn rows() -> Vec<Row> {
    (0..64)
        .map(|i: i64| Row {
            id: i + 1,
            ts: 134_374_896_000_000_000 + u64::try_from(i).unwrap_or(0) * 10_000,
            kind: if i % 7 == 0 {
                Kind::Deleted
            } else {
                Kind::Modified
            },
            source: Source::Rdcw,
            raw_mask: 3,
            is_dir: i % 5 == 0,
            size: Some(1_000 + u64::try_from(i).unwrap_or(0)),
            attrs: Some(0x20),
            reparse_tag: 0,
            file_id: u128::try_from(i).unwrap_or(0),
            name: Vec::new(),
            // A comma, a quote and a non-ASCII letter, so the quoting runs.
            path: format!("C:\\w\\dir{i}\\\"report\", \u{e9}.txt")
                .encode_utf16()
                .collect(),
            old_path: Vec::new(),
            root: "C:\\w".encode_utf16().collect(),
            flags: 0,
        })
        .collect()
}

fn per_row(format: Format, rows: &[Row], count: u64) -> Option<u64> {
    // The output has room for every line, so the sink never grows.
    let sink = Vec::with_capacity(usize::try_from(count).ok()?.saturating_mul(1024) + 4096);
    let mut writer = RowWriter::new(sink, format, Options::default()).ok()?;
    let offset = |_ts: u64| 7_200;
    let mut failed = false;
    let mut at = 0_usize;
    let delta = measure(64, count, || {
        let row = rows.get(at % rows.len());
        at += 1;
        if let Some(row) = row {
            failed |= writer.write_row(row, &offset).is_err();
        }
    });
    let _ = writer.finish().ok()?;
    if failed { None } else { Some(delta.allocs) }
}

fn main() -> ExitCode {
    if answer_nextest_list("export_alloc") {
        return ExitCode::SUCCESS;
    }
    let rows = rows();
    let mut ok = true;
    for (name, format) in [("csv", Format::Csv), ("jsonl", Format::JsonLines)] {
        let (Some(few), Some(many)) = (
            per_row(format, &rows, 1_000),
            per_row(format, &rows, 20_000),
        ) else {
            return ExitCode::FAILURE;
        };
        // Allocations per row for a short and a long export: the same, and
        // within the budget.
        let (few_each, many_each) = (few / 1_000, many / 20_000);
        let _ = writeln!(
            std::io::stderr(),
            "export_alloc: {name}: {few} allocations in 1,000 rows, {many} in 20,000 \
             ({few_each} and {many_each} per row, budget {BUDGET})"
        );
        ok &= few_each <= BUDGET && many_each <= BUDGET;
        // Constant per row: 20x the rows may not cost 20x + more.
        ok &= many <= few.saturating_mul(20).saturating_add(64);
    }
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
