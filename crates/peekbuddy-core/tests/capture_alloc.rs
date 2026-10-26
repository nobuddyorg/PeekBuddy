//! The capture path (parse, map, filter) allocates nothing per event once
//! the matcher scratch exists (#23, F10). `harness = false`: libtest itself
//! allocates.

use std::alloc::System;
use std::hint::black_box;
use std::io::Write as _;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use peekbuddy_alloc::{CountingAlloc, answer_nextest_list, measure};
use peekbuddy_core::capture::{Decoder, EventSink, Stamp};
use peekbuddy_core::event::{Event, Flags, NameKind, NameRef, RootIdx, Source, VolIdx};
use peekbuddy_core::filter::{CompileOptions, FilterSet, Pattern, Rule, Scope, Scratch, presets};
use peekbuddy_core::name::{CaseMode, WideStr};
use peekbuddy_core::notify::{Class, layout, parse};
use peekbuddy_core::pause::{PausableSink, Pause};

#[global_allocator]
static ALLOC: CountingAlloc<System> = CountingAlloc::new(System);

const BUFFER: usize = 64 * 1024;

/// What the engine's sink does: evaluate the filters, keep what is recorded.
struct FilteredRing<'a> {
    set: &'a FilterSet,
    scratch: Scratch,
    stored: Vec<Event>,
    offered: usize,
}

impl EventSink for FilteredRing<'_> {
    fn push(&mut self, event: &Event, name: &[u16]) -> bool {
        self.offered += 1;
        let verdict = self.set.eval(event, WideStr::new(name), &mut self.scratch);
        if verdict.record {
            self.stored.push(*event);
        }
        true
    }
}

/// A 64 KiB buffer of extended records with names from a fixed set (some
/// match the presets, some do not); returns it and the record count.
fn build() -> (Vec<u8>, usize) {
    let names: Vec<Vec<u16>> = [
        "src\\main.rs",
        "node_modules\\x\\index.js",
        "Docs\\~$draft.docx",
        "crash.dmp",
        "backup.bak12",
        "a\\b\\c\\d\\e.txt",
    ]
    .iter()
    .map(|n| n.encode_utf16().collect())
    .collect();
    let header = Class::Extended.header_len();
    let mut buf = vec![0_u8; BUFFER];
    let (mut at, mut count, mut last) = (0_usize, 0_usize, 0_usize);
    loop {
        let name = names.get(count % names.len()).cloned().unwrap_or_default();
        let size = (header + name.len() * 2).next_multiple_of(4);
        if at + size > BUFFER {
            break;
        }
        let mut put = |offset: usize, bytes: &[u8]| {
            if let Some(dst) = buf.get_mut(at + offset..at + offset + bytes.len()) {
                dst.copy_from_slice(bytes);
            }
        };
        put(
            layout::NEXT_ENTRY_OFFSET,
            &u32::try_from(size).unwrap_or(0).to_le_bytes(),
        );
        put(layout::ACTION, &3_u32.to_le_bytes());
        put(
            layout::EXTENDED_NAME_LENGTH,
            &u32::try_from(name.len() * 2).unwrap_or(0).to_le_bytes(),
        );
        put(layout::FILE_ATTRIBUTES, &0x20_u32.to_le_bytes());
        for (i, unit) in name.iter().enumerate() {
            put(header + i * 2, &unit.to_le_bytes());
        }
        last = at;
        at += size;
        count += 1;
    }
    if let Some(dst) = buf.get_mut(last..last + 4) {
        dst.fill(0);
    }
    (buf, count)
}

