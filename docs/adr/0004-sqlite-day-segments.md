# 0004. Store events in daily SQLite segments with one writer

- Status: Accepted
- Date: 2026-10-22
- Implemented or verified by: #35, #52, #53, #54, #55

## Context and problem statement

The log grows without bound, must survive crashes, must page fast at millions
of rows and must shrink by retention without long vacuums.

## Decision drivers

- 20k events/s sustained, 100k events/s bursts (#1 budgets).
- Retention must be cheap.
- One file format a user can inspect.

## Considered options

1. One SQLite database, retention by `DELETE` and `VACUUM`.
2. One SQLite file per day ("segment"), retention by deleting whole files (F5).
3. A custom append-only log format.

## Decision outcome

Chosen option 2.

- rusqlite 0.40.2 with bundled SQLite 3.53.2 (libsqlite3-sys 0.38.2).
- One writer thread; group commit with a short coalescing wait (default 5 ms)
  and upper bounds of 1,000 events or 100 ms (R1).
- One global contiguous event id sequence across all segments (R2).
- STRICT tables; 128-bit file ids as two 64-bit columns (R3).
- Keyset pagination; v1 sort order is time only; FTS5 trigram over names;
  glob and regex run as a streaming post-filter.
- Retention deletes whole segment files.

## Consequences

- Good: retention is a file delete; a corrupt day costs one day.
- Good: queries across days are a bounded `UNION ALL` over attached segments.
- Bad: queries spanning many days need attach management (#55).

## Evidence

- rusqlite 0.40.2, checked 2026-10-22: <https://crates.io/crates/rusqlite/0.40.2>
- libsqlite3-sys 0.38.2 (bundles SQLite 3.53.2), checked 2026-10-22: <https://crates.io/crates/libsqlite3-sys/0.38.2>
- SQLite WAL, checked 2026-10-22: <https://sqlite.org/wal.html>
- FTS5 trigram tokenizer, checked 2026-10-22: <https://sqlite.org/fts5.html#the_trigram_tokenizer>
