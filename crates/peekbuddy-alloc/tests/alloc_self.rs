//! Self-test of the counting allocator installed as the global allocator.
//!
//! `harness = false`: libtest's own threads and output capture allocate, so
//! allocation tests run as a plain `main` (#23).

use std::alloc::{Layout, System, alloc, dealloc};
use std::hint::black_box;
use std::process::ExitCode;

use peekbuddy_alloc::{CountingAlloc, measure, stats};
use proptest::prelude::{Strategy, any, prop};
use proptest::test_runner::{Config, TestCaseError, TestRunner};

#[global_allocator]
static ALLOC: CountingAlloc<System> = CountingAlloc::new(System);

fn check(name: &str, ok: bool, failures: &mut u32) {
    if !ok {
        *failures += 1;
        // Not print!: a failing check names itself through the exit code
        // and this marker, written without allocating a format string.
        let _ = std::io::Write::write_all(&mut std::io::stderr(), name.as_bytes());
        let _ = std::io::Write::write_all(&mut std::io::stderr(), b": FAILED\n");
    }
}

fn main() -> ExitCode {
    if peekbuddy_alloc::answer_nextest_list("alloc_self") {
        return ExitCode::SUCCESS;
    }
    let mut failures = 0;

    // A no-op allocates nothing.
    check(
        "noop",
        measure(10, 10_000, || {}).allocs == 0,
        &mut failures,
    );

    // A Vec push is seen (the negative control for every budget test).
    let pushes = measure(0, 100, || {
        let v = vec![black_box(1u8)];
        black_box(&v);
    });
    check("vec_push_is_counted", pushes.allocs == 100, &mut failures);
    check("vec_push_bytes", pushes.bytes >= 100, &mut failures);

    // Random alloc/free sequences: live bytes return to where they started
    // and the peak never drops below live.
    let mut runner = TestRunner::new(Config {
        cases: 64,
        ..Config::default()
    });
    let sizes = prop::collection::vec((1usize..4096, any::<bool>()), 1..64);
    let result = runner.run(&sizes.boxed(), |ops| {
        let mut live: Vec<(*mut u8, Layout)> = Vec::with_capacity(ops.len());
        let before = stats();
        // Borrow `ops`: consuming it would free its buffer inside the window.
        for &(size, free_early) in &ops {
            let layout =
                Layout::from_size_align(size, 8).map_err(|e| TestCaseError::fail(e.to_string()))?;
            // SAFETY: non-zero size; freed below with the same layout.
            let ptr = unsafe { alloc(layout) };
            if ptr.is_null() {
                return Err(TestCaseError::fail("allocation failed"));
            }
            if free_early {
                // SAFETY: `ptr` was just allocated with `layout`.
                unsafe { dealloc(ptr, layout) };
            } else {
                live.push((ptr, layout));
            }
            let now = stats();
            if now.peak_live_bytes < now.live_bytes {
                return Err(TestCaseError::fail("peak below live"));
            }
        }
        for (ptr, layout) in live.drain(..) {
            // SAFETY: each pointer was allocated with its layout above.
            unsafe { dealloc(ptr, layout) };
        }
        let after = stats();
        if after.live_bytes != before.live_bytes {
            return Err(TestCaseError::fail("live bytes did not return"));
        }
        Ok(())
    });
    check("proptest_live_and_peak", result.is_ok(), &mut failures);

    if failures == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
