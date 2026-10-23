# Mutation testing

[cargo-mutants](https://mutants.rs/) 27.1.0 changes the code (a `+` becomes a
`-`, a function returns its default) and checks that some test fails. A
mutant that survives is code no test really checks.

## Where it runs

- **PR** (`mutants in diff (advisory)` in `ci.yml`): only mutants in lines the
  PR changes, on `windows-2025`, after `test (windows x64)`. Missed mutants
  show as annotations on the diff. Advisory for four weeks, then required
  for `peekbuddy-core` and `peekbuddy-store`.
- **Nightly** (#17): the whole workspace in 8 shards on Windows, plus
  `peekbuddy-core` on Linux.

What is examined is set in `.cargo/mutants.toml` (`examine_re`,
`exclude_re`). There is no score threshold: any missed mutant in examined
code fails.

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
