# Test strategy

PeekBuddy's claims (zero undetected loss, raw is truth, name fidelity) are
only as good as its tests. This page says which tests exist, where they run
and how to add one.

## Pyramid

| Layer | What | Where it lives | Runs on |
| --- | --- | --- | --- |
| Unit and property | Pure logic: event model, names, filters, derived views, parsers | `mod tests` in each module; proptest | Linux and Windows, every PR |
| Win32 integration | Real watchers, real NTFS, pipes, tokens | `tests/` of `peekbuddy-win`, `-engine`, `-store` | Windows x64 every PR, arm64 advisory |
| End-to-end | Launch `peekbuddy.exe`, drive it, check the log | `tests/e2e*.rs` | Windows, every PR (`e2e` group) |
| UI | egui_kittest snapshots and UIA smoke | `peekbuddy-viewer` (#71) | Windows |
| Performance ([PERFORMANCE.md](PERFORMANCE.md)) | Allocation budgets (#23), micro-benchmarks (#26), end-to-end harness (#27) | own test binaries and `benches` | Windows, PR (deterministic) and nightly (trends) |

## Runner and profiles

[cargo-nextest](https://nexte.st/) runs the tests; doctests run separately
with `cargo test --doc` (nextest does not run them). Configuration is in
`.config/nextest.toml`:

- `default`: slow tests are terminated after 2 × 60 s; a test that leaves a
  child process or handle behind for more than 500 ms fails as `LEAK-FAIL`.
- `ci`: no fail-fast, no retries, JUnit to `target/nextest/ci/junit.xml`.
- Test group `fs-serial` (one at a time): every test that opens a watcher, a
  VHD or a USN journal, or shares a fixed root. Test binaries named
  `fs_serial*` join it automatically; add an override for other binaries.
- Test group `e2e` (two at a time, retried twice with exponential backoff):
  tests that launch `peekbuddy.exe`. A retried pass shows as flaky in JUnit
  and needs an issue.

`PROPTEST_CASES=256` on PRs; the nightly run raises it (#17).

## What runs when

| Job | PR | Nightly |
| --- | --- | --- |
| `core (linux)`: `peekbuddy-core` tests | yes | yes |
| `test (windows x64)`: nextest `ci` + doctests | yes (required) | yes |
| `test (windows arm64)` | yes (advisory) | yes |
| Coverage (#14), mutation in diff (#15) | yes | full shards |
| Miri, fuzzing, long proptests, beta canary (#17) | no | yes |

## Allocation tests

`peekbuddy-alloc` provides `CountingAlloc`, a counting wrapper around the
system allocator (#23). Allocation tests are integration tests with
`harness = false` (libtest's own threads and output capture allocate), one
binary per budget, that install the wrapper as `#[global_allocator]`:

```rust
#[global_allocator]
static ALLOC: CountingAlloc<System> = CountingAlloc::new(System);

fn main() -> ExitCode {
    if peekbuddy_alloc::answer_nextest_list("alloc_perf") {
        return ExitCode::SUCCESS; // nextest's --list probe
    }
    let delta = peekbuddy_alloc::measure(1_000, 100_000, || work());
    if delta.allocs == 0 { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
```

Wrap inputs and outputs in `std::hint::black_box`, or the optimiser can
remove an allocation and hide a regression. Current binaries:
`peekbuddy-alloc/tests/alloc_self` (the counter itself, with a Vec-push
negative control) and `peekbuddy-win/tests/alloc_perf` (milestone and ETW
writes: 0 allocations). Capture, writer and viewer budgets follow with
issues #44, #52 and #66.

## Nightly jobs

`.github/workflows/nightly.yml` runs at 02:17 UTC. A failing job opens or
updates one issue labelled `nightly`; the next green run closes it. The
owner triages `nightly` issues; an agent picks them up like any other issue.

| Job | Runner | What |
| --- | --- | --- |
| `miri` | ubuntu | `cargo +nightly miri test` for `peekbuddy-core`, `peekbuddy-win`, `peekbuddy-alloc` and `peekbuddy-churn` (OS-independent code, including the unsafe record parsers, the counting allocator and the churn comparator) |
| `fuzz` | ubuntu | every cargo-fuzz target for 20 minutes ([fuzzing](docs/how-to/fuzzing.md)) |
| `long property tests` | `windows-2025` | `PROPTEST_CASES=10000` |
| `fs-serial stress` | `windows-2025` | `fs-serial` tests repeated for 30 minutes (`--stress-duration`) |
| `beta canary` | `windows-2025` | clippy and tests on beta, ahead of the toolchain bump |
| `fresh dependencies` | `windows-2025` | `cargo update`, then build and test |
| `mutants` | `windows-2025` × 8 shards, plus core on ubuntu | full mutation run ([mutation testing](docs/how-to/mutation-testing.md)) |

## Helpers: `peekbuddy-testkit`

A dev-only crate:

- `TestDir::new(name)`: a unique temp root under a short base
  (`%TEMP%\pbt`, or `PEEKBUDDY_TEST_ROOT`). `on_cleanup` registers hooks
  (stop and join watchers) that run before removal; removal retries with
  backoff because an open handle blocks deletion on Windows. `close()`
  reports failures, `Drop` cannot.
- `TestDir::fixed(name)`: a fixed root that first removes what a killed run
  left behind. Only for `fs-serial` tests.
- `snapshot_filters()`: insta filters for backslashes, drive letters, CRLF and
  timestamps, so snapshots authored on Linux pass on Windows.
- `run_unelevated(cmd)` (Windows): runs a child with Administrators
  deny-only, Medium integrity and no console (`DETACHED_PROCESS`; with a
  console such a child dies with `STATUS_DLL_INIT_FAILED`, #34). Hosted
  runners are elevated (F12), so unelevated behaviour is only testable this
  way. The child reports through its exit code; a test can re-run its own
  binary as the child with `<exe> <ignored test> --exact --ignored`.
- `assert_interactive_session()` (Windows): fails unless the test runs in an
  interactive session (tray and window tests).

## Adding a test

**Unit**, next to the code:

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn rename_pairs_old_and_new() {
        assert_eq!(super::pair("a", "b"), ("a", "b"));
    }
}
```

**Property** (proptest; commit the `proptest-regressions/` files):

```rust
proptest::proptest! {
    #[test]
    fn utf16_round_trips(units in proptest::collection::vec(proptest::num::u16::ANY, 0..64)) {
        proptest::prop_assert_eq!(decode(&encode(&units)), units);
    }
}
```

**Win32 integration**, in `tests/fs_serial_<topic>.rs` so it joins
`fs-serial`:

```rust
#[test]
fn create_is_reported() {
    let mut dir = peekbuddy_testkit::TestDir::new("create").unwrap();
    let watcher = start_watcher(dir.path());
    dir.on_cleanup(move || watcher.stop_and_join());
    std::fs::write(dir.path().join("a.txt"), b"x").unwrap();
    // assert the event
    dir.close().unwrap();
}
```

**Snapshot** (file snapshots only, never inline):

```rust
insta::with_settings!({ filters => peekbuddy_testkit::snapshot_filters() }, {
    insta::assert_snapshot!("event_row", render(&event));
});
```

**Matrix** (rstest):

```rust
#[rstest::rstest]
#[case::ntfs("NTFS")]
#[case::refs("ReFS")]
fn volume_kinds(#[case] fs: &str) { /* ... */ }
```

## Conventions

- `mod tests` per module; integration tests in `tests/`; allocation tests in
  their own test binaries (#23), because a counting allocator is global.
- Each nextest test is its own process: set up per-process Win32 state (COM,
  message loop, tray) in the test.
- On Windows nextest kills timed-out tests immediately, without cleanup: tests
  must be idempotent and clean up stale state at the start.
- Dev-dependency versions live in `[workspace.dependencies]`: proptest 1.12.0,
  proptest-state-machine 0.9.0, insta 1.49.0, rstest 0.28.0. Each is added
  there with its first user, because Cargo 1.100 warns on unused workspace
  dependencies.

## Coverage

CI measures coverage on Windows x64 with cargo-llvm-cov 0.9.1 and uploads it
to Codecov (patch target 80 %; `codecov.yml`). arm64 is not measured
(malformed `.profraw` files, rust-lang/rust#150123). Locally:

```sh
cargo install --locked cargo-llvm-cov@0.9.1
cargo llvm-cov nextest --open
```

On Linux this covers only code that builds there; `cfg(windows)` code is
measured in CI.

## Flake policy

Flakes are bugs. No retries outside the `e2e` group. A flaky test gets an
issue within one day, or it is quarantined with a link to that issue. Never
loosen a timeout or an assertion to get green.

## Runner assumptions

The repository is public (#4), so hosted Windows runners have 4 vCPU and
16 GB. Test-group thread counts assume that size.
