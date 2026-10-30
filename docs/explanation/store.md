# How the store writes

The engine records every captured change as one SQLite row (ADR 0004, F3).
This page explains the path from capture to disk: batching, durability, the
spill file and what is deliberately not done. The code is in
`peekbuddy-store` (`batch`, `policy`, `writer`, `pipeline`, `spill`) and
`peekbuddy-engine::store_win`.

## The path

```text
capture thread --Batch--> ring --> writer thread --> segment.db (WAL)
                  |
                  +-- ring full / spill active --> overflow --> spill thread --> spill/spill.bin
                                                                   (replayed in order by the writer)
```

- Capture fills a **batch** (up to 256 events and 32 Ki name units, storage
  reserved once) and pushes it. Pushing takes a lock for a pointer move and
  never touches the disk. A used batch comes back empty from the writer, so
  the capture path allocates nothing per event (F10).
- One thread owns the connection. It takes the next batch, then keeps taking
  whatever else is queued until the commit policy says stop, and writes the
  group in **one transaction**: events, `stats` upserts, the capture cursors
  and (for replayed records) the spill position.
- Names are paths relative to the watch root. The writer interns every
  component (`node` table, in-memory map) and stores the node id on the row.

## Group commit

A group is committed when it holds 1,000 events, or 5 ms after its **first**
event, and never later than 100 ms (`policy::Policy`). Restarting the 5 ms at
every event would never fire under steady arrivals: the spike measured p50
51 ms for that reading and 3.2 ms for the first-event reading (report 0005).
The wait exists only while events are pending; an idle writer sleeps without a
timeout. After a commit the writer arms one 1 s wait to run a single `PASSIVE`
checkpoint when the burst is over; a WAL above 64 MB is checkpointed between
batches.

## What "durable" means

Pragmas: `journal_mode=WAL`, `synchronous=NORMAL`. A commit is in the WAL, so
it survives a **process crash**; the last transactions can be lost on **power
loss**. That is acceptable because the capture cursor is written in the same
transaction as the events it covers: after a crash the cursor never points past
a committed row, and the USN replay and the reconcile scan (#46, #47) refill
the gap. A debug build asserts this after every commit.

On quit the engine flushes the queue (at most 5 s, F19), stops the writer,
runs `wal_checkpoint(TRUNCATE)` and closes the file.

## Back-pressure and the spill file

If the writer stalls (a slow disk, a long checkpoint, Defender scanning the
WAL) the ring fills. From then on batches go to an in-memory overflow list and
a second thread appends them to `spill/spill.bin` as `length | CRC-32 |
payload` records. While a spill is active **everything** goes there, so ids
stay in order; once the ring and the overflow are empty the writer replays the
file record by record and removes it.

- The replay position (file generation and byte offset) is committed in the
  same transaction as the replayed rows (`meta.spill_gen`, `meta.spill_offset`).
  After a crash or a quit in the middle of a replay the next start resumes
  exactly after the last record that reached the database: no record is
  committed twice. (A property test found the duplicate this prevents.)
- A torn tail (a crash in the middle of an append) ends the replay at the last
  whole record. The batch being appended is lost and is covered by the
  reconcile pass.
- If the spill cannot be written either (disk full), the batch is dropped and
  counted in `lost_batches`. The engine must record a gap row and ask for a
  reconcile: a loss is allowed only when it is detected (F2).

## Day segments and retention (#54)

One file per local day, `events-YYYY-MM-DD.db` (and `events-YYYY-MM-DD.N.db`
when the size cap, 1 GiB, forces another rotation within a day), next to a
small `catalog.db`. The event id is **one sequence across all files**, so a
cursor works across them and retention only ever removes whole oldest files.
The catalog lists each segment with its id range, times, rows and size; it is
a cache and is rebuilt from the files at start. The dictionary is repeated in
every segment (ADR 0017).

- **Rotation** happens when the first event of a later local day arrives, or
  when the file is over the cap: never on a timer. The writer checkpoints and
  closes the file, opens the next one with the same roots registered,
  carries the spill replay position and the capture cursors over, continues
  the ids and writes a `SegmentRotated` row first (its `usn` is the last id
  before, `size` the rows, the name the new file). A clock that moves back
  keeps writing in the open file; an empty file is dropped, not sealed.
- **Sealing** runs on a background thread with its own connection:
  `PRAGMA optimize`, then the segment is marked sealed in the catalog. A
  sealed segment is never written again.
- **Retention** (`[store] max_age_days`, `max_total_mb`, `max_rows`) runs
  after a rotation and at start, deleting the oldest sealed files. The newest
  segment and any unsealed one are never deleted. Each deletion is a
  `RetentionPurged` row (`usn` the first id, `size` the rows, `file_id` the
  bytes, the name the file), written with the next commit.
- **Retirement handshake**: before a file is deleted the viewer is asked to
  close it (`Retire::announce`); a delete that fails because the file is still
  open (Windows does not let SQLite's files be deleted while open) or that is
  not acknowledged is deferred, remembered in the catalog (`purge_pending`)
  and tried again at the next rotation or start. There is no viewer yet, so
  the engine uses a handshake that always acknowledges; the IPC message comes
  with the viewer (#65).
- **Readers** walk the catalog newest first with one read-only connection at a
  time (`segment::NewestFirst`). A segment that was retired in the meantime
  is a `Retired` step, not an error.
- **Start** reads the directory: a file with a segment name that is not a
  usable segment (garbage, a foreign `application_id`, a newer schema) is
  skipped, left untouched and written to `logs\segments.log`.

## What this layer does not do

- No coalescing or de-duplication: one row per raw record.
- No read paths (#55), no export (#56).
- Settings changes are not applied to the capture layer yet (#68).
