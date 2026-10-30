# Export the log

An export writes the rows of the current filter, one line per stored record,
oldest first, to CSV or JSON Lines. It streams: a million rows never sit in
memory. The file is written as `<name>.partial` and renamed when it is
complete; cancelling, closing the viewer or a failed write (a full disk)
deletes the partial file, and a file that already had the final name is left
alone.

## Formats

- **CSV** follows RFC 4180: CRLF line ends, fields in quotes when they hold a
  comma, a quote or a line break, UTF-8. A byte order mark for Excel is an
  option and off by default.
- **JSON Lines** has one object per line, UTF-8, no byte order mark.

## Columns

| Column | Meaning |
| --- | --- |
| `id` | event id |
| `ts_utc` | `YYYY-MM-DDTHH:MM:SS.fffffffZ`, 100 ns precision; this is the authoritative time |
| `ts_local` | the same instant with the offset that was valid then on the exporting machine |
| `kind` | `created`, `modified`, `deleted`, `renamed_from`, `renamed_to`, `attributes_changed`, `security_changed`, `stream_changed`, `link_changed`, `reconciled_*`, `overflow:<reason>`, `watch_state:<state>` and the engine's own rows |
| `source` | `rdcw`, `usn`, `reconcile` or `system` |
| `path`, `old_path` | lossy UTF-8, components joined by `\`; `old_path` is set on renames |
| `is_dir` | `true` or `false` |
| `size`, `attrs` | empty when unknown; `attrs` is the decimal attribute mask |
| `raw_mask` | the OS value, `0x` and 8 hex digits |
| `file_id` | the 128-bit file id, 32 hex digits |
| `root` | the watch root |
| `marker` | `gap` for a gap row, `reconcile` for rows the reconciler wrote, else empty |
| `flags` | row flag bits |
| `path_utf16`, `old_path_utf16`, `root_utf16` | the exact name, only when it is not valid Unicode |

Names that Windows allows but Unicode does not (a lone surrogate) are written
in the `_utf16` columns as printable ASCII, `\\` for a backslash and
`\u{XXXX}` for every other UTF-16 code unit. A JSON `\uD800` escape never
appears. Right-to-left and other format characters are written as they are:
they are data, and only the viewer escapes them for display.

## Excel-safe mode

A file name can start with `=`, `+`, `-` or `@`, and a spreadsheet may treat
such a cell as a formula. The optional Excel-safe CSV mode puts `'` before
those cells. It alters the data, so it is off by default; the reader undoes it
when told the file was written that way.

## Reading an export back

`peekbuddy_core::export::read` parses both formats into rows. The tests, the
churn comparator and the fuzzers use it; a damaged line is an error with its
line number, never a panic.
