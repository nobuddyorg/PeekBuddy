# 0010. Pin Win32 bindings and keep the core dependency-light

- Status: Accepted
- Date: 2026-10-22
- Implemented or verified by: #9, #12, #83

## Context and problem statement

Win32 bindings change shape between releases; a surprise upgrade can change
`unsafe` call signatures. Generic file-watching crates hide the overflow
semantics this project depends on.

## Decision drivers

- `unsafe` code must not change behind a lockfile update.
- Zero undetected loss needs direct control over the Win32 calls.

## Considered options

1. Use the `notify` crate in the core.
2. Use `windows-sys` directly, exactly pinned (F20).

## Decision outcome

Chosen option 2.

- `windows-sys = "=0.61.2"` in `[workspace.dependencies]`.
- No `notify` crate in the core or engine.
- cargo-deny `multiple-versions = "warn"`.
- Dependabot auto-merges only patch updates of 0.x crates; everything else,
  and every `windows-sys` bump, is a reviewed PR (#12).
- Supply-chain checks run through cargo-deny's advisories (RustSec). The
  rustsec `audit-check` action is not used.

### Filter engine dependencies (#40)

`peekbuddy-core` stays pure Rust and gains three dependencies, all MIT or
Apache-2.0:

- `regex-automata` 0.4.18 (with `regex-syntax` 0.8.11): fully built dense
  DFAs, so matching has no lazy cache and never allocates. Features:
  `dfa-build`, `dfa-search`, `std`, `syntax`, `unicode`.
- `toml` 1.1.8 (`parse` and `serde` only) and `serde`: the embedded preset
  files.
- `rstest` 0.28.0, `insta` 1.49.0 and `proptest-state-machine` 0.9.0 are
  test-only.

## Consequences

- Good: Win32 changes are always deliberate.
- Bad: manual bumps (tracked in #83).

## Evidence

- windows-sys 0.61.2, checked 2026-10-22: <https://crates.io/crates/windows-sys/0.61.2>
- notify 8.2.0 (rejected for the core), checked 2026-10-22: <https://crates.io/crates/notify/8.2.0>
- regex-automata 0.4.18, checked 2026-10-24: <https://crates.io/crates/regex-automata/0.4.18>
- toml 1.1.8, checked 2026-10-24: <https://crates.io/crates/toml/1.1.8>
- cargo-deny, checked 2026-10-22: <https://embarkstudios.github.io/cargo-deny/>
