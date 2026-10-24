//! `FilterSet::eval` allocates nothing once its `Scratch` exists (#40, F10).
//! `harness = false`: libtest itself allocates.

use std::alloc::System;
use std::hint::black_box;
use std::process::ExitCode;

use peekbuddy_alloc::{CountingAlloc, answer_nextest_list, measure};
use peekbuddy_core::event::{Event, Flags, Kind, NameKind, NameRef, RootIdx, Source, VolIdx};
use peekbuddy_core::filter::{CompileOptions, FilterSet, Pattern, Rule, Scratch, presets};
use peekbuddy_core::name::{CaseMode, WideStr};

#[global_allocator]
static ALLOC: CountingAlloc<System> = CountingAlloc::new(System);

fn event(root: u16, is_dir: bool) -> Event {
    Event {
        ts: 1,
        qpc: 1,
        src_ts: 0,
        source: Source::Rdcw,
        kind: Kind::Modified,
        raw_mask: 3,
        source_info: 0,
        is_dir,
        size: 0,
        attrs: 0,
        reparse_tag: 0,
        file_id: 1,
        parent_file_id: 1,
        usn: 0,
        vol: VolIdx(0),
        root: RootIdx(root),
        name: NameRef::EMPTY,
        name_kind: NameKind::Long,
        flags: Flags::NONE,
    }
}

fn main() -> ExitCode {
    if answer_nextest_list("filter_alloc") {
        return ExitCode::SUCCESS;
    }
    let mut rules: Vec<Rule> = presets::all()
        .unwrap_or_default()
        .iter()
        .flat_map(|p| p.rules(peekbuddy_core::filter::Scope::Global))
        .collect();
    for i in 0..20 {
        rules.push(Rule::exclude(Pattern::Glob(format!("user-{i}/**/*.log"))).in_watch(RootIdx(1)));
    }
    rules.push(Rule::exclude(Pattern::Regex(r"(?i)\.bak\d*$".into())));
    rules.push(Rule::exclude(Pattern::Ext(vec!["dmp".into()])).never_record());
    let options = CompileOptions {
        default_case: CaseMode::Insensitive,
        overrides: vec![(RootIdx(2), CaseMode::Sensitive)],
    };
    let Ok(set) = FilterSet::compile_with(&rules, &options) else {
        return ExitCode::FAILURE;
    };
    let mut scratch = Scratch::new();
    let names: Vec<Vec<u16>> = [
        "src\\main.rs",
        "node_modules\\x\\index.js",
        "Docs\\~$draft.docx",
        "user-3\\a\\b\\c.log",
        "crash.dmp",
        "\u{C4}rger\\caf\u{E9}.txt",
        "backup.bak12",
    ]
    .iter()
    .map(|n| n.encode_utf16().collect())
    .collect();
    let resolved: Vec<u16> = "Long Name.txt".encode_utf16().collect();
    let mut hidden = 0_u64;
    let delta = measure(100, 20_000, || {
        for (i, name) in names.iter().enumerate() {
            let root = u16::try_from(i % 3).unwrap_or(0);
            let ev = event(root, i % 2 == 0);
            let v = set.eval_with_resolved(
                black_box(&ev),
                WideStr::new(black_box(name)),
                Some(WideStr::new(&resolved)),
                &mut scratch,
            );
            hidden += u64::from(!v.visible);
        }
    });
    black_box(hidden);
    // A filter that never matched anything measured nothing.
    if hidden == 0 {
        return ExitCode::FAILURE;
    }
    if delta.allocs == 0 && delta.bytes == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
