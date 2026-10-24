# 0002. Capture through source-agnostic backends with zero undetected loss

- Status: Accepted
- Date: 2026-10-22
- Implemented or verified by: #31, #32, #45, #46, #47, #48, #50

## Context and problem statement

Windows offers several ways to see file changes. ReadDirectoryChangesExW
(RDCW) works unelevated on any watched folder but its buffer can overflow
under bursts, and Windows then reports only "something changed". The USN
change journal is complete per volume but may need elevation and is
volume-wide. A tool that claims "lossless" capture over RDCW would lie.

## Decision drivers

- No admin in v1 (O1).
- The user must always know when the log is incomplete.
- Backends must be swappable without touching store or viewer.

## Considered options

1. RDCW only, overflow silently ignored.
2. USN journal only (whole volume).
3. Source-agnostic event model with backends RDCW, USN and RECONCILE; overflow
   is a gap event followed by a reconciliation scan (F2, F4, F19).

## Decision outcome

Chosen option 3.

- Every event carries a `source`: `Rdcw`, `Usn` or `Reconcile`.
- v1 ships the RDCW backend over IOCP. Information class order: Full
  (`ReadDirectoryNotifyFullInformation`), then Extended, then plain
  `ReadDirectoryChangesW`.
- Buffer sizes are decided by spike #32 (proposal: 1 MiB local, 64 KiB UNC,
  64 MiB total cap, R5).
- An overflow (`ERROR_NOTIFY_ENUM_DIR` or a zero-byte completion) writes a
  visible gap row and schedules a reconciliation scan of the affected subtree.
  The scanner emits `Reconcile` events for differences it finds.
- The USN backend is subtree-scoped (filtered by parent file ids under the
  watched roots, never a whole-volume path map) and joins v1 only if spike #31
  shows unelevated reading works; otherwise it moves to the service mode (#79).
- Every watch follows an explicit lifecycle state machine covering root removal,
  volume dismount, sleep/resume and session end (F19). A watch is never
  silently dead: losing and restoring it are logged events.
- "Lossless" is always written as **zero undetected loss**.

## Amendment: spike #31 (2026-10-24)

Report: [0001](../spikes/0001-unprivileged-usn.md). `FSCTL_READ_UNPRIVILEGED_USN_JOURNAL`
works for a real standard user on NTFS and ReFS (windows-2025 build 26100,
windows-11 build 26200), is overlapped and cancellable and has p99 latency of
a few milliseconds, but its records never carry file names. The decision rule of
the spike requires names, so the USN backend does not join v1: v1 captures with
RDCW and reconciliation; USN stays an optional backend for the service mode
(#79) and a later FRN-keyed completeness check, with names resolved from an
FRN-to-path cache and `OpenFileById`.

## Consequences

- Good: honest guarantee, provable in CI with a churn generator and a
  ground-truth ledger (#25, #50; R7).
- Good: store and viewer do not care where an event came from.
- Bad: reconciliation scans cost I/O after bursts; they are scoped to the
  overflowed subtree.

## Evidence

- ReadDirectoryChangesExW, checked 2026-10-22: <https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-readdirectorychangesexw>
- FILE_NOTIFY_FULL_INFORMATION (Windows 11 22H2+), checked 2026-10-22: <https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-file_notify_full_information>
- Change journals, checked 2026-10-22: <https://learn.microsoft.com/en-us/windows/win32/fileio/change-journals>
