# The USN journal backend

The change journal records every change on an NTFS or ReFS volume, including
changes made while PeekBuddy was not running. Its records carry file
reference numbers (FRNs), not paths, and the unprivileged read carries no
names at all (spike 0001). ADR 0018 keeps the reader off in v1; this page
describes the parts that exist and are tested.

## Records

`peekbuddy_core::usn::record` walks a buffer of `FSCTL_READ_USN_JOURNAL`
output: the next USN, then records led by their length and major version.
Versions 2 (64-bit FRNs) and 3 (128-bit) become a `Record`; version 4 range
records are skipped and counted. Version 2 numbers are widened, with the
sequence number kept in the top 16 bits of the low half, so every comparison
is on the whole number. A record that does not fit ends the walk with one
error after the records before it; nothing indexes the buffer.

## Scope

`usn::scope::Scope` holds the FRNs of the watched root and every directory
below it, each with its parent and name. A record is accepted when its parent
is in the set, so memory follows the watched directories, not the volume.

| Directory record | Effect |
| --- | --- |
| create under a member | added |
| rename into the tree | added, and the caller walks it (its descendants emit nothing) |
| rename inside the tree | parent and name updated |
| rename out of the tree | removed with every member below |
| delete | removed with every member below |

The path of a record is computed before the record is applied, so the old
name of a renamed directory is in its `RENAME_OLD_NAME` row. A directory
added without a name asks for one (`Action::NeedName`); until it is
resolved, paths below it are flagged `path_partial`.

## Resuming

`usn::checkpoint::plan_resume` compares the saved `(journal id, next USN)`
with `USN_JOURNAL_DATA_V2`. An intact journal continues without a gap. A
changed id, a saved USN below `LowestValidUsn` or `FirstUsn`, or one beyond
the journal's next USN gives exactly one gap row, then a reconcile and a read
from the journal's current end. A first run starts at the end: there is no
history to be missing. Errors 1181, 1179 and 1178 map to the same gaps.

## The reader

`peekbuddy_win::usn::Journal` (feature `usn`, off by default) opens the volume
on its GUID path as a directory (the `guid-dir` handle of the spike), queries
the journal with `FSCTL_QUERY_USN_JOURNAL` and reads with
`FSCTL_READ_UNPRIVILEGED_USN_JOURNAL`, version 2 and 3 records, without
waiting for new records. Both calls are overlapped on a private event and
waited for. The Windows test creates a file on the volume of the temp
directory and finds its create record by file reference number; it reads, and
never creates or deletes a journal. It does nothing on a volume without one.

For the shipped shape of the reader the journal can be put on the engine's
completion port (`Journal::associate`). `begin_wait` then starts a read with
`BytesToWaitFor = 1` that stays pending until a record exists, so an idle
volume costs no wakeups; `finish` takes its result from the dequeued
completion; `cancel` ends a pending read with a 995 completion, after which the
journal reads again. On a port the waiting `query` and `read` are refused.

## What is not here

`Journal::resolve_path` gives the volume-relative path of a file reference
number with `OpenFileById` (attribute-only, no reparse follow, no recall); a
deleted file or a reused slot is an error, never another file's name.

The synchronous fallback on `ReFS` (error 1784), the engine wiring, and the destructive tests that need a VHD. Linux NTFS drivers
write no journal records and keep the journal id, which is an undetectable
gap; it is a limit of the source, not of this code.
