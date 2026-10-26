# Architecture decision records

Each ADR follows the [template](template.md) (MADR). Accepted ADRs are
immutable: a change is a new ADR that supersedes the old one.

Status legend: **Proposed** (waiting for evidence), **Accepted**,
**Superseded** (by the ADR named in its status line).

| ADR | Title | Status | Covers |
| --- | --- | --- | --- |
| [0001](0001-two-process-single-exe.md) | One exe, engine and on-demand viewer roles | Accepted | F1 |
| [0002](0002-source-agnostic-capture-and-zero-undetected-loss.md) | Source-agnostic capture, zero undetected loss | Accepted | F2, F4, F19, R5, R7 |
| [0003](0003-raw-is-truth.md) | Raw is truth | Accepted | F3 |
| [0004](0004-sqlite-day-segments.md) | SQLite day segments, one writer | Accepted | F5, R1, R2, R3 |
| [0005](0005-viewer-renderer.md) | Viewer renderer by measurement | Proposed | F9 |
| [0006](0006-wake-and-timer-budget.md) | No periodic timers while idle | Accepted | F6 |
| [0007](0007-performance-gate-policy.md) | Performance gate policy | Accepted | F7, F10, F11 |
| [0008](0008-data-dir-and-self-exclusion.md) | Data dir and self-exclusion | Accepted | F16 |
| [0009](0009-name-fidelity.md) | Name fidelity | Accepted | F17 |
| [0010](0010-dependency-policy.md) | Dependency policy | Accepted | F20 |
| [0011](0011-repository-visibility.md) | Public repository, MIT | Accepted | F14, O2 |
| [0012](0012-code-signing.md) | Unsigned v1, free signing later | Accepted | F15, O3, O6, B4 |
| [0013](0013-supported-windows-and-cpu-baseline.md) | Supported Windows and CPU baseline | Accepted | F13, B1 |
| [0014](0014-contribution-policy.md) | Contribution policy | Accepted | B2, B3 |
| [0015](0015-raw-win32-tray.md) | Tray on raw Win32 | Accepted | #60 |

Decisions owned elsewhere: F8 memory budgets and R6 SQLite heap limit in
`PERFORMANCE.md` (#20, #52); F12 runners in #11 and #34; F18 links and cloud
placeholders in #49; R4 system event kinds in #38.
