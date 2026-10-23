//! A counting global allocator for allocation-budget tests (#23, F10).
//!
//! Wrap the real allocator in [`CountingAlloc`] and install it in a test
//! binary:
//!
//! ```ignore
//! #[global_allocator]
//! static ALLOC: peekbuddy_alloc::CountingAlloc<std::alloc::System> =
//!     peekbuddy_alloc::CountingAlloc::new(std::alloc::System);
//! ```
//!
//! Then measure a closure with [`measure`] or an [`AllocScope`]. Counting is a
//! few relaxed atomic adds per call and never allocates itself: the
//! per-thread counters are `const`-initialised `Cell`s without `Drop`, so
//! touching them cannot recurse into the allocator.
//!
//! This crate holds the only `unsafe` outside `peekbuddy-win`: implementing
//! [`GlobalAlloc`] is unsafe by definition.

use std::alloc::{GlobalAlloc, Layout};
use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};

static ALLOCS: AtomicU64 = AtomicU64::new(0);
static FREES: AtomicU64 = AtomicU64::new(0);
static REALLOCS: AtomicU64 = AtomicU64::new(0);
static BYTES_ALLOCATED: AtomicU64 = AtomicU64::new(0);
static LIVE_BYTES: AtomicU64 = AtomicU64::new(0);
static PEAK_LIVE_BYTES: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static THREAD_ALLOCS: Cell<u64> = const { Cell::new(0) };
    static THREAD_BYTES: Cell<u64> = const { Cell::new(0) };
}

/// Wraps an allocator and counts what goes through it.
#[derive(Debug, Default)]
pub struct CountingAlloc<A> {
    inner: A,
}

impl<A> CountingAlloc<A> {
    /// Wraps `inner`; usable in a `static`.
    pub const fn new(inner: A) -> Self {
        Self { inner }
    }
}

fn widen(size: usize) -> u64 {
    u64::try_from(size).unwrap_or(u64::MAX)
}

fn on_alloc(size: usize) {
    let size = widen(size);
    ALLOCS.fetch_add(1, Ordering::Relaxed);
    BYTES_ALLOCATED.fetch_add(size, Ordering::Relaxed);
    let live = LIVE_BYTES
        .fetch_add(size, Ordering::Relaxed)
        .saturating_add(size);
    PEAK_LIVE_BYTES.fetch_max(live, Ordering::Relaxed);
    // `try_with` fails only while the thread's TLS is being torn down; such
    // late allocations are counted globally but not per thread.
    let _ = THREAD_ALLOCS.try_with(|c| c.set(c.get().saturating_add(1)));
    let _ = THREAD_BYTES.try_with(|c| c.set(c.get().saturating_add(size)));
}

fn on_free(size: usize) {
    FREES.fetch_add(1, Ordering::Relaxed);
    LIVE_BYTES.fetch_sub(widen(size), Ordering::Relaxed);
}

// SAFETY: every method forwards to `inner` with the caller's arguments
// unchanged, so `inner`'s guarantees carry over; the counting around the
// calls touches only atomics and const-initialised thread locals and never
// allocates.
unsafe impl<A: GlobalAlloc> GlobalAlloc for CountingAlloc<A> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller upholds `alloc`'s contract for `layout`.
        let ptr = unsafe { self.inner.alloc(layout) };
        if !ptr.is_null() {
            on_alloc(layout.size());
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller upholds `alloc_zeroed`'s contract for `layout`.
        let ptr = unsafe { self.inner.alloc_zeroed(layout) };
        if !ptr.is_null() {
            on_alloc(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: the caller guarantees `ptr` came from this allocator with
        // `layout`.
        unsafe { self.inner.dealloc(ptr, layout) };
        on_free(layout.size());
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller upholds `realloc`'s contract.
        let new = unsafe { self.inner.realloc(ptr, layout, new_size) };
        if !new.is_null() {
            REALLOCS.fetch_add(1, Ordering::Relaxed);
            // A realloc is a free of the old block and an allocation of the
            // new one, so live bytes and per-thread counts stay exact.
            on_free(layout.size());
            on_alloc(new_size);
        }
        new
    }
}

/// Process-wide totals since start.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    /// Successful allocations (including the new block of each realloc).
    pub allocs: u64,
    /// Frees (including the old block of each realloc).
    pub frees: u64,
    /// Successful reallocations.
    pub reallocs: u64,
    /// Bytes ever allocated.
    pub bytes_allocated: u64,
    /// Bytes currently live.
    pub live_bytes: u64,
    /// Highest live bytes seen.
    pub peak_live_bytes: u64,
}

/// Reads the process-wide totals.
#[must_use]
pub fn stats() -> Stats {
    Stats {
        allocs: ALLOCS.load(Ordering::Relaxed),
        frees: FREES.load(Ordering::Relaxed),
        reallocs: REALLOCS.load(Ordering::Relaxed),
        bytes_allocated: BYTES_ALLOCATED.load(Ordering::Relaxed),
        live_bytes: LIVE_BYTES.load(Ordering::Relaxed),
        peak_live_bytes: PEAK_LIVE_BYTES.load(Ordering::Relaxed),
    }
}

