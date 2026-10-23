# 0008. Keep data in LOCALAPPDATA and never watch it

- Status: Accepted
- Date: 2026-10-22
- Implemented or verified by: #59, #62, #40

## Context and problem statement

PeekBuddy writes its log constantly. If a watched root contains the data
directory, every write produces an event, which produces a write: a feedback
loop. Search indexers reading the segments cost I/O too.

## Decision drivers

- No feedback loops, ever.
- Per-user data, not shared between accounts.

## Considered options

1. Data next to the exe.
2. `%LOCALAPPDATA%\nobuddy\PeekBuddy`, plus an opt-in portable mode (F16).

## Decision outcome

Chosen option 2.

- Data lives in `%LOCALAPPDATA%\nobuddy\PeekBuddy\`.
- The directory is created with `FILE_ATTRIBUTE_NOT_CONTENT_INDEXED`.
- The data directory is always excluded from every watch, by a rule that
  cannot be turned off, matched on the final path after resolving the root.
- **Portable mode** only when a marker file `peekbuddy.portable` (name is a
  best guess; #62 may rename it) sits next to the exe; data then lives in a `data` folder next to the exe. A portable data
  dir is excluded the same way. Without the marker, the exe's folder is never
  used.

## Consequences

- Good: no feedback loop; per-user isolation by Windows ACLs.
- Bad: two locations to document.

## Evidence

- Known folders (LocalAppData), checked 2026-10-22: <https://learn.microsoft.com/en-us/windows/win32/shell/knownfolderid>
- File attribute constants, checked 2026-10-22: <https://learn.microsoft.com/en-us/windows/win32/fileio/file-attribute-constants>
