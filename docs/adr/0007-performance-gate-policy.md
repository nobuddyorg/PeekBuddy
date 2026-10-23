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

## Consequences

- Good: red means real.
- Bad: A/B doubles benchmark time in the perf job.

## Evidence

- GitHub-hosted runner specs, checked 2026-10-22: <https://docs.github.com/en/actions/using-github-hosted-runners/about-github-hosted-runners>
