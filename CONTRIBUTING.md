# Contributing

See [TEST_STRATEGY.md](TEST_STRATEGY.md) for tests and [AGENTS.md](AGENTS.md)
for the agent workflow. Security reports go through [SECURITY.md](SECURITY.md);
participation follows the [code of conduct](CODE_OF_CONDUCT.md).

## No outside contributions

PeekBuddy is a personal hobby project. I don't accept pull requests, issues
or feature requests from outside the project; they are closed without
review.

Want to change something? **Fork it.** The [MIT License](LICENSE) lets you
use, change and redistribute the code in your own fork, no need to ask. There
is no support for forks.

The rest of this guide is how the project itself is worked on; it applies
equally to a fork.

## Workspace layout

| Crate | Owns | `unsafe` |
| --- | --- | --- |
| `crates/peekbuddy-core` | Event model, names, filters, derived views. Pure Rust, builds and tests on Linux, no `cfg(windows)`. | forbidden |
| `crates/peekbuddy-alloc` | Counting global allocator for allocation-budget tests (dev-only). | allowed (`GlobalAlloc` is unsafe) |
| `crates/peekbuddy-churn` | Churn generator: plans, executor, ledger and comparator; the ground truth for correctness and perf runs (dev-only, #25). Win32 it needs comes from `peekbuddy-win`. | forbidden |
| `crates/peekbuddy-win` | `windows-sys` wrappers and OS-independent record parsers (these also compile on Linux for Miri). | allowed, every block needs `// SAFETY:` |
| `crates/peekbuddy-store` | SQLite day segments, writer thread, queries, export. | forbidden |
| `crates/peekbuddy-engine` | Capture threads, reconcile scanner, tray, pipe server. No GPU or UI crates. | forbidden |
| `crates/peekbuddy-viewer` | The egui viewer window. | forbidden |
| `crates/peekbuddy` | The single `peekbuddy.exe`: engine by default, `--viewer` for the viewer. | forbidden |
| `xtask` | Repository automation (`cargo xtask <command>`). | forbidden |
| `benches` | Micro-benchmarks. | forbidden |

Two checks keep the layout honest:

- `cargo xtask check-unsafe`: every crate root has `#![forbid(unsafe_code)]`
  except the allow-list in `xtask/src/unsafe_code.rs` (`peekbuddy-win` and
  `peekbuddy-alloc`).
- `cargo xtask check-deps`: `peekbuddy-core` and `peekbuddy-engine` never
  depend on `eframe`, `egui`, `winit`, `wgpu` or `glow` (ADR 0001), and no
  crate uses PDH: no `Win32_System_Performance` feature, no `Pdh*` call, no
  `pdh.dll` (#24). Process metrics come from `peekbuddy_win::metrics`.

## Running tests

```sh
cargo nextest run            # unit and integration tests
cargo test --doc             # doctests (nextest does not run them)
cargo xtask check            # the full local loop before a push
```

See [TEST_STRATEGY.md](TEST_STRATEGY.md) for layers, groups and helpers.

## Performance rule

Performance is a feature. Every PR that touches a hot path (capture, parse,
filter, writer, viewer frame) shows numbers in its description: the
same-runner A/B timing from CI or the allocation counts (#23, #26). Budgets
and gate policy: [ADR 0007](docs/adr/0007-performance-gate-policy.md).

## Lint policy

Warnings are errors in CI (`CARGO_BUILD_WARNINGS=deny`). `unwrap`, `expect`,
`panic`, `todo`, `dbg!` and printing macros are denied outside tests; indexing
and slicing are denied in `peekbuddy-core` and `peekbuddy-win`. Use
`#[expect(..., reason = "...")]`, never `#[allow]`.

## Hooks and commit messages

The repository uses [prek](https://github.com/j178/prek) (pre-commit
compatible) with the hooks in `.pre-commit-config.yaml`:

```sh
pip install prek==0.5.7   # or: uv tool install prek
prek install              # installs the pre-commit and commit-msg hooks
prek run --all-files      # everything, once
```

The first run builds `taplo`, `cargo-sort` and `committed` from source and
takes a few minutes; later runs take seconds. `cargo-clippy` is slow on a cold
cache: skip it locally with `SKIP=cargo-clippy git commit ...`; CI still runs
it on Windows.

Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/),
checked by [committed](https://github.com/crate-ci/committed) (`committed.toml`):
`type(scope): summary` with a lowercase summary of at most 50 characters,
body lines of at most 72. Allowed types: `fix`, `feat`, `chore`, `docs`,
`style`, `refactor`, `perf`, `test`, `build`, `ci`. Commits by Dependabot and
release-plz are exempt.

```text
update stuff                  rejected
feat(core): add event kind    accepted
```

## CI gates

`ci-ok` is the only required check; it fails when any required job fails, is
cancelled or is skipped. Jobs in `.github/workflows/ci.yml`:

| Job | Runner | Gate |
| --- | --- | --- |
| `prek` | ubuntu-latest | required: all hooks except clippy; commit messages of the PR |
| `core (linux)` | ubuntu-latest | required: `peekbuddy-core`, `peekbuddy-alloc` and `peekbuddy-churn` tests on Linux; `xtask budgets --check` |
| `lint (windows x64)` | `windows-2025` | required: fmt, clippy, rustdoc, cargo-machete, `check-unsafe`, `check-deps`, benches compile |
| `test (windows x64)` | `windows-2025` | required: preflight, nextest, doctests, release build without Tracy (`check-release --exe`, #22) and within the size cap (`xtask size`, #28); `xtask churn` edge-mix exercises every operation class, burst throughput reported (#25) |
| `bench a/b (windows x64)` | `windows-2025` | required on PRs: same-runner A/B of the criterion benches, warn +15 %, fail +50 % (#26); skipped (and accepted by `ci-ok`) on push and merge group |
| `test (windows arm64) (advisory)` | `windows-11-vs2026-arm` | advisory until two weeks of green runs, then promoted |

Perf (#23, #26, #27) and release-build (#28) jobs join as those issues land.
`profiling.yml` builds the Tracy profiling exe weekly and on PRs that touch
it (#22); it is not part of `ci-ok`. The bench A/B table is in the job summary
of `bench a/b (windows x64)` (no PR comment), and `bench-noop.yml` is the
gate's false-positive check (#26); it is not part of `ci-ok`. The nightly
`bench trend (windows x64)` job publishes bench medians to gh-pages.
The `main` ruleset for the owner to apply is in `.github/rulesets/main.json`
with `ci-ok` and `CodeQL (rust)` as required checks. `codeql.yml` runs CodeQL
for Rust on every PR; `scorecard.yml` runs OpenSSF Scorecard weekly.
