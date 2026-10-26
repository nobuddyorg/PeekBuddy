# Reconciliation

Overflows, lost watches and the time PeekBuddy was not running are
unavoidable with `ReadDirectoryChangesW`. The reconciler closes those gaps: it
compares the real tree with the last known state and logs what changed, so a
loss is detected and explained instead of silent. It is not polling. It runs
only after a gap or a start, never on a timer.

## Triggers

An overflow (`TRUE` with 0 bytes, `ERROR_NOTIFY_ENUM_DIR`), a ring-full gap,
a watch that came back, an engine start (also after Fast Startup) and a USN
journal gap. `peekbuddy_engine::reconciler::Reconciler` is the worker: one
background thread in `THREAD_MODE_BACKGROUND_BEGIN`, asleep on a condition
variable when idle. While a root is being walked, further triggers for it fold
into one follow-up run. `cancel_current` stops the walk at the next directory.

## The walk

`peekbuddy_core::reconcile::walk::run` lists each directory with
`GetFileInformationByHandleEx(FileIdExtdDirectoryInfo)` into one reused
64 KiB buffer, reads the same directory's children from the snapshot, sorts
both by name and joins them. It never opens file data and never descends into
a reparse point: junctions, symlinks, mount points and cloud placeholders are
entries, not trees.

| On disk and in the snapshot | Row |
| --- | --- |
| same name, same kind, id/attributes/tag differ, or for files size, write time or change time differ | `ReconciledModified` |
| same name, file became a directory or the reverse | `ReconciledDeleted`, then `ReconciledCreated` |
| only on disk | `ReconciledCreated` |
| only in the snapshot (a directory takes its known subtree) | `ReconciledDeleted` |
| only on disk and only in the snapshot with the same 128-bit file id and kind | `ReconciledRenamed` (new path) |
| a directory that cannot be listed | `AccessDenied`, once per directory per run; its snapshot stays |

A directory's own size and times change with its entries and are not
compared. ChangeTime is compared as well as the write time, otherwise
attribute-only and ACL changes would be missed. Renames across directories
pair by file id at the end of the walk; moving a directory pairs every entry
below it, so it gives one row per entry. On FAT and exFAT, where ids are not
stable, a rename degrades to delete and create.

Every row has `source = Reconcile` and the path relative to the root. A walk
is framed by `ReconcileStarted` (the gap that asked for it is in `raw_mask`,
tag + 1) and `ReconcileFinished` (`size` entries scanned, `file_id` rows
written, `src_ts` duration in 100 ns ticks; `raw_mask` 1 when cancelled).

## The first walk

A root with no snapshot gets a baseline: the walk writes the snapshot and one
`BaselineEstablished` row (`size` files, `file_id` directories), not a
Created row per existing file. A root whose own directory cannot be listed
gets no baseline.

## The snapshot

`snapshot.db` (`peekbuddy_store::snapshot`) is its own SQLite file, because
event segments rotate by day and are deleted by retention while the snapshot
must outlive both. Memory never scales with the tree: a walk holds one
directory's listing plus the map of unmatched file ids used to pair renames.

One walk is one transaction. The snapshot is committed only after the walk's
rows are durable in the event store (`StoreRows::flush` waits for the writer
and checks that no batch was lost). A cancelled walk and a walk whose rows
are not durable roll back, so the snapshot never moves past the rows that
explain it. A crash between the two commits replays a reconcile: rows can
appear twice, never be missing.

Names are exact UTF-16LE bytes and file ids 16 bytes, so lone surrogates and
ReFS ids round-trip.

## Not done yet

The snapshot is updated by the walks only, not by the live stream, so a walk
after an overflow also reports changes the live stream had already logged
before the gap (never fewer). The derived view folds such duplicates (#41).
Keeping the snapshot current from live records in the writer is a follow-up.
