# Churn generator and comparator

`peekbuddy-churn` (#25) produces file-system churn whose ground truth is
known, and compares that truth with what PeekBuddy logged. It is dev-only
and never shipped. The correctness suite (#50), the perf harness (#27) and
the RDCW/SQLite spikes (#32, #35) all use it as their oracle.

The OS-independent parts (plans, profiles, ledger, comparator) are in this
crate and run on Linux and under Miri. The Windows executor that performs
the plan and `cargo xtask churn` follow in a later PR.

## Profiles

| Profile | Mix | Target | Names |
| --- | --- | --- | --- |
| `burst-100k` | perf mix | 1,000,000 raw events within 10 s (100k ev/s) | ASCII |
| `sustained-20k` | perf mix | 20,000 raw ev/s for 60 s | ASCII |
| `latency-10k` | perf mix | 10,000 raw ev/s for 30 s | ASCII |
| `edge-mix` | every operation class, equal weight | 200 raw ev/s for 10 s | exotic |

The perf mix is create, write, append, rename, delete, directory create and
timestamp change, weighted like editor and build-tool churn. `edge-mix`
covers all 17 classes: create file, write, append, truncate, set size,
rename, rename chain (a->b->c), move across directories, delete, directory
create, recursive delete, attribute change, timestamp change, hard link
create and delete, case-only rename and junction create (the junction is
logged as the link object and never followed, F18). Exotic names mix CJK,
combining marks, emoji, lone surrogates (F17) and 120-character names, so
paths pass MAX_PATH.

`perf/budgets.toml` names its workloads after these profiles.

A run reports its achieved raw event rate. Below the profile's target the
verdict is "not exercised": the run says nothing about the budget and must
not count as passed.

## Plans

Each thread works in its own directory `t<n>` and keeps a model of its
tree, so every planned operation is valid when run in order. Classes that
cannot run on the current tree (nothing to rename yet) become file
creates. Streams come from an in-tree `SplitMix64`/`xoshiro256**` PRNG, so they
never change when a dependency updates: the same seed and thread give the
same operations. The ledger, not the seed, is the ground truth for how
threads interleave.

## Ledger format

JSONL, written after the run and outside the watched root. The first line
is the header:

```json
{"version":1,"profile":"edge-mix","seed":1,"threads":1,"os_build":"10.0.26100.6584","runner":"windows-2025","cpus":4,"file_system":"NTFS","defender_realtime":false,"short_names":false}
```

Every other line is one operation:

```json
{"seq":7,"thread":0,"op":"rename","paths":["t0\\d3\\f9","t0\\d3\\r12"],"t_start":134055678901234567,"t_end":134055678901240000,"result":0,"expected":[{"path":"t0\\d3\\f9","kind":"renamed_old"},{"path":"t0\\d3\\r12","kind":"renamed_new"}]}
```

- Paths are relative to the churn root and exact UTF-16. A path that is not
  valid Unicode, or that starts with `utf16:`, is written as `utf16:` and
  four hex digits per unit.
- Times are UTC FILETIME ticks from `GetSystemTimePreciseAsFileTime`. ARM
  runners use Pacific local time and x64 runners UTC, so nothing uses local
  time.
- `result` is the Win32 error code. A failed operation expects nothing.
- `expected` lists the raw records the watcher must log: one `kind` per
  `FILE_ACTION_*` value (`added`, `removed`, `modified`, `renamed_old`,
  `renamed_new`).

## Comparator semantics

`peekbuddy_churn::compare::compare(records, rows, gaps)`:

- An expected record is **matched** by a logged row with the same path and
  kind whose time is at or after the operation's start. Paths compare
  exactly, including case. One row may match several operations: the file
  system can merge repeated identical notifications, so same-kind repeats
  count as "at least one". One logical save is 2-4 raw records; extra rows
  are fine.
- An operation with an unmatched record is **covered by a gap** when one gap
  (overflow, F2) holds every unmatched path in its subtree and its
  reconciliation finished after the operation ended.
- Anything else is an **undetected loss**: the count must be 0
  (`ingest.undetected_loss`).
- Rows no operation expects are counted as unexpected and reported, not
  failed. A ledger record with a malformed path fails the comparison.

Property tests check the comparator against a brute-force oracle with random
drops, duplicates, unrelated rows and gaps. They also check that removing the
only witness of an expected record, with no gap, is always reported, and that
a covering gap makes the run pass. cargo-mutants examines the crate: an
oracle that cannot fail is worthless.

Still to come with the executor and the export (#56): mapping the export or
segment DBs to rows and gaps, reconstructing the final tree from the log and
comparing it with a walk of the real tree, latency samples (ledger `t_end` to
batch commit time from #24's probe), and the ledger header's machine facts.
