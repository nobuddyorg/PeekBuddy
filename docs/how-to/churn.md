# Churn generator and comparator

`peekbuddy-churn` (#25) produces file-system churn whose ground truth is
known, and compares that truth with what PeekBuddy logged. It is dev-only
and never shipped. The correctness suite (#50), the perf harness (#27) and
the RDCW/SQLite spikes (#32, #35) all use it as their oracle.

The OS-independent parts (plans, profiles, ledger, comparator and the
executor loop) are in this crate and run on Linux and under Miri. The
Windows file system and clock (`exec::windows`) drive the real run.

## Running it

```powershell
cargo xtask churn --profile edge-mix --seed 1
cargo xtask churn --profile burst-100k --threads 4
cargo xtask churn --profile latency-10k --duration 5 --root D:\churn --keep
```

| Option | Default | Meaning |
| --- | --- | --- |
| `--profile <name>` | `edge-mix` | named workload (below) |
| `--seed <n>` | 1 | seed of every thread's stream |
| `--threads <n>` | 1 | generator threads, each in its own `t<n>` |
| `--rate <ev/s>`, `--duration <s>` | the profile's | make the run steady at this rate |
| `--burst <events>`, `--within <s>` | the profile's | make the run a burst: unpaced, target `events / within` per second |
| `--shape <depth,width,files>` | `2,3,4` | initial tree per thread |
| `--mix <profile>` | the profile's | take the operation mix of another profile |
| `--root <dir>` | fresh `%TEMP%\pbchurn-<pid>` | churn root; must be empty |
| `--ledger <path>` | `target/churn/<profile>-seed<seed>.jsonl` | where the ledger goes |
| `--keep` | off | keep a root the command created |

The command prints (and appends to the GitHub job summary) the operations,
the achieved raw event rate against the target, the verdict and per-class
ok/failed counts. It fails when a class of the mix never succeeded; a
"not exercised" verdict is reported, not failed. A root the command created
is deleted afterwards; a `--root` you pass is never deleted. Delete a watched
root only after the watcher is stopped and joined.

CI runs `edge-mix` (seed 1, 2 threads) in `test (windows x64)` and the arm64
job and uploads the ledger (`churn-ledger-x64`, `churn-ledger-arm64`). A
50,000-event burst with 4 threads reports what the runner can generate
(report only). First numbers (debug xtask, `%TEMP%` on the system disk,
4 threads, 0 failed operations):

| Burst | windows-2025 | windows-11-vs2026-arm |
| --- | --- | --- |
| 50,000 raw events | 8,178 ev/s | 8,925 ev/s |
| 200,000 raw events | 2,255 ev/s | 3,127 ev/s |

The rate falls as each thread's tree grows (every create adds a file to a
few directories), so compare runs of the same length only. A hosted runner
is far from the 100k ev/s of `burst-100k` with this executor, so
`burst-100k` runs there are "not exercised"; #32 decides whether that needs
a faster executor or another machine.

How a run works:

- Every thread plans its whole stream before the clock starts, runs its
  setup (the initial tree, unpaced and not part of the rate) and waits at a
  barrier, so all threads start together.
- A steady run is paced per thread by a token bucket (rate / threads, plus
  2 % headroom so jitter alone never makes it "not exercised"): it
  sleeps only when at least 1 ms ahead and drops backlog beyond 10 ms, so
  a stall is not followed by an unbounded burst. Sleeps use a
  `CREATE_WAITABLE_TIMER_HIGH_RESOLUTION` timer; `timeBeginPeriod` is never
  called. A burst runs unpaced.
- Timings go into vectors sized up front; ledger records are built after
  the run, sorted by end time into `seq`.
- Operations use `std::fs` with exact UTF-16 paths (`OsString::from_wide`),
  so lone surrogates survive and long paths go through `\\?\`.
  Junctions (`FSCTL_SET_REPARSE_POINT`) and the archive-attribute toggle
  come from `peekbuddy_win::fs`.
- A truncate sets the size to 0, which updates the last-write time even on
  an empty file; a set-size grows the file by 4 KiB. So every operation
  changes something and produces its record. A timestamp change sets a new last-write time each time.
- The raw event count of an operation is the number of records it expects
  (`Op::expected`); the achieved rate counts successful paced operations
  over the span from the first paced start to the last paced end.

## Profiles

| Profile | Mix | Target | Names |
| --- | --- | --- | --- |
| `burst-100k` | perf mix | 1,000,000 raw events within 10 s (100k ev/s) | ASCII |
| `burst-50k` | perf mix | 50,000 raw events as fast as possible (the forced-overflow test of #50) | ASCII |
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

The ledger header's machine facts come from `cmd /c ver` (OS build with the
update revision), `available_parallelism`, `GetVolumeInformationW` (file
system), `Get-MpComputerStatus` (Defender real-time) and an 8.3 probe file
created next to the root (`GetShortPathNameW`). A fact that cannot be read
is `null` or `unknown`. The runner label is `PEEKBUDDY_RUNNER_LABEL`, else
`ImageOS`, else the computer name.

Still to come with the export (#56) and the probe (#24): mapping the export
or segment DBs to rows and gaps, reconstructing the final tree from the log
and comparing it with a walk of the real tree, and latency samples (ledger
`t_end` to batch commit time).
