# Capture with ReadDirectoryChangesExW

`peekbuddy_win::rdcw` is the default capture source: unelevated, low latency,
and it works on any folder. Read naively, `ReadDirectoryChangesW` discards
whole batches without a word. This backend turns every such loss into a
**visible gap row followed by a reconcile request**, which is what "zero
undetected loss" means here.

The work is split so most of it runs on Linux:

- `peekbuddy_core::capture` decodes a finished buffer: parse
  ([`notify`](../../crates/peekbuddy-core/src/notify/mod.rs)), map to the
  event model, drop the data directory's own churn, hand each event to an
  `EventSink`. It allocates nothing per event.
- `peekbuddy_win::rdcw` owns the I/O: one thread, one completion port, one
  armed buffer per root.

## The thread

One thread serves all roots. It blocks in `GetQueuedCompletionStatusEx` with
no timeout, so an idle engine has no wakeups from it. Adding or removing a
root and stopping arrive as posted completions.

A completion is handled in this order:

1. **Re-arm first**, with a spare buffer of the same size. The completed
   buffer is decoded afterwards and becomes the new spare, so `N` roots need
   `N + 1` buffers per size class.
2. Decode: parse, map, exclude, offer to the sink (filters, ring).
3. Overflow, a full ring or a malformed buffer owes a gap row and asks the
   `LifecycleSink` for a reconcile of that root. The root stays armed.

Every record of a completion shares one clock reading (RDCW records carry no
event time) and keeps its position in the batch.

## Loss is never silent

| Situation | What is recorded |
| --- | --- |
| `TRUE` with 0 bytes | `Overflow(BufferOverflow)` row, reconcile |
| `ERROR_NOTIFY_ENUM_DIR` | `Overflow(NotifyEnumDir)` row, reconcile |
| The sink refuses an event (ring full) | the record is counted, `Overflow(RingFull)` row, reconcile |
| A buffer fails to parse | records before the fault are kept, `Overflow(ParserError)` row, reconcile |
| Any other error (root deleted, volume gone) | `WatchStateChanged(Lost)` through the lifecycle sink |

A gap row that cannot be stored right away (the ring is still full) is
*owed*: the decoder pushes it before anything newer at the next completion,
so a gap row always precedes later events. The counters satisfy
`gap_rows == gaps_owed_total` once the ring has room, and the number of
overflows equals the number of overflow gap rows.

The back-pressure rule: the IOCP thread never blocks on the ring. It drops,
counts, and schedules a reconcile.

## Info classes

A root starts with `FILE_NOTIFY_FULL_INFORMATION`, falls back to the extended
class, then to plain `ReadDirectoryChangesW`, when the file system refuses a
class. The class in use is reported with the `Live` state. A buffer is fixed
by the kernel at the first call on a handle, so a new size means a new
watch. On a network path the buffer is capped at 64 KiB.

## The data directory

Events under the data or export directory are dropped before the ring and
counted (`Counters::excluded`). It is the one ingest-time drop. The caller
passes canonical paths (`GetFinalPathNameByHandleW`); the backend makes them
relative to each root's own canonical path and compares ignoring case.

## Stopping

`CancelIoEx`, then wait for the completion. Records that arrive on the way
are still delivered. Only then is the handle closed, so a directory with a
stopped watch can be renamed or deleted at once.
