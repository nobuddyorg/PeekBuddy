# 0018. Keep the USN backend behind a feature in v1

- Status: Accepted
- Date: 2026-10-30
- Implemented or verified by: #47

## Context and problem statement

Issue #47 is gated on spike #31: if `FSCTL_READ_UNPRIVILEGED_USN_JOURNAL`
works unelevated and returns names, the USN backend ships in v1 as the
primary source for local NTFS and ReFS roots. Otherwise the code lands behind
a disabled feature and becomes the engine of the service mode (#79). The
spike (report 0001) found the call usable for a standard user but found no
names in any record, for any token, so the second half of the condition
fails.

## Decision drivers

- No admin in v1 (O1).
- Zero undetected loss is claimed only with evidence (ADR 0002).
- The code written now must be tested without elevation and without a real
  journal, and must not cost the shipped binary anything.

## Considered options

1. Ship the USN backend in v1 and resolve every name with `OpenFileById`.
2. Land the backend behind a feature that is off in v1.
3. Do not write the backend before the service mode.

## Decision outcome

Chosen option 2.

- The OS-independent part is in `peekbuddy-core::usn` and is always built and
  tested: the record decoder, the subtree scope and the resume and gap
  decisions. It has no cost in the shipped binary beyond its size.
- The part that talks to the journal (volume handles, `FSCTL_*`, the reader
  on the shared IOCP) goes into `peekbuddy-win` behind a `usn` feature that
  is off by default and not enabled by `peekbuddy`.
- Option 1 is rejected because names are never returned: every file row
  would need a lookup, deleted files could not be named at all, and files the
  caller cannot open would stay `path_partial`. RDCW with reconciliation
  names everything it reports.
- A USN reader stays useful in v1 only as an FRN-keyed completeness check,
  which needs the same decoder and scope and no names.

## Consequences

- Good: v1 behaviour and the binary size do not change; the service mode (#79)
  starts from tested parts.
- Good: the scoping rules (move in, move out, slot reuse) are fixed by tests
  on recorded streams before any elevated code exists.
- Bad: the journal reader (handles, IOCP, the `ERROR_INVALID_USER_BUFFER`
  fallback on ReFS) and its Windows and destructive tests are not in this
  change; they follow with the feature.
- Bad: a gap row for a not-active or being-deleted journal reuses
  `JournalIdChanged` (both end with a new journal id); a separate reason would
  change the on-disk enum, which is append-only.

## Evidence

- `docs/spikes/0001-unprivileged-usn.md`: names empty in 100% of records on
  windows-2025 (26100) and windows-11-vs2026-arm (26200), all tokens.
- `crates/peekbuddy-core/src/usn/*/tests.rs`: decoder round trips with
  version 4 records interleaved, adversarial buffers, the scoping scenarios
  and the resume decisions.
