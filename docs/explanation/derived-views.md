# Derived views

PeekBuddy stores every event the OS reports, unchanged (raw is truth,
[ADR 0003](../adr/0003-raw-is-truth.md)). One logical save is two to four raw
events and a rename is two, so a viewer that only shows raw rows is noisy.
`peekbuddy-core::derive` turns the raw rows into a readable list of display
items **without ever hiding a row**.

It runs in the viewer and the query layer. The engine never calls it, and
nothing it produces is stored.

## The contract: "raw events: n"

- A display item is a raw row, an unpaired rename half, or a **group**.
- A group lists the ids of every raw row it stands for and their count
  (`raw_count`). The UI shows that count ("raw events: 7") and can expand the
  group to the raw rows.
- Across all items, every raw row appears exactly once, and the items are in
  order of their first member's id. A group takes the place of its first
  member.
- `Overflow`, `WatchStateChanged` and `Reconciled*` rows are barriers: they
  are never part of a group and they close every open group of their root.
  Rows after an overflow describe a state diff, never stitched to rows
  before the gap.

A wrong guess is therefore cheap: if a pattern groups rows that were not one
save, the group still shows all of them and how many there are.

## How it works

The deriver is a streaming state machine. It reads event ids and the event
time `ts` only: no wall clock, no timer (F6). Chunking the input, taking
items in pages or reading it in one go gives the same output.

**Bursts.** Events about one path (compared with the case rules of
[ADR 0009](../adr/0009-name-fidelity.md), `/` and `\` alike) that follow each
other within a window belong to one burst. The window is 50 to 500 ms (the
save window in detail mode, the larger of the save and coalescing windows in
compact mode). A burst closes when its window passes without a new event,
when a barrier hits its root, when the stream ends or when a bound is hit.

**Bounds.** At most `max_open_groups` bursts are open (default 4,096). When
a new one would exceed that, the oldest closes as it is and `forced_flushes`
counts it. Pending items are bounded to four times that.

**Holding back.** An open burst holds the rows behind it until it closes,
to keep the order. In a live view that is at most one window of event time;
a viewer frame calls `advance(ts)` to let time pass without events.

## Patterns

When a burst closes, it becomes one of these.

### Rename

A `RenamedFrom` waits for its `RenamedTo`:

- the very next event, when it names the same file id (or either id is
  unknown): `ReadDirectoryChangesW` delivers the two consecutively;
- a later event with the same file id (USN records pair by FRN).

A matched pair is a **Renamed** group with the old and the new path. A
`RenamedTo` without a waiting `RenamedFrom`, and a `RenamedFrom` whose burst
closes first, are shown as **unpaired** rows. Halves in different roots never
pair, so a move out of the tree shows an unpaired half (or a plain removal,
depending on what the OS reports).

### Atomic save (**Saved**)

A burst is a save when it matches one of these, checked in this order:

1. **Temp file, then rename**: a file is created, written and renamed to
   another name (`Created t`, `Modified t`, `Renamed t -> x`). Word and most
   editors that write a temp file.
2. **Backup**: the target is renamed to a backup, a new file is written under
   its name and the backup is deleted (`Renamed x -> b`, `Created x`,
   `Deleted b`).
3. **Delete and create**: a name is deleted and created again.

`ReplaceFileW` (temp file, backup, rename, delete) shows up as 1 and 2 together
and matches 1. A save is one **Saved** group with every row of the burst and
the target path.

### Coalescing (**Coalesced**)

In compact mode, a burst of two or more events without renames is one group
per path: a program that writes a file in several steps shows one row.
Detail mode does not coalesce.

## Examples

A Word save (detail mode), nine raw rows:

```text
Saved [1, 2, 3, 4, 5, 6, 7, 8, 9] report.docx (raw events: 9)
```

A rename and a later edit of the new name:

```text
Renamed [1, 2] docs\a.md -> archive\a.md (raw events: 2)
raw 3
```

A rename half that never found its other half:

```text
unpaired 1
```

## What it does not do

- It does not decide what the user sees by default; that is the viewer's
  filter and view mode (#66, #67).
- It does not use look-behind: reading a page backward means deriving again
  from a quiet point before the page. A group that spans a page edge is the
  same as long as the deriver started before its first member.
- It does not know application save strategies it was not taught. New
  patterns go into `derive/classify.rs` with a fixture.
