//! Listing a directory for the reconcile walk allocates nothing once its
//! buffers exist (#46, #23): parse a 64 KiB `FILE_ID_EXTD_DIR_INFO` buffer
//! into a reused [`Listing`] and sort it. `harness = false`: libtest itself
//! allocates.

use std::alloc::System;
use std::hint::black_box;
use std::process::ExitCode;

use peekbuddy_alloc::{CountingAlloc, answer_nextest_list, measure};
use peekbuddy_core::reconcile::dirinfo::{self, ALIGN, HEADER_LEN, layout};
use peekbuddy_core::reconcile::{Listing, Meta};

#[global_allocator]
static ALLOC: CountingAlloc<System> = CountingAlloc::new(System);

const BUFFER: usize = 64 * 1024;

/// A buffer of records with names of varying length; returns it and the
/// record count.
fn build() -> (Vec<u8>, usize) {
    let mut buf = vec![0_u8; BUFFER];
    let (mut at, mut count, mut last) = (0_usize, 0_usize, 0_usize);
    loop {
        // Names go z..a so the sort has work to do.
        let name: Vec<u16> = format!("file-{:05}.dat", 99_999 - count)
            .encode_utf16()
            .collect();
        let size = (HEADER_LEN + name.len() * 2).next_multiple_of(ALIGN);
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
        put(layout::ATTRIBUTES, &0x20_u32.to_le_bytes());
        put(
            layout::NAME_LENGTH,
            &u32::try_from(name.len() * 2).unwrap_or(0).to_le_bytes(),
        );
        put(layout::FILE_ID, &(count as u128 + 1).to_le_bytes());
        for (i, unit) in name.iter().enumerate() {
            put(layout::NAME + i * 2, &unit.to_le_bytes());
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
    if answer_nextest_list("reconcile_alloc") {
        return ExitCode::SUCCESS;
    }
    let (buf, expected) = build();
    let mut listing = Listing::new();
    let mut seen = 0;
    let delta = measure(20, 500, || {
        listing.clear();
        for entry in dirinfo::parse(black_box(&buf), buf.len()) {
            let Ok(entry) = entry else { return };
            listing.push_units(
                entry.name.len(),
                entry.name.units(),
                Meta {
                    file_id: entry.file_id,
                    attributes: entry.attributes,
                    reparse_tag: entry.reparse_tag,
                    size: u64::try_from(entry.size).unwrap_or(0),
                    write_time: entry.write_time,
                    change_time: entry.change_time,
                },
                0,
            );
        }
        listing.sort();
        seen = listing.len();
    });
    // A run that stopped early, or an unsorted listing, measured nothing.
    let sorted = listing.items().windows(2).all(|pair| match pair {
        [a, b] => listing.name(a) <= listing.name(b),
        _ => true,
    });
    if seen != expected || !sorted || delta.allocs != 0 || delta.bytes != 0 {
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
