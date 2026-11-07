# Mutation testing

[cargo-mutants](https://mutants.rs/) 27.1.0 changes the code (a `+` becomes a
`-`, a function returns its default) and checks that some test fails. A
mutant that survives is code no test really checks.

## Where it runs

- **Nightly** (#17): the whole workspace in 32 shards on Windows, plus
  `peekbuddy-core` in 4 shards on Linux, 360 minutes each. Never part of the
  PR flow. Incremental on Monday to Saturday: only mutants in lines changed
  in the last 26 hours (`--in-diff`), without a score gate; Sundays and
  manual runs examine everything and gate on the score.

What is examined is set in `.cargo/mutants.toml` (`examine_re`,
`exclude_re`). The nightly gate is a score of 80 %
(caught / caught + missed + timeout).

## Running locally

```sh
cargo install --locked cargo-mutants@27.1.0
cargo mutants -p peekbuddy-core            # one crate
git diff origin/main... > git.diff
cargo mutants --in-diff git.diff           # like the PR job
```

On Windows, set `TMP` and `TEMP` to a short path (for example `C:\t`) or use
`--in-place`: long temp paths hit MAX_PATH (`os error 3` or `206`).

Builds use `[profile.mutants]` (no debug info). Doctests do not run under
nextest, so they never count as catching a mutant.

## Reading `mutants.out`

- `caught.txt`: mutants a test caught (good).
- `missed.txt`: mutants no test caught. Add or sharpen a test.
- `unviable.txt`: mutants that do not compile (ignored).
- `timeout.txt`: mutants that made tests hang; usually a loop condition. Treat
  like missed unless the hang is the expected failure.
- `diff/`: the patch of each mutant.

Two things end as a timeout. A **real hang** (a wait loop that never ends
because of the mutant): make the test fail fast with a short deadline, so it
is reported as caught. A **slow failure**: the mutant breaks a test that
waits for something with a long deadline (a viewer test with a 20 s `until`),
so the failing run is slower than the limit of the shard. The nightly gives
every mutant at least 150 s (`--minimum-test-timeout`) so slow failures are
caught; what still times out after that is a real hang.

## Survivors that need Win32 fault injection (#310, #336, #337)

The nightly of 2026-11-05 (run 42003537721) found 275 missed and 237 timed-out
mutants against 4,425 caught. Most of the rest sits behind Win32 calls or
thread timing that a unit test cannot reach. Per file (missed / timeouts):

| File | Missed | Timeouts | What it needs |
| --- | --- | --- | --- |
| `peekbuddy-engine/src/capture_host.rs` | 48 | 73 | a fake watcher with injected errors, overflow and shutdown |
| `peekbuddy-win/src/rdcw.rs` | 30 | 27 | a fake `ReadDirectoryChangesExW` (error codes, buffer overflow) |
| `peekbuddy-engine/src/usn_source.rs` | 26 | 0 | a fake journal reader (read errors, requery, drain) |
| `peekbuddy-engine/src/lib.rs` | 21 | 5 | a whole engine process (autostart registry, synthetic events) |
| `peekbuddy-win/src/chrome.rs`, `picker.rs` | 15 / 13 | 0 | a desktop (window chrome, COM dialog) |
| `peekbuddy-engine/src/instrument.rs` | 11 | 0 | an ETW consumer |
| `peekbuddy-engine/src/ipc_server.rs` | 10 | 4 | a scripted pipe client |
| `peekbuddy-store/src/pipeline.rs` | 9 | 27 | timing of the writer thread and the spill |
| `peekbuddy-win/src/pipe.rs` | 8 | 21 | wait results of a named pipe |
| the rest | about 80 | about 80 | one to seven each |

These are tracked in one place, issue #365, with the fakes they need. The
equivalent ones are excluded in `.cargo/mutants.toml` with their reason.

## When a skip is acceptable

Only when no test can reasonably observe the difference, for example a
log message, a `Debug` impl or a performance hint whose effect only a
benchmark sees. Mark it and say why on the same line:

```rust
#[cfg_attr(test, mutants::skip)] // only changes log wording
fn describe(&self) -> String { /* ... */ }
```

Never skip to get green; the nightly summary counts skips so growth is
visible.
