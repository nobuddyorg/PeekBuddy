# Working on PeekBuddy as an agent

Claude Code agents implement the issues; the owner reviews (owner answer O5
in #1). Agents run in Linux cloud containers, PeekBuddy is Windows-only, so
the loop is: fast checks on Linux, Windows CI as the test oracle, scripted
owner runs for real hardware.

Read [docs/architecture.md](docs/architecture.md), the
[ADRs](docs/adr/README.md) and [CONTRIBUTING.md](CONTRIBUTING.md) before
writing code.

## Local loop

```sh
cargo xtask check
```

It runs, in order: `cargo fmt --check`; prek (or typos while there is no
prek config); the Linux tests (nextest when installed) and doctests;
`xtask check-unsafe`, `check-deps` and `check-release`; clippy with
warnings denied for `x86_64-pc-windows-msvc` and `aarch64-pc-windows-msvc`.
Clippy for the Windows targets uses plain `cargo clippy --target`, which
needs no linker but cannot compile C code, so it checks the default features
and a host clippy run covers `--all-features` (Tracy, #22). Set
`PEEKBUDDY_XWIN=1` to run the Windows targets through `cargo xwin clippy`
with all features instead. Tests that need Win32 run only in CI.

## Crate rule

Anything that does not need Win32 lives in a crate that builds and tests on
Linux. Win32 access goes through traits in `peekbuddy-win` with in-memory
fakes, so engine logic is testable on Linux too.

## Code rules

- `unsafe` only in `peekbuddy-win`, every block with a `// SAFETY:` comment
  (`cargo xtask check-unsafe`).
- No `unwrap`, `expect`, `panic` or indexing outside tests; `#[expect]` with a
  reason, never `#[allow]` (lint policy in CONTRIBUTING.md).
- PRs that touch a hot path show A/B or allocation numbers (performance rule
  in CONTRIBUTING.md).

## Issue-to-PR contract

- Pick the lowest-numbered open issue whose "Blocked by" issues are all
  closed and that has no `agent-blocked` or `needs-owner-run` label.
- One issue per PR. Branch `issue/<n>-<slug>` (or the branch the session
  assigns). PR body starts with `Closes #<n>`. The PR title is a conventional
  commit (#10).
- The PR body copies the issue's acceptance criteria and ticks only items
  with evidence: a CI job link, an artifact, a test name or a perf number.
- Stay in scope. Follow-ups go into the PR description as proposed issues,
  not into the PR.
- Blocked on a question: comment on the issue with the question and a
  recommended answer, add `agent-blocked`, take the next issue. Never wait.
- Unclear detail: record a best guess in the issue and mention it in the PR
  so the owner can veto it.
- Performance-relevant PRs attach the A/B or allocation numbers from CI
  (#26, #23).
- Never disable, skip or loosen a gate to get green. No PR merges with a red
  `ci-ok`.

## Owner runs

Every step that needs real hardware is a PowerShell script under
`scripts/owner/`: no Rust toolchain needed, at most 15 minutes, writes a JSON
report. The PR gets `needs-owner-run`; the owner attaches the report. At most
one owner run per week unless a release is due. See
[docs/how-to/owner-runs.md](docs/how-to/owner-runs.md).

## Review flow

`ci-ok` (#11) plus Claude code review on every PR. The owner reviews the
diff and merges.
