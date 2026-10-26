# 0016. Pause keeps the watches armed

- Status: Accepted
- Date: 2026-10-26
- Implemented or verified by: #63

## Context and problem statement

Users pause PeekBuddy during noisy work (a build, a sync) and expect to see
afterwards what was skipped. A paused watch must never be silently dead (F19),
and recorded rows stay the truth (F3).

Two ways to pause:

- **A**: the directory handles stay armed; each event is parsed, counted and
  discarded.
- **B**: cancel the I/O and close the handles; on resume run a reconcile.

## Decision drivers

- An exact "missed while paused" count, instant resume, no reconcile walk of
  large trees.
- CPU while paused: the lifecycle decision says to choose A unless a paused
  buffer costs more than 25% of a live one.
- The capture path stays at 0 allocations per event (F10).

## Considered options

1. A: armed, count and discard after the parse.
2. B: close the handles, reconcile on resume.

## Decision outcome

Chosen option A.

**Measured** (`cargo test --release -p peekbuddy-core --test capture_alloc`
with `PEEKBUDDY_PRINT_TIMES=1`, owner PC, a 64 KiB buffer of 574 extended
records through the decoder with the filter set of the allocation gate):
a live buffer costs 257.5 us, a paused one 23.6 us, so paused is **9%** of
live. The limit is 25%. Both runs allocate nothing. The live number includes
the filter evaluation, which a paused buffer skips; the parse itself is the
same.

How it works (`peekbuddy_core::pause`, `peekbuddy-engine::pause_ctl`):

- A watch is paused when the global switch or its own switch is on. Resuming
  everything leaves a watch paused on its own paused, with no row.
- Every effective change is one row: pausing stores
  `WatchStateChanged(SuspendedByUser)`, resuming stores
  `WatchStateChanged(Live)` with the missed count in `size`. Missed events are
  counted with one atomic increment and never stored.
- A gap while paused (an overflow) is discarded too; it makes the count a
  lower bound, flagged on the resume row with `GAP_FOLLOWS` ("at least N").
  The backend has already asked for a reconcile of that root.
- A per-watch pause is written to `settings.toml` (`enabled = false`) and
  logged as a row at the next start. A global pause is not persisted: a
  restart resumes. A settings file from a newer build is never written; the
  pause then lasts until the engine restarts.
- An event that races with the resume instant is counted once and shows up
  in the next row of that watch; it is never lost or counted twice.

## Consequences

- Good: exact counts, instant resume, no reconcile IO, nothing new on the
  capture path beyond one relaxed atomic load per event.
- Bad: the engine still wakes for every event while paused (9% of the live
  cost per buffer). An overflow while paused turns the count into a lower
  bound.
- #48 (lifecycle) takes `SuspendedByUser` and `Live` as written here.

## Autostart

Opt-in, off by default: `HKCU\...\Run` value `PeekBuddy` = the quoted exe
path, engine role only (no viewer). Task Manager's switch,
`...\Explorer\StartupApproved\Run`, is respected: if it says disabled the
engine reports "disabled in Task Manager" and never re-enables it at startup;
only an explicit toggle by the user does. A stale path (a moved portable exe)
is rewritten at startup when the intent is on. The registry is the truth;
settings store the intent. The flag byte of `StartupApproved` is not
documented: an even first byte is enabled, an odd one disabled with a
timestamp after it (the three entries of the owner PC all start with `03` and
a FILETIME). The "Startup impact" column is read in the owner run
(`scripts/owner/autostart-check.ps1`).

## Evidence

- `crates/peekbuddy-core/tests/capture_alloc.rs`, the 0-allocation gate and
  the paused/live timing.
- `crates/peekbuddy-win/tests/fs_serial_rdcw.rs`
  (`a_paused_watch_stores_nothing_and_counts_every_event`): 50 files created
  under a pause give 0 stored rows and a missed count equal to the records the
  backend decoded.
- auto-launch 0.6.0 for the `StartupApproved` handling (reference only).
