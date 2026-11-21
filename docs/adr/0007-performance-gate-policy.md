# 0007. Gate deterministic metrics strictly, wall-clock only by A/B

- Status: Accepted
- Date: 2026-10-22
- Implemented or verified by: #20, #23, #26, #27, #28

## Context and problem statement

Hosted CI runners are noisy. An absolute wall-clock budget as a PR gate turns
red at random and teaches people to ignore red. Deterministic metrics do not
have that problem.

## Decision drivers

- Performance is a product feature (#1).
- Gates must be trusted.

## Considered options

1. Absolute wall-clock budgets as PR gates.
2. Deterministic metrics gate strictly; wall-clock metrics gate only through
   same-runner interleaved A/B comparison; nightly trend alerts (F10, F11).

## Decision outcome

Chosen option 2.

- **Strict gates** (every PR): allocation counts (zero per event on the capture
  path after warm-up, bounded per writer batch and per viewer frame, F10),
  undetected-loss count (must be 0), overflow and gap counters, binary size,
  handle counts.
- **Wall-clock gates**: base and head are built and run in the same job on
  `windows-2025`, interleaved, at least 10 alternating samples, medians
  compared. Warn at +15 %, fail at +50 %.
- **Nightly trend**: alert at +25 % against the trailing baseline.
- **Startup definitions** (F7): measured from process creation through ETW
  milestones. Engine ready = tray shown and watchers armed. Viewer ready =
  first frame with rows.

Budget numbers live in [PERFORMANCE.md](../../PERFORMANCE.md) and
`budgets.toml` (#20); this ADR does not copy them.

## Amendment: rows no run can measure (#497, 2026-11-21)

A budget row stays only if a run measures it. Five provisional rows had no
run on any machine the project has, and each has another row that already
covers what it protects, so they are dropped rather than kept provisional
forever: the two cold-start rows (need a reboot or an elevated cache flush;
the warm and disk-read rows stay), the periodic-timer count (no registry;
`engine.idle.wakes` fails on a periodic timer, shown by the `inject timer`
self-check), the notification-buffer total (set by the settings, not by
the engine) and the 100k events/s burst rate (no generator reaches it, and
the stages measured alone stay below it). A row whose end-to-end run is out
of reach but whose stages are measured is calibrated per stage and judged end
to end only when the generator reaches its rate (`ingest.sustained`). The
reasons for each row are in PERFORMANCE.md.

The two event latency rows are loosened from 10 and 50 ms to 100 and 750 ms.
The old limits came from the commit path measured alone in spike #35 (3.2 and
6.3 ms). The rows measure end to end, from the end of a file operation to its
row in the log: on the owner PC that is 75 to 91 ms at the median and 451 to
622 ms at the 99th percentile in ten runs, the tail being commits held up by a
passive checkpoint. On the hosted runners the churn saturates the disk and a
row waits minutes; moving the checkpoint off the writer thread did not change
that (it only moved the wait), so the absolute hosted number is reported and
the rows gate by same-runner A/B, as this ADR decides for every timing row.

## Consequences

- Good: red means real.
- Bad: A/B doubles benchmark time in the perf job.

## Evidence

- GitHub-hosted runner specs, checked 2026-10-22: <https://docs.github.com/en/actions/using-github-hosted-runners/about-github-hosted-runners>
