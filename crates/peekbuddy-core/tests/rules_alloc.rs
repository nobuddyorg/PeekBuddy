//! `Evaluator::on_event` allocates nothing once compiled (#81, F10), with 50
//! rules, globs, kinds and watch lists, firings and cooldowns included.
//! `harness = false`: libtest itself allocates.

use std::alloc::System;
use std::hint::black_box;
use std::process::ExitCode;

use peekbuddy_alloc::{CountingAlloc, answer_nextest_list, measure};
use peekbuddy_core::event::{Event, Flags, Kind, NameKind, NameRef, RootIdx, Source, VolIdx};
use peekbuddy_core::name::WideStr;
use peekbuddy_core::rules::Rule;
use peekbuddy_core::rules::eval::Evaluator;

#[global_allocator]
static ALLOC: CountingAlloc<System> = CountingAlloc::new(System);

fn event(ts: u64, kind: Kind, root: u16) -> Event {
    Event {
        ts,
        qpc: ts,
        src_ts: 0,
        source: Source::Rdcw,
        kind,
        raw_mask: 3,
        source_info: 0,
        is_dir: false,
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

fn rules() -> Vec<Rule> {
    (0..50)
        .map(|i| Rule {
            name: format!("rule {i}"),
            watches: if i % 3 == 0 {
                vec![String::from("a")]
            } else {
                Vec::new()
            },
            glob: match i % 4 {
                0 => String::from("*.log"),
                1 => format!("dir{}/**/*.txt", i % 5),
                2 => String::new(),
                _ => String::from("**/cache/*"),
            },
            kinds: if i % 2 == 0 {
                vec![String::from("created"), String::from("modified")]
            } else {
                Vec::new()
            },
            threshold: 1 + (i % 7),
            window_s: 5,
            cooldown_s: 2,
            ..Rule::default()
        })
        .collect()
}

fn main() -> ExitCode {
    if answer_nextest_list("rules_alloc") {
        return ExitCode::SUCCESS;
    }
    let ids = vec![String::from("a"), String::from("b")];
    let Ok(mut evaluator) = Evaluator::compile(&rules(), &ids) else {
        return ExitCode::FAILURE;
    };
    let names: Vec<Vec<u16>> = [
        "src\\main.rs",
        "x.log",
        "dir1\\sub\\n.txt",
        "a\\cache\\blob",
        "dir3\\q\\r.txt",
        "notes.md",
    ]
    .iter()
    .map(|n| n.encode_utf16().collect())
    .collect();
    let kinds = [Kind::Created, Kind::Modified, Kind::Deleted];
    let mut fired = 0_u64;
    let mut ts = 0_u64;
    let delta = measure(100, 20_000, || {
        for (i, name) in names.iter().enumerate() {
            // 40 ms apart: bursts that fire, then cooldowns that suppress.
            ts += 400_000;
            let kind = kinds.get(i % kinds.len()).copied().unwrap_or(Kind::Created);
            let ev = event(ts, kind, u16::try_from(i % 2).unwrap_or(0));
            evaluator.on_event(black_box(&ev), WideStr::new(black_box(name)), |_| {
                fired += 1;
            });
        }
    });
    black_box(fired);
    // Rules that never fired measured nothing.
    if fired == 0 {
        return ExitCode::FAILURE;
    }
    if delta.allocs == 0 && delta.bytes == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
