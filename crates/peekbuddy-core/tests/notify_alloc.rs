//! Parsing a `ReadDirectoryChanges*` buffer allocates nothing (#44).
//! `harness = false`: libtest itself allocates.

use std::alloc::System;
use std::hint::black_box;
use std::process::ExitCode;

use peekbuddy_alloc::{CountingAlloc, answer_nextest_list, measure};
use peekbuddy_core::notify::{Class, layout, parse};

#[global_allocator]
static ALLOC: CountingAlloc<System> = CountingAlloc::new(System);

const BUFFER: usize = 64 * 1024;

/// Fills a 64 KiB buffer with records of 8 to 64 unit names; returns the
/// buffer and how many records it holds.
fn build(class: Class) -> (Vec<u8>, usize) {
    let mut buf = vec![0_u8; BUFFER];
    let header = class.header_len();
    let mut at = 0_usize;
    let mut count = 0_usize;
    let mut last = 0_usize;
    loop {
        let units = 8 + (count * 7) % 57;
        let size = (header + units * 2).next_multiple_of(4);
        if at + size > BUFFER {
            break;
        }
        let name_len = u32::try_from(units * 2).unwrap_or(0);
        let name_at = match class {
            Class::Plain => layout::PLAIN_NAME_LENGTH,
            Class::Extended | Class::Full => layout::EXTENDED_NAME_LENGTH,
        };
        let step = u32::try_from(size).unwrap_or(0);
        for (offset, bytes) in [
            (layout::NEXT_ENTRY_OFFSET, step.to_le_bytes().to_vec()),
            (layout::ACTION, 3_u32.to_le_bytes().to_vec()),
            (name_at, name_len.to_le_bytes().to_vec()),
        ] {
            if let Some(dst) = buf.get_mut(at + offset..at + offset + bytes.len()) {
                dst.copy_from_slice(&bytes);
            }
        }
        if let Some(dst) = buf.get_mut(at + header..at + header + units * 2) {
            for (i, b) in dst.iter_mut().enumerate() {
                *b = if i % 2 == 0 {
                    b'a' + u8::try_from(i % 26).unwrap_or(0)
                } else {
                    0
                };
            }
        }
        last = at;
        at += size;
        count += 1;
    }
    // The last record ends the chain.
    if let Some(dst) = buf.get_mut(last..last + 4) {
        dst.fill(0);
    }
    (buf, count)
}

fn main() -> ExitCode {
    if answer_nextest_list("notify_alloc") {
        return ExitCode::SUCCESS;
    }
    for class in [Class::Plain, Class::Extended, Class::Full] {
        let (buf, expected) = build(class);
        let mut scratch = [0_u16; 128];
        let mut seen = 0_usize;
        let mut units = 0_usize;
        let delta = measure(10, 200, || {
            seen = 0;
            units = 0;
            for item in parse(black_box(&buf), buf.len(), class) {
                let Ok(record) = item else { return };
                seen += 1;
                units += record.name.copy_into(&mut scratch).unwrap_or(0);
                black_box(record.extended);
            }
        });
        black_box(units);
        // A parse that stopped early measured nothing.
        if seen != expected || units == 0 || delta.allocs != 0 || delta.bytes != 0 {
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}
