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

- rusqlite 0.40.2 with bundled SQLite 3.53.2 (libsqlite3-sys 0.38.2); since
  2026-11-22 rusqlite 0.43.0 with SQLite 3.54.1 (libsqlite3-sys 0.40.0).
- One writer thread; group commit with a short coalescing wait (default 5 ms)
  and upper bounds of 1,000 events or 100 ms (R1). The wait is measured from
  the first pending event of a batch, not restarted at every event: restarting
  never fires under steady arrivals (p50 51 ms against 3.2 ms at 10k events/s).
- `wal_autocheckpoint=0` with a size-triggered `PASSIVE` checkpoint between
  batches. The default 1,000-page autocheckpoint caps the writer at 16.8k
  events/s with 124 ms stalls (budget: 20k/s). Readers keep their read
  transactions short, because a pinned snapshot lets the WAL grow without
  bound (1.4 GB in the spike).
- 4,096-byte pages, interned paths (76 B per event with both indexes; inline
  paths cost 115 B), `cache_size` 2 MB (+4.8 MB private bytes after a 1M-event
  burst), stock bundled SQLite build (the `LIBSQLITE3_FLAGS` speed set changed
  nothing measurable).
- The writer alone commits 77k to 83k events/s in a burst, below the 100k/s
  burst budget: the capture ring holds about 170k records or spills (#52;
  65,536 since #497, see the amendment).
- `node_fts` uses `detail='full'`; substring search is `MATCH` on a quoted
  phrase. `detail='none'` and `'column'` reject a multi-trigram phrase, and
  `LIKE` never uses the trigram index.
- One global contiguous event id sequence across all segments (R2).
- STRICT tables; 128-bit file ids as two 64-bit columns (R3).
- Keyset pagination; v1 sort order is time only; FTS5 trigram over names;
  glob and regex run as a streaming post-filter.
- Retention deletes whole segment files.

## Consequences

- Good: retention is a file delete; a corrupt day costs one day.
- Good: queries across days are a bounded `UNION ALL` over attached segments.
- Bad: queries spanning many days need attach management (#55).

## Amendment: a ring that fits the memory budget (#497, 2026-11-21)

The ring holds 256 batches (65,536 events) instead of about 170k events. A
full batch holds about 57 KB, so the old 700 batches were 40 MB on their own,
the whole burst budget of the engine (`engine.private_bytes.burst`, F8). On
the hosted runners, whose disk the churn saturates (the generator reaches
about 2k of its 10k operations/s), the ring filled and the engine peaked at
54 to 56 MB in latency-10k. With 256 batches it peaked at 27.8 to 28.6 MB with
the ring full, and nothing was lost. A deficit longer than the ring goes to
the spill file, in order, as before. On the owner PC the ring never holds more
than about 20 batches in the same run, so nothing changes there.

## Evidence

- rusqlite 0.40.2, checked 2026-10-22: <https://crates.io/crates/rusqlite/0.40.2>
- libsqlite3-sys 0.38.2 (bundles SQLite 3.53.2), checked 2026-10-22: <https://crates.io/crates/libsqlite3-sys/0.38.2>
- SQLite WAL, checked 2026-10-22: <https://sqlite.org/wal.html>
- FTS5 trigram tokenizer, checked 2026-10-22: <https://sqlite.org/fts5.html#the_trigram_tokenizer>
- Spike #35 report and raw data: `docs/spikes/0005-sqlite-ingest-query.md`, `docs/spikes/data/0005/`.
