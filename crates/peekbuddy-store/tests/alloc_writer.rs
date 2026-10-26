//! Heap allocations of the writer per 1,000-event batch once warm (#52, F10).
//!
//! SQLite allocates through its own allocator, not the Rust one, so this counts
//! what the Rust side of the writer does: statement lookup, parameter binding,
//! the stats aggregation, the transaction. `harness = false`: libtest itself
//! allocates.

use std::alloc::System;
use std::io::Write as _;
use std::process::ExitCode;

use peekbuddy_alloc::{CountingAlloc, answer_nextest_list, measure};
use peekbuddy_core::event::{
    Event, Flags, Kind, NameKind, NameRef, RootIdx, Source, VolIdx,
};
use peekbuddy_store::batch::Batch;
use peekbuddy_store::schema;
use peekbuddy_store::writer::{RootInfo, VolInfo, Writer};
use rusqlite::Connection;

#[global_allocator]
static ALLOC: CountingAlloc<System> = CountingAlloc::new(System);

/// The budget `alloc.writer_per_1000` in `perf/budgets.toml`.
const BUDGET: u64 = 2;
const ITERATIONS: u64 = 40;

fn event(ts: u64) -> Event {
    Event {
        ts,
        qpc: ts,
        src_ts: 0,
        source: Source::Rdcw,
        kind: Kind::Modified,
        raw_mask: 3,
        source_info: 0,
        is_dir: false,
        size: 100,
        attrs: 0x20,
        reparse_tag: 0,
        file_id: u128::from(ts % 64),
        parent_file_id: 5,
        usn: 0,
        vol: VolIdx(0),
        root: RootIdx(0),
        name: NameRef::EMPTY,
        name_kind: NameKind::Long,
        flags: Flags::NONE,
    }
}

fn main() -> ExitCode {
    if answer_nextest_list("alloc_writer") {
        return ExitCode::SUCCESS;
    }
    let Ok(mut conn) = Connection::open_in_memory() else {
        return ExitCode::FAILURE;
    };
    if schema::migrate(&mut conn).is_err() {
        return ExitCode::FAILURE;
    }
    let Ok(mut writer) = Writer::from_connection(conn) else {
        return ExitCode::FAILURE;
    };
    let registered = writer.register_root(&RootInfo {
        idx: RootIdx(0),
        vol: VolInfo {
            guid: String::from("vol"),
            serial: 1,
            letter: None,
            journal_id: None,
        },
        path: r"\\?\C:\w".encode_utf16().collect(),
        recursive: true,
    });
    if registered.is_err() {
        return ExitCode::FAILURE;
    }
    // 1,000 events in four batches over 64 names: after the first write every
    // node is interned, so a hit allocates nothing.
    let names: Vec<Vec<u16>> = (0..64)
        .map(|i| format!("dir\\file{i}.txt").encode_utf16().collect())
        .collect();
    let mut batches: Vec<Batch> = (0..4).map(|_| Batch::with_capacity(256, 16 * 1024)).collect();
    let mut ts = 0u64;
    for batch in &mut batches {
        for _ in 0..250 {
            let name = names.get(usize::try_from(ts % 64).unwrap_or(0)).cloned().unwrap_or_default();
            if batch.push(event(ts), &name).is_err() {
                return ExitCode::FAILURE;
            }
            ts += 1;
        }
    }
    let mut failed = false;
    let delta = measure(5, ITERATIONS, || {
        failed |= writer.write(&batches, &[]).is_err();
    });
    let per_batch = delta.allocs / ITERATIONS;
    let _ = writeln!(
        std::io::stderr(),
        "alloc_writer: {per_batch} allocations and {} bytes per 1,000-event batch (budget {BUDGET})",
        delta.bytes / ITERATIONS
    );
    if failed || per_batch > BUDGET {
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
