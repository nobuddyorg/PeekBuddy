# 0005. SQLite ingest and query performance on Windows NTFS

- Issue: #35. Status: x64 and ARM64 data in; owner-PC run pending (needs
  hardware, see open items).
- Harness: `spikes/store-bench`, workflow `store-bench`.
- Raw data: `docs/spikes/data/0005/` (one JSON object per cell).
- Machine: GitHub-hosted `windows-2025`, 4 vCPU, Defender real-time off,
  data on `D:`; SQLite 3.53.2 bundled (rusqlite 0.40.2). Each cell is the
  median of 3 repetitions unless it is a single run.

The product schema from `peekbuddy-store` is used as is. Paths are interned
(node table); events carry random node ids, which is a deliberately hostile
write pattern for the `event(node_id, id)` index.

## Decisions for #52 and #53

1. **Group commit with the linger measured from the first pending event.**
   5 ms: p50 3.2 ms and p99 6.3 ms at 10k events/s, inside the budget (p50
   10 ms, p99 50 ms) before the capture share. Restarting the linger at every
   event (the naive reading of "commit when the ring is empty after a short
   linger") never fires under steady arrivals: p50 51 ms, p99 100 ms.
2. **Batch cap 1,000 with a 100 ms hard bound** is enough; a 10,000 cap only
   helps at a 25 ms linger (99k events/s burst), which costs 13.7 ms p50.
3. **Do not use the default autocheckpoint.** `wal_autocheckpoint=1000`
   sustains 16.8k events/s with a 124 ms worst commit, below the 20k/s
   budget. 4000 pages: 30.0k/s, 181 ms worst commit. Off: 63.1k/s, 7.5 ms
   worst commit, but the WAL reaches 1.6 GB for 500k events. Use
   `wal_autocheckpoint=0` plus a size-triggered `PASSIVE` checkpoint between
   batches; the stall of that checkpoint at a 64 MB threshold still needs a
   measurement.
4. **Burst of 100k events/s for 10 s is not met** (best 83k/s typical, 99k/s
   with the 25 ms linger and a 10k cap). The writer falls behind by about
   17k events/s, so the ring must hold roughly 170k records or spill; #52 sizes
   the ring and the spill threshold from this.
5. **Change `node_fts` to `detail='full'` in #53** (merged in #139). With
   `detail='none'` and also with `detail='column'` a phrase `MATCH` longer than
   one trigram (5 and 10 characters) is rejected with "phrase queries are not
   supported"; only `full` answers it. `LIKE` is a virtual-table scan that is
   slower than scanning `node`. The first version of this report said `column`
   works; that was a harness bug (errors were counted as rows), corrected by
   the rerun below.
6. **Keep 4096-byte pages and interned paths.** 8192 saves nothing; inline
   paths cost 115 B/event against 76 B.
7. **`LIBSQLITE3_FLAGS` speed set: no measurable gain** in the commit cells
   (burst 83.7k vs 83.9k events/s at batch 1,000 / 5 ms, same latencies).
   Keep the stock build unless ARM64 says otherwise.
8. **Cache 2 MB** (see memory): +4.8 MB private bytes after a 1M-event burst,
   against +22 MB at 16 MB; `PRAGMA shrink_memory` returns to about 3.3 MB.

## Q1 commit policy (10k events/s paced, 50k events; burst 300k events)

| Batch cap | Linger | Burst events/s | p50 | p99 | Commits/s |
| --- | --- | --- | --- | --- | --- |
| 100 | 5 ms | 61.1k | 3.2 ms | 6.2 ms | 180 |
| 100 | 10 ms | 61.3k | 5.6 ms | 10.5 ms | 99 |
| 100 | 25 ms | 61.8k | 5.6 ms | 10.5 ms | 99 |
| 1,000 | 5 ms | 77.0k | 3.4 ms | 7.0 ms | 175 |
| 1,000 | 10 ms | 83.1k | 6.1 ms | 11.8 ms | 92 |
| 1,000 | 25 ms | 63.4k | 14.1 ms | 27.4 ms | 38 |
| 10,000 | 5 ms | 82.9k | 3.2 ms | 6.3 ms | 178 |
| 10,000 | 10 ms | 84.5k | 5.9 ms | 11.5 ms | 93 |
| 10,000 | 25 ms | 98.9k | 13.7 ms | 26.8 ms | 38 |
| 1,000 | 5 ms from the last event | 84.8k | 51.0 ms | 100.0 ms | 9 |

The batch cap does not matter at 10k events/s (a 5 ms batch holds about 50
events); it matters for the burst rate.

## Q2 checkpoint (500k events, batch 1,000, 5 ms)

| Policy | Events/s | Worst commit | Peak WAL |
| --- | --- | --- | --- |
| `wal_autocheckpoint=1000` | 16.8k | 124 ms | 6.2 MB |
| `wal_autocheckpoint=4000` | 30.0k | 181 ms | 18.8 MB |
| off, one `PASSIVE` at the end | 63.1k | 7.5 ms | 1,629 MB |

The 3.2 KB of WAL per event comes from the random `node_id` index: nearly
every event dirties a different leaf page and the WAL stores whole pages.
Real churn has more locality; the corpus of #25 should be replayed in #52 to
size the threshold.

## Q3 size (500k events, 50k nodes)

| Variant | B/event | Table | Indexes | Node dictionary |
| --- | --- | --- | --- | --- |
| interned, both indexes (4096) | 76.3 | 44.6 | 31.8 | 11.4 |
| interned, no index | 44.6 | 44.6 | 0 | 11.4 |
| interned, `event(node_id, id)` only | 61.3 | 44.6 | 16.7 | 11.4 |
| interned, `event(root_id, id)` only | 59.6 | 44.6 | 15.0 | 11.4 |
| inline path, `root` index only | 115.1 | 100.1 | 15.0 | 0 |
| interned, both indexes (8192) | 76.2 | 44.4 | 31.7 | 11.2 |

The node dictionary is per distinct node, not per event, so it shrinks per
event as a segment grows. Each index costs about 15 to 17 B/event.

## Q4 queries at 10M rows (200-row keyset page, newest first, warm cache)

| Shape | p50 | p99 |
| --- | --- | --- |
| newest, no filter | 80 µs | 138 µs |
| root filter (index) | 176 µs | 357 µs |
| kind filter, 10% of rows | 200 µs | 353 µs |
| kind filter, 0.1% of rows (no index) | 13.0 ms | 14.9 ms |
| subtree: 50 node ids (index) | 906 µs | 3.5 ms |

A rare kind has no index and scans: rare kinds (overflow, watch state) need
either an index or a bound from `stats` in #55. The 10M-row fill ran at 31k
events/s on average (100k/s at 2M rows): once the `node_id` index outgrows the
16 MB cache each insert touches a cold page.

### FTS5 trigram over 200k node names

| Index | Size | `LIKE` 3/5/10 chars | `MATCH "needle"` 3/5/10 chars |
| --- | --- | --- | --- |
| `detail='none'` (current) | 12.6 MB (1.7x the names) | 39 ms, a full scan | rejected: phrase queries need detail |
| `detail='column'` | 27.6 MB (3.6x) | 39 ms, a full scan | 3 chars only; 5 and 10 chars rejected (errors) |
| `detail='full'` | 27.6 MB (3.6x) | 39 ms, a full scan | 93 µs / 193 µs / 436 µs |
| plain scan of `node` | 0 | 21 ms | not applicable |

`LIKE` never uses the trigram index in this build, with any `detail`, so the
answer is `MATCH` on a quoted phrase with `detail='full'`. Below 3
characters a scan of `node` (21 ms at 200k nodes) is the fallback. The sizes
of `column` and `full` are within 4 KB of each other (the corpus is one column
of short names, so there is nothing for `full` to add).

## Q5 and Q10 open cost and sealing

| Cell | Result |
| --- | --- |
| open a 10M-row segment and fetch the first page, warm cache | 0.6 ms (p50), 0.8 ms (max) |
| open, cold cache | not measured (needs privileges to flush the cache) |
| `VACUUM INTO` of a 10M-row segment | 5.4 s, 885 MB to 854 MB (3.5% smaller) |
| zstd 0.14.3 ratio | not measured |

## Q6 readers (writer batches of 1,000 events, 400k events)

| Case | Writer events/s | Reader p50 / p99 | Peak WAL |
| --- | --- | --- | --- |
| no reader | 14.5k | n/a | 11 MB |
| read-only connection, `mmap_size=0` | 20.9k | 85 / 184 µs | 61 MB |
| read-only connection, `mmap_size=256 MB` | 19.5k | 76 / 146 µs | 59 MB |
| reader holds a snapshot | 41.1k | 100 / 190 µs | 1,372 MB |

The writer rates differ because autocheckpoint stalls land differently run to
run, not because a reader helps; the reader cost to the writer is below the
noise. A reader that holds a snapshot pins the WAL: it grew to 1.4 GB, so #55
must keep read transactions short and #52 must not rely on checkpoints while a
viewer is open.

## Q7 Defender, Q8 flags, Q9 memory

- The `windows-2025` image has Defender off, so excluding the data directory
  changes nothing there (burst and latency within 2%). Defender-on numbers
  come from the ARM64 runner and the owner PC.
- Flags A/B: no measurable difference, see decision 7.
- Memory after 1M events (private bytes, after minus before):

| `cache_size` | After burst | After `shrink_memory` |
| --- | --- | --- |
| 16 MB | +22.0 MB | +2.9 MB |
| 4 MB | +7.5 MB | +2.4 MB |
| 2 MB | +4.8 MB | +2.6 MB |

- Rust-side allocations per warm 1,000-event batch through `prepare_cached`:
  1 allocation of 280 bytes. K for the F10 writer gate starts at 1; SQLite's
  own heap is outside the counting allocator.

## Rerun after the corrections (24 Oct 2026): x64 and ARM64

Raw data: `data/0005/windows-2025-rerun.jsonl` and
`data/0005/windows-11-vs2026-arm.jsonl` (workflow run 40133757148, PR #144).
The ARM64 query cells use 2M rows (a 10M-row fill does not fit the job
time there); x64 `stock` uses 10M rows.

| Cell | x64 `windows-2025` | ARM64 `windows-11-vs2026-arm` |
| --- | --- | --- |
| burst, batch 1,000 / 5 ms | 127.9k events/s | 59.3k events/s |
| burst, batch 10,000 / 25 ms | 187.7k events/s | 116.3k events/s |
| paced 10k/s p50 / p99, batch 1,000 / 5 ms | 3.3 ms / 95 ms | 3.3 ms / 15.3 ms |
| `wal_autocheckpoint=1000` | 35.4k events/s, worst commit 66 ms | **1.8k events/s, worst commit 1,284 ms** |
| `wal_autocheckpoint=4000` | 44.7k events/s, 124 ms | **4.1k events/s, 1,978 ms** |
| checkpoint off, `PASSIVE` at the end | 62.7k events/s, 6.4 ms | 36.4k events/s, 9.5 ms |
| fill, 2M rows (x64 10M rows: 55k events/s) | 163k events/s | 20.9k events/s |
| newest page p50 / p99 | 40 / 67 µs | 51 / 78 µs |
| root filter p50 | 117 µs | 150 µs |
| rare kind (no index) p50 | 7.9 ms | 12.4 ms |
| `MATCH` `detail='full'` 3 / 5 / 10 chars p50 | 93 / 193 / 436 µs | 102 / 201 / 453 µs |
| open a segment, warm | 0.37 ms | 0.77 ms |
| `VACUUM INTO` (x64 10M rows; ARM 2M rows) | 4.0 s | 2.3 s |

Findings that change decisions:

- **ARM64 checkpoint stalls are the big difference**: with an autocheckpoint
  a commit blocks for 1.3 to 2.0 s and throughput falls to 2 to 4k events/s.
  Excluding the data directory from Defender does not change it
  (1,323 ms and 1,815 events/s excluded), so the cost is the runner's disk
  sync, not real-time scanning. This confirms decision 3 harder: no
  autocheckpoint, a size-triggered `PASSIVE` checkpoint, and #52 must measure
  that checkpoint's stall on ARM before the threshold is fixed.
- **The x64 numbers moved between the two runs on the same image**: the first
  run measured a burst of 77k to 99k events/s and a paced p99 of 6 to 7 ms,
  this run 128k to 188k events/s and a paced p99 of 95 ms. The runner's
  data disk (`D:`) varies from run to run, so single-run burst and p99
  figures are not budget material. The p50 (3.3 ms) is stable. Decision 4
  ("100k/s burst not met") is therefore *inconclusive on x64*: met in one run,
  not in the other; ARM64 reaches 60k to 116k. #52 keeps the ring sized for
  the worse case, and the budget row stays `nightly-trend` rather than
  gating. A 50 ms p99 is only safe with the commit off the capture thread
  (it is, decision 1) and a ring that rides out a 100 ms stall.
- Memory, size and decision 6 are identical on both architectures
  (bytes per event are equal to the digit).
- The speed flags are again no gain on either architecture (burst and query
  cells within noise), decision 7 stands.
- Defender: the x64 image has it off; on ARM64 it is on and makes no
  difference to the stalls, so the owner-PC run is for the NVMe numbers
  rather than for Defender.

## Open items

- The owner run on the Win11 PC (NVMe, Defender on and excluded), HDD, and a
  cold-cache open: `scripts/owner/store-bench.ps1` (follow-up).
- Stall of a size-triggered `PASSIVE` checkpoint, replay of the #25 corpus for
  WAL growth, zstd ratio, `MEMSTATUS` 0 versus 1.
- `budgets.toml` and ADR 0004 are not updated yet; the numbers above are
  from one runner class and one repetition set.
