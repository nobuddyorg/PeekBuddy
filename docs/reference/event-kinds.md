# Event kinds

Every row of the log has a kind: what happened, derived from the raw record
(which is kept in the row). The kinds are part of the stored format: a tag is
only ever appended, never reused.

## Changes to files

| Tag | Kind | A file or directory ... |
| --- | --- | --- |
| 0 | `Created` | appeared |
| 1 | `Modified` | had its data changed |
| 2 | `Deleted` | went away |
| 3 | `RenamedFrom` | lost its old name in a rename |
| 4 | `RenamedTo` | got its new name in a rename |
| 5 | `AttributesChanged` | had attributes, times, extended attributes or similar metadata changed |
| 6 | `SecurityChanged` | had its security descriptor changed |
| 7 | `StreamChanged` | had a named stream changed |
| 8 | `LinkChanged` | had a hard link or reparse point changed |
| 14 | `Other` | changed in a way that is not classified; the raw value is kept |

## Gaps and the walks that explain them

A gap is a stretch in which events may have been lost. It is always a visible
row, and the walk that follows (see [gaps and reconciliation](../explanation/gaps-and-reconciliation.md))
writes what it found as reconciled rows.

| Tag | Kind | Meaning |
| --- | --- | --- |
| 9 | `Overflow(reason)` | A gap, with the reason below as payload. |
| 11 | `ReconciledCreated` | The walk found a file that appeared during a gap. |
| 12 | `ReconciledModified` | The walk found a file that changed during a gap. |
| 13 | `ReconciledDeleted` | The walk found a file that vanished during a gap. |
| 20 | `ReconciledRenamed` | The walk found a file under a new name or place (same file id); the row carries the new path. |
| 21 | `ReconcileStarted` | A walk of a root started. `raw_mask` is 1 plus the tag of the gap reason that asked for it, 0 when none did. |
| 22 | `ReconcileFinished` | A walk finished: `size` is the entries scanned, `file_id` the rows it wrote, `src_ts` its duration in 100 ns ticks. `raw_mask` is 1 when it was cancelled. |
| 23 | `BaselineEstablished` | The first scan of a root wrote its baseline: `size` is the files, `file_id` the directories found. No row per file is written. |
| 24 | `AccessDenied` | A directory could not be listed; one row per directory per run. |

### Gap reasons (payload of `Overflow`)

| Tag | Reason |
| --- | --- |
| 0 | `BufferOverflow`: the change buffer of the watch overflowed. |
| 1 | `NotifyEnumDir`: Windows says to enumerate the directory (`ERROR_NOTIFY_ENUM_DIR`). |
| 2 | `JournalEntryDeleted`: USN journal entries were deleted before they were read. |
| 3 | `JournalIdChanged`: the USN journal was recreated. |
| 4 | `ParserError`: a buffer failed to parse. |
| 5 | `EngineNotRunning`: the engine was not running (the start-up gap). |
| 6 | `RingFull`: the ring to the writer was full and records were dropped. |

## Watches and the engine

| Tag | Kind | Meaning |
| --- | --- | --- |
| 10 | `WatchStateChanged(state)` | A watch changed state (below). The reason code is in the low 16 bits of `raw_mask` and the old state in bits 16 to 23 ([reasons](../explanation/watch-lifecycle.md#reasons)). |
| 15 | `EngineStarted` | The engine started. |
| 16 | `EngineStopped` | The engine stopped. |
| 25 | `EngineAbnormalExit` | The previous run ended abnormally (a crash marker was found at start); the name is the crash log. |
| 19 | `SettingsChanged` | The settings changed. |
| 28 | `Marker` | The user marked a point in time; the name is the marker's name. |

### Watch states (payload of `WatchStateChanged`)

| Tag | State |
| --- | --- |
| 0 | `Arming` |
| 1 | `Live` |
| 2 | `Overflowed` |
| 3 | `Reconciling` |
| 4 | `SuspendedByUser` |
| 5 | `SuspendedQueryRemove` (the volume is being removed) |
| 6 | `Lost` |
| 7 | `Restored` |
| 8 | `SuspendedLocked` (locked, dismounted or being checked) |

## The store

| Tag | Kind | Meaning |
| --- | --- | --- |
| 17 | `SegmentRotated` | The store rotated to a new segment. |
| 18 | `RetentionPurged` | Retention deleted old data. |
| 26 | `StoreDegraded` | The store cannot keep up or cannot write; capture goes on and the spill file takes the overflow. `raw_mask` 1: a reader holds a snapshot that keeps the write-ahead log from being reset; 2: the writer cannot commit (a full disk, a read-only file) and retries. |
| 27 | `StoreRecovered` | The store is well again; `raw_mask` is the cause that ended. |
