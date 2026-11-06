# 0020. Leave sealed days as they are and let a busy segment wait

- Status: Accepted
- Date: 2026-11-06
- Implemented or verified by: #54, #218

## Context and problem statement

ADR 0004 keeps one SQLite file per local day, and retention deletes whole
files (ADR 0017). Three questions were left open when the segments landed
(#54), and the viewer exists now to answer them:

1. Does retention need to tell the viewer before it deletes a segment
   (the `Retire` handshake, which the engine answers with `NoViewer` today)?
2. Should a sealed day be compacted (`VACUUM INTO`, and maybe compression)?
3. What does a day cost, and are the timers idle?

## Decision drivers

- F6: no timers and no extra work while idle.
- Zero undetected loss and no lost rows beat disk space.
- Retention must not be blocked by a viewer, and a viewer must not crash
  because a file went away.

## Considered options

1. A pipe message from the engine to every connected viewer ("close
   `<segment>`"), acknowledged before the delete.
2. No announcement: the viewer holds no connection to a segment between
   requests, and a delete that finds the file busy is deferred and retried.
3. Compact sealed days with `VACUUM INTO`, with or without compression.
4. Leave sealed days as they are.

## Decision outcome

Options 2 and 4.

**Retirement.** The viewer opens a segment for the length of one page and
closes it again; what it keeps between pages is a cache of path components in
memory (`Reader`), not a file handle. The one long reader is an export, and a
file it still has open makes the delete fail with a sharing violation. The
purge already handles that: `retention::purge` tries three times, then marks
the segment `purge_pending` in the catalog and tries again at the next
rotation, and a reader that finds a segment gone gets it in `Page::retired`
instead of an error. So the handshake would only add a message, a protocol
change and a wait on the engine side for nothing. `Retire` stays a trait: the
day the viewer keeps connections open (a prepared live cursor, say), an
implementation over the pipe plugs in without touching the purge.

**Compaction.** Measured on a sealed day of 10,000,000 rows (832 MB, 1.01M
nodes, owner PC, 2026-11-06):

| | result |
| --- | --- |
| `VACUUM INTO` | 34 s, 832 MB to 794 MB (4.5 % smaller; the freelist was empty) |
| zlib level 3 on 64 MB samples of the file | 25 to 39 % of the size, 40 to 59 MB/s |

A sealed day is written append-only in id order, so it has no free pages to
give back, and `VACUUM INTO` reorganises nothing that matters for 4.5 %. The
compression would bring the file to a third, but SQLite cannot open it:
reading an old day would need a decompression to a temporary file (seconds
and as many bytes as the day) or a compressed VFS, which is a dependency and a
new failure mode in the read path of the log. The disk is not the scarce
resource of this product (the retention caps are), and a day this size is far
beyond real use (10M events in a day). Not in v1.

**Timers and the dictionary share.** Nothing in the segment code owns a
timer: rotation is checked when a batch arrives, retention runs after a
rotation, and the idle wake budget is tracked by the `idle-60` job of the
nightly (F6). The dictionary share of ADR 0017 is not re-measured here: the
corpus of the 10M-row file is synthetic (1.01M nodes for 10M events), like the
spike's, and a real day to measure it on will exist once the owner has run the
release for a while.

## Consequences

- Good: no new message in the protocol, nothing for retention to wait on, no
  compressed format to support.
- Bad: a very large day stays large until retention removes it.
- Revisit when a real day of the owner's data is available (dictionary share,
  size per event) or when the viewer starts keeping connections open.

## Evidence

- `crates/peekbuddy-store/src/retention.rs` (`Retire`, `purge`, `Outcome::Deferred`).
- The 10M-row file was built with the query bench (`benches/benches/query.rs`)
  and measured with Python's `sqlite3` (`VACUUM INTO`) and `zlib`, 2026-11-06.
