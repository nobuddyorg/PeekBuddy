//! The churn classifier allocates nothing per change (#265, #23, F10): it is
//! pure arithmetic on the record. `harness = false`: libtest itself
//! allocates.

use std::alloc::System;
use std::hint::black_box;
use std::process::ExitCode;

use peekbuddy_alloc::{CountingAlloc, answer_nextest_list, measure};
use peekbuddy_core::event::map::usn_reason as reason;
use peekbuddy_core::reparse::{
    Change, SOURCE_CLIENT_REPLICATION_MANAGEMENT, attr, classify, link_kind, tag,
};

#[global_allocator]
static ALLOC: CountingAlloc<System> = CountingAlloc::new(System);

/// A mix of what a sync client and a user produce: placeholders flipping,
/// replicated writes, user writes, plain files, links.
fn changes() -> Vec<Change> {
    let cloud = tag::CLOUD | 0x6000;
    let make = |tag: u32, reason: u32, source_info: u32, before: u32, after: u32| Change {
        tag,
        tag_before: Some(tag),
        reason,
        source_info,
        attrs_before: Some(before),
        attrs_after: after,
    };
    vec![
        make(
            cloud,
            reason::BASIC_INFO_CHANGE | reason::CLOSE,
            0,
            0x20,
            0x20,
        ),
        make(
            cloud,
            reason::DATA_EXTEND | reason::CLOSE,
            SOURCE_CLIENT_REPLICATION_MANAGEMENT,
            0x20,
            0x20,
        ),
        make(cloud, reason::DATA_EXTEND, 0, 0x20, 0x20),
        make(
            cloud,
            reason::REPARSE_POINT_CHANGE,
            0,
            0x20 | attr::RECALL_ON_DATA_ACCESS,
            0x20,
        ),
        make(cloud, reason::FILE_CREATE, 0, 0, 0x20),
        make(0, reason::BASIC_INFO_CHANGE, 0, 0x20, 0x20),
        make(tag::SYMLINK, reason::REPARSE_POINT_CHANGE, 0, 0x400, 0x400),
        make(tag::APPEXECLINK, 0, 0, 0x400, 0x400),
    ]
}

fn main() -> ExitCode {
    if answer_nextest_list("reparse_alloc") {
        return ExitCode::SUCCESS;
    }
    let changes = changes();
    let mut churn = 0_usize;
    let delta = measure(100, 20_000, || {
        churn = 0;
        for change in &changes {
            churn += usize::from(classify(black_box(change)).is_some());
            black_box(link_kind(black_box(change.tag)));
        }
    });
    // The mix has both answers: the classifier really ran.
    if churn == 0 || churn == changes.len() {
        return ExitCode::FAILURE;
    }
    if delta.allocs == 0 && delta.bytes == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
