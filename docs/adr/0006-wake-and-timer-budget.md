# 0006. Keep the idle engine free of periodic timers

- Status: Accepted
- Date: 2026-10-22
- Implemented or verified by: #27, #48, #59, #60

## Context and problem statement

A tray app that wakes every second costs battery and shows up in power
diagnostics. Health checks and tray updates tempt developers into timers.

## Decision drivers

- Idle CPU budget 0.05 % and at most 2 wakeups per minute (#1).

## Considered options

1. Periodic health and tray-refresh timers.
2. Event-driven health with no periodic timers while idle (F6).

## Decision outcome

Chosen option 2. While idle the engine runs zero periodic timers. Health is
event driven (completion errors, device and power notifications, session
messages). The single exception is one timer of at least 60 s, and only while
a UNC root is watched, because network shares can disappear without a
notification. Tooltip event rates are computed when the tooltip is requested.
Writer flush deadlines exist only while events are pending.

## Consequences

- Good: near-zero idle cost.
- Bad: every health signal needs an event source; the lifecycle state machine
  (#48) must model them explicitly.

## Evidence

- Wakeups are measured by the perf harness (#27), checked 2026-10-22.
