# See what changed since a moment

"What did that installer, build or sync just do?" is a question the raw log
answers with thousands of rows. A marker plus the Changes tab turns them into
a short list.

## Mark a moment

1. Press **Ctrl+M** anywhere in the viewer, or open the **Changes** tab and
   press **Mark now**. The engine writes a marker row into the log; it is
   named `Marker` and the time.
2. Rename it with **Rename** on the Changes tab, for example to
   `before install`. Only markers in today's day file can be renamed; older
   ones keep their name.
3. Do the thing you want to look at.

A marker is an ordinary row of the log (kind `marker`), so it also shows in
the Log tab where it happened.

## Ask what changed

On the Changes tab press **Changes since** next to the marker, or one of
**Changes in the last** 15 minutes, hour or 24 hours. PeekBuddy reads every
raw row after that point, one pass over the day files, and folds them per
file:

| Line | Meaning |
| --- | --- |
| `+ Created` | The file did not exist before and does now. |
| `- Deleted` | The file existed before and is gone. |
| `~ Modified` | The file existed throughout and changed, or was deleted and created again under the same name (a replace, such as an atomic save). |
| `> Renamed` | The file exists under another name: `old -> new`, with "also changed" when its content changed too. |

Files that were created and deleted again inside the range (temporary files)
are counted in the totals line and not listed.

Every line says `raw events: n`. Click it to open the Log tab showing the
raw rows of that file.

Moving a file out of a watched folder is shown as a deletion and moving one
in as a creation: Windows reports no rename across the border of a watch,
and PeekBuddy does not claim one.

The scan shows its progress and can be stopped with **Stop**; a stopped
scan says its list is partial. **Copy list** copies the lines as tab-separated text.

## When the answer is incomplete

If the log has a hole in the range (a buffer overflow, a lost watch, a gap
in the USN journal) and no reconcile scan covered it afterwards, the result
says **Incomplete** and names the folders. A scan that did run after the hole
found what was missed, and its rows are part of the list, so the result is
complete again and the gap is only noted. A diff is never shown as complete
across a gap nobody closed.

## Markers and retention

Markers live in the day file of their moment and go with it when retention
deletes it (`store.max_age_days`, `store.max_total_mb`, `store.max_rows`).
The Changes tab warns **! goes with its day file soon** two days before.
Export the rows you need before that.
