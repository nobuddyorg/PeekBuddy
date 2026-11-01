//! The USN feed allocates nothing per file record once its scratch buffers
//! have grown (#212, #23, F10). Half of the records carry their name, half
//! have it withheld and are resolved through [`Names`], like the unprivileged
//! read. `harness = false`: libtest itself allocates.

use std::alloc::System;
use std::hint::black_box;
use std::process::ExitCode;

use peekbuddy_alloc::{CountingAlloc, answer_nextest_list, measure};
use peekbuddy_core::capture::{EventSink, Stamp};
use peekbuddy_core::event::{Event, RootIdx, VolIdx};
use peekbuddy_core::usn::feed::{Feed, Names};
use peekbuddy_core::usn::scope::Scope;

#[global_allocator]
static ALLOC: CountingAlloc<System> = CountingAlloc::new(System);

const ROOT: u128 = 1;
const DIRS: u128 = 64;
const RECORDS: usize = 500;
const FILE_CREATE: u32 = 0x100;
const DATA_EXTEND: u32 = 0x2;

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

/// Version 3 record bytes.
fn record(frn: u128, parent: u128, usn: i64, reason: u32, name: &[u16]) -> Vec<u8> {
    let name_bytes: Vec<u8> = name.iter().flat_map(|unit| unit.to_le_bytes()).collect();
    let length = (76 + name_bytes.len()).next_multiple_of(8);
    let mut out = Vec::with_capacity(length);
    out.extend_from_slice(&u32::try_from(length).unwrap_or(0).to_le_bytes());
    out.extend_from_slice(&3_u16.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out.extend_from_slice(&frn.to_le_bytes());
    out.extend_from_slice(&parent.to_le_bytes());
    out.extend_from_slice(&usn.to_le_bytes());
    out.extend_from_slice(&0_i64.to_le_bytes());
    out.extend_from_slice(&reason.to_le_bytes());
    out.extend_from_slice(&0_u32.to_le_bytes());
    out.extend_from_slice(&0_u32.to_le_bytes());
    out.extend_from_slice(&0x20_u32.to_le_bytes());
    out.extend_from_slice(&u16::try_from(name_bytes.len()).unwrap_or(0).to_le_bytes());
    out.extend_from_slice(&76_u16.to_le_bytes());
    out.extend_from_slice(&name_bytes);
    out.resize(length, 0);
    out
}

/// Resolves any file to a fixed path below the root.
struct Fs {
    path: Vec<u16>,
}

impl Names for Fs {
    fn path_of(&mut self, _frn: u128, out: &mut Vec<u16>) -> bool {
        out.clear();
        out.extend_from_slice(&self.path);
        true
    }

    fn walk(&mut self, _frn: u128, _scope: &mut Scope) {}
}

#[derive(Default)]
struct Count {
    events: usize,
    unresolved: usize,
}

impl EventSink for Count {
    fn push(&mut self, event: &Event, name: &[u16]) -> bool {
        self.events += 1;
        self.unresolved += usize::from(event.flags != peekbuddy_core::event::Flags::NONE);
        black_box(name);
        true
    }
}

fn main() -> ExitCode {
    if answer_nextest_list("usn_alloc") {
        return ExitCode::SUCCESS;
    }
    // root \ d2 \ d3 ... a chain of directories, so paths are long.
    let mut scope = Scope::new(ROOT);
    for dir in 0..DIRS {
        let parent = if dir == 0 { ROOT } else { ROOT + dir };
        scope.insert(ROOT + dir + 1, parent, &units(&format!("dir{dir}")));
    }
    let mut feed = Feed::new(scope, VolIdx(0), RootIdx(0), &units(r"\proj"));

    let mut parts = Vec::new();
    for index in 0..RECORDS {
        let parent = ROOT + (index as u128 % DIRS) + 1;
        let reason = if index % 5 == 0 {
            FILE_CREATE
        } else {
            DATA_EXTEND
        };
        let name = if index % 2 == 0 {
            units(&format!("file-{index}.txt"))
        } else {
            Vec::new()
        };
        parts.push(record(
            1_000 + index as u128,
            parent,
            i64::try_from(index).unwrap_or(0),
            reason,
            &name,
        ));
    }
    let mut buf = 9_999_i64.to_le_bytes().to_vec();
    for part in &parts {
        buf.extend_from_slice(part);
    }

    let mut names = Fs {
        path: units(r"\proj\dir0\dir1\withheld.txt"),
    };
    let mut sink = Count::default();
    let stamp = Stamp { ts: 1, qpc: 1 };
    let delta = measure(20, 500, || {
        sink.events = 0;
        sink.unresolved = 0;
        let _ = feed.decode(black_box(&buf), stamp, &mut names, &mut sink);
    });
    // A run that took nothing measured nothing; the fake resolves every
    // withheld name, so no row is flagged.
    if sink.events != RECORDS || sink.unresolved != 0 || delta.allocs != 0 || delta.bytes != 0 {
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