/// Allocations made by one thread between [`AllocScope::begin`] and
/// [`AllocScope::end`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AllocDelta {
    /// Number of allocations.
    pub allocs: u64,
    /// Bytes allocated.
    pub bytes: u64,
}

/// Counts the current thread's allocations over a region.
#[derive(Debug, Clone, Copy)]
pub struct AllocScope {
    allocs: u64,
    bytes: u64,
}

fn thread_counts() -> (u64, u64) {
    (
        THREAD_ALLOCS.try_with(Cell::get).unwrap_or(0),
        THREAD_BYTES.try_with(Cell::get).unwrap_or(0),
    )
}

impl AllocScope {
    /// Starts counting on the current thread.
    #[must_use]
    pub fn begin() -> Self {
        let (allocs, bytes) = thread_counts();
        Self { allocs, bytes }
    }

    /// Stops counting and returns what this thread allocated since `begin`.
    #[must_use]
    pub fn end(self) -> AllocDelta {
        let (allocs, bytes) = thread_counts();
        AllocDelta {
            allocs: allocs.saturating_sub(self.allocs),
            bytes: bytes.saturating_sub(self.bytes),
        }
    }
}

/// Runs `body` `warmup` times, then counts the allocations of `iterations`
/// further runs on the current thread.
pub fn measure(warmup: u64, iterations: u64, mut body: impl FnMut()) -> AllocDelta {
    for _ in 0..warmup {
        body();
    }
    let scope = AllocScope::begin();
    for _ in 0..iterations {
        body();
    }
    scope.end()
}

/// For `harness = false` test binaries: answers nextest's
/// `--list --format terse` with one test called `name` and returns `true`;
/// returns `false` for a normal run. Without this nextest skips the binary.
#[must_use]
pub fn answer_nextest_list(name: &str) -> bool {
    let mut args = std::env::args();
    let listing = args.any(|arg| arg == "--list");
    if !listing {
        return false;
    }
    if !std::env::args().any(|arg| arg == "--ignored") {
        let mut out = std::io::stdout();
        let _ = std::io::Write::write_all(&mut out, name.as_bytes());
        let _ = std::io::Write::write_all(&mut out, b": test\n");
    }
    true
}

#[cfg(test)]
mod tests {
    use super::{AllocScope, CountingAlloc, measure, stats};
    use std::alloc::{GlobalAlloc, Layout, System};

    // Not installed as the global allocator: these tests call it directly,
    // so only their own calls are counted on this thread.
    static COUNTING: CountingAlloc<System> = CountingAlloc::new(System);

    fn alloc(layout: Layout) -> *mut u8 {
        // SAFETY: every layout in these tests has a non-zero size.
        let ptr = unsafe { COUNTING.alloc(layout) };
        assert!(!ptr.is_null());
        ptr
    }

    fn free(ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` came from `COUNTING` with `layout`.
        unsafe { COUNTING.dealloc(ptr, layout) };
    }

    #[test]
    fn alloc_and_free_are_counted_per_thread() {
        let layout = Layout::from_size_align(64, 8).unwrap();
        let scope = AllocScope::begin();
        free(alloc(layout), layout);
        let delta = scope.end();
        assert_eq!(delta.allocs, 1);
        assert_eq!(delta.bytes, 64);
    }

    #[test]
    fn zeroed_and_aligned_blocks_are_counted() {
        let layout = Layout::from_size_align(256, 64).unwrap();
        let scope = AllocScope::begin();
        // SAFETY: non-zero layout.
        let ptr = unsafe { COUNTING.alloc_zeroed(layout) };
        assert!(!ptr.is_null());
        assert_eq!(ptr.align_offset(64), 0);
        // SAFETY: the block is at least one byte long and zeroed.
        assert_eq!(unsafe { *ptr }, 0);
        free(ptr, layout);
        assert_eq!(scope.end().allocs, 1);
    }

    #[test]
    fn realloc_counts_one_new_block() {
        let layout = Layout::from_size_align(16, 8).unwrap();
        let before = stats().reallocs;
        let scope = AllocScope::begin();
        let ptr = alloc(layout);
        // SAFETY: `ptr` came from `COUNTING` with `layout`; 1024 is non-zero.
        let grown = unsafe { COUNTING.realloc(ptr, layout, 1024) };
        assert!(!grown.is_null());
        free(grown, Layout::from_size_align(1024, 8).unwrap());
        let delta = scope.end();
        assert_eq!(delta.allocs, 2);
        assert_eq!(delta.bytes, 16 + 1024);
        assert!(stats().reallocs > before);
    }

    #[test]
    fn measure_runs_warmup_then_counts_iterations() {
        let layout = Layout::from_size_align(8, 8).unwrap();
        let mut runs = 0;
        let delta = measure(3, 5, || {
            runs += 1;
            free(alloc(layout), layout);
        });
        assert_eq!(runs, 8);
        assert_eq!(delta.allocs, 5);
        assert_eq!(delta.bytes, 40);
    }

    #[test]
    fn an_empty_scope_counts_nothing() {
        assert_eq!(measure(0, 100, || {}).allocs, 0);
    }
}
