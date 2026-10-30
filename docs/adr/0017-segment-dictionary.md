# 0017. Replicate the dictionary in every segment

- Status: Accepted
- Date: 2026-10-26
- Implemented or verified by: #54

## Context and problem statement

Events refer to nodes (the interned directory tree), roots and volumes by id.
With one file per local day (ADR 0004), where do those tables live: in every
segment, or once in `catalog.db`?

## Decision drivers

- Retention should be a file delete, with no garbage collection of nodes that
  no event refers to any more.
- A day should be copyable, exportable and openable on its own.
- Ingest writes one file per batch, not two.
- Size: the dictionary must not be a large part of a segment.

## Considered options

1. The dictionary (`node`, `root`, `vol` and the FTS index) in every
   segment.
2. One shared dictionary in `catalog.db`.

## Decision outcome

Chosen option 1, as the issue recommended, with a caveat about the size
threshold.

**What the numbers say.** The ingest spike (#35, report 0005, Q3) measures the
node dictionary at 11.4 B per event next to 76.3 B per event in total, so
about 15%, plus the trigram index of the names (12.6 MB for 200k names). That
is above the "about 10%" threshold of the issue. But the corpus is
deliberately hostile: 50k distinct nodes for 500k events, one new directory
entry per ten events. A real day is repetitive (the same build output, the
same project folders, the same temp files), so the number of distinct nodes
per event is much smaller and the dictionary share falls with it (the report
notes that it "shrinks per event as a segment grows"). The churn corpus of #25
replayed through the engine has not been measured yet; #27 will, and this ADR
is revisited if a real day shows more than 15%.

**What the choice buys.**

- Retention deletes a file and its `-wal` and `-shm`. No node is ever left
  behind, so there is nothing to collect and the catalog cannot grow.
- A segment is self-contained: it can be copied, exported or opened without
  `catalog.db`, and `catalog.db` is only a cache (it is rebuilt from the files
  at start).
- The writer's interner starts empty with each segment, so its memory is
  bounded by one day.
- A batch is one transaction in one file.

**How it works.** On rotation the writer opens the next file, registers every
root again (which interns the root paths into the new dictionary), carries the
spill replay position and the capture cursors over, and continues the id
sequence. Node ids are therefore per file; the global event id is not.

## Consequences

- Good: the above. Retention, export and the sealing job are all file
  operations.
- Bad: node rows and FTS entries are created again every day, and a query
  across days resolves node ids per segment (#55 iterates the segments from
  Rust, newest first, one read-only connection at a time).
- Bad: the first events of a day each intern their path again, a one-off
  cost per day that shows in the first commit of a new segment.

## Evidence

- `docs/spikes/0005-sqlite-ingest-query.md`, Q3 (size) and the FTS table,
  checked 2026-10-26.
- `crates/peekbuddy-store/tests/segments.rs`: roots, names and cursors after a
  rotation, the id sequence across files, retention as a file delete.