fn main() -> ExitCode {
    if answer_nextest_list("capture_alloc") {
        return ExitCode::SUCCESS;
    }
    let mut rules: Vec<Rule> = presets::all()
        .unwrap_or_default()
        .iter()
        .filter(|p| {
            matches!(
                p.name.as_str(),
                "temp-lock-swap" | "build-dirs" | "cloud-churn"
            )
        })
        .flat_map(|p| p.rules(Scope::Global))
        .collect();
    rules.push(Rule::exclude(Pattern::Regex(r"(?i)\.bak\d*$".into())));
    rules.push(Rule::exclude(Pattern::Ext(vec!["dmp".into()])).never_record());
    let options = CompileOptions {
        default_case: CaseMode::Insensitive,
        overrides: Vec::new(),
    };
    let Ok(set) = FilterSet::compile_with(&rules, &options) else {
        return ExitCode::FAILURE;
    };
    let (buf, expected) = build();
    let mut scratch = Scratch::new();
    let mut units = [0_u16; 128];
    let (mut seen, mut hidden) = (0_usize, 0_usize);
    let delta = measure(20, 500, || {
        seen = 0;
        hidden = 0;
        for record in parse(black_box(&buf), buf.len(), Class::Extended) {
            let Ok(record) = record else { return };
            let Some(extended) = record.extended else {
                return;
            };
            let len = record.name.copy_into(&mut units).unwrap_or(0);
            let event = Event {
                ts: 1,
                qpc: 1,
                src_ts: 0,
                source: Source::Rdcw,
                kind: record.action.kind(),
                raw_mask: record.action.raw(),
                source_info: 0,
                is_dir: extended.is_dir(),
                size: u64::try_from(extended.file_size).unwrap_or(0),
                attrs: extended.attributes,
                reparse_tag: extended.reparse_tag,
                file_id: u128::try_from(extended.file_id).unwrap_or(0),
                parent_file_id: u128::try_from(extended.parent_file_id).unwrap_or(0),
                usn: 0,
                vol: VolIdx(0),
                root: RootIdx(0),
                name: NameRef::EMPTY,
                name_kind: NameKind::Unknown,
                flags: Flags::NONE,
            };
            let name = units.get(..len).unwrap_or_default();
            let verdict = set.eval(black_box(&event), WideStr::new(name), &mut scratch);
            seen += 1;
            hidden += usize::from(!verdict.visible);
        }
    });
    // A run that stopped early or hid nothing measured nothing.
    if seen != expected || hidden == 0 || delta.allocs != 0 || delta.bytes != 0 {
        return ExitCode::FAILURE;
    }
    // The same buffer through the RDCW decoder (#45): parse, map, drop the
    // data directory's churn, filter, enqueue.
    let mut sink = FilteredRing {
        set: &set,
        scratch: Scratch::new(),
        stored: Vec::with_capacity(expected),
        offered: 0,
    };
    let mut decoder = Decoder::new(
        RootIdx(0),
        VolIdx(0),
        Class::Extended,
        &[vec![u16::from(b'z')]],
    );
    let delta = measure(20, 500, || {
        sink.stored.clear();
        sink.offered = 0;
        decoder.decode(
            black_box(&buf),
            buf.len(),
            Stamp { ts: 1, qpc: 1 },
            &mut sink,
        );
    });
    if sink.offered != expected || sink.stored.is_empty() || delta.allocs != 0 || delta.bytes != 0 {
        return ExitCode::FAILURE;
    }
    paused_run(&buf, expected, sink)
}

/// The same buffer while the watch is paused (#63): every event is counted
/// and discarded after the parse, still without an allocation. With
/// `PEEKBUDDY_PRINT_TIMES` set it also prints the cost of a live and a paused
/// buffer, the numbers behind the pause decision (ADR 0016).
fn paused_run(buf: &[u8], expected: usize, live: FilteredRing<'_>) -> ExitCode {
    let pause = Arc::new(Pause::new());
    let stamp = Stamp { ts: 1, qpc: 1 };
    let mut sink = PausableSink::new(live, Arc::clone(&pause));
    let mut decoder = Decoder::new(RootIdx(0), VolIdx(0), Class::Extended, &[]);
    pause.set_watch(RootIdx(0), true);
    let delta = measure(20, 500, || {
        decoder.decode(black_box(buf), buf.len(), stamp, &mut sink);
    });
    let counted = pause.missed(RootIdx(0));
    // 520 buffers were decoded (20 warm-up, 500 measured).
    if counted != u64::try_from(expected * 520).unwrap_or(0)
        || delta.allocs != 0
        || delta.bytes != 0
    {
        return ExitCode::FAILURE;
    }
    if std::env::var_os("PEEKBUDDY_PRINT_TIMES").is_some() {
        let live = sink.into_inner();
        let mut live_sink = live;
        let mut live_decoder = Decoder::new(RootIdx(0), VolIdx(0), Class::Extended, &[]);
        let time = |run: &mut dyn FnMut()| {
            let start = Instant::now();
            for _ in 0..2_000 {
                run();
            }
            start.elapsed().as_nanos() / 2_000
        };
        let live_ns = time(&mut || {
            live_sink.stored.clear();
            live_decoder.decode(black_box(buf), buf.len(), stamp, &mut live_sink);
        });
        let mut paused_sink = PausableSink::new(live_sink, Arc::clone(&pause));
        let paused_ns = time(&mut || {
            live_decoder.decode(black_box(buf), buf.len(), stamp, &mut paused_sink);
        });
        let _ = writeln!(
            std::io::stderr(),
            "records per buffer {expected}: live {live_ns} ns, paused {paused_ns} ns ({}%)",
            paused_ns * 100 / live_ns.max(1)
        );
    }
    ExitCode::SUCCESS
}
