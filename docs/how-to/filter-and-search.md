# Filter and search the log

The bar above the grid narrows the log while you type. A change is applied
50 ms after the last key; an older read that is still running is cancelled.
An invalid entry keeps the previous results and says what is wrong next to the
field.

## The bar

| Control | What it does |
| --- | --- |
| **Search** | Names that contain the text (`Ctrl+F` focuses it, `Esc` clears it). Three characters or more use the trigram index; one or two use a `LIKE` over the names. Matching is ASCII case-insensitive in both: SQLite folds only `A-Z`, so a short search for a non-ASCII letter is case-sensitive. |
| **Created, Modified, ...** | Kind chips. Nothing chosen means every kind. A chip stands for a class of kinds: *Gaps* covers overflows, access-denied directories and store failures; *Engine* covers starts, stops, rotations and watch state. |
| **When** | Any time, the last 5 minutes, the last hour, or today (since local midnight). A custom range is written in the filter text. |
| **More** | The advanced row (below). |
| **Show / Hide** | Rows the view hides by default (cloud sync churn) are stored and counted, never dropped: "N hidden by the view" with a button to show them. |

Only names are searched by the box, not whole paths (the index holds names).
To search a path, use a glob or a regex, or a folder.

## The advanced row

- **Match**: a glob (gitignore style, like the capture filters) or a regex
  searched anywhere in the path. They are compiled by the same engine as the
  capture filters ([filters](../reference/filters.md)), so they mean the same.
  They run as a streaming post-filter over the path, in steps of a few
  milliseconds; "Scanned 3 200 000 of 10 000 000" shows the progress, and
  **Stop** ends it with what was found so far.
- **Watch**: one watch root.
- **Folder**: rows in a folder and below it (set from a row's menu).
- **Filter text** and **Copy filter**: the whole filter as one line.

## The filter text

```text
name:"report" kind:created,deleted root:"C:\\work" in:"C:\\work\\src"
since:1h glob:"*.tmp" hidden:show
```

| Key | Value |
| --- | --- |
| `name:` | text, in quotes; `\\` for a backslash and `\"` for a quote |
| `kind:` | `created`, `modified`, `deleted`, `renamed`, `attributes`, `gaps`, `engine`, comma separated |
| `root:` | a watch root, in quotes |
| `in:` | a folder, in quotes |
| `since:` | `5m`, `1h` or `today` |
| `from:`, `to:` | UTC times like `2026-10-26T12:00:00.0000000Z`; either alone is fine; not with `since:` |
| `glob:` or `re:` | one of them, in quotes |
| `hidden:` | `show` |

Edit the line and press Enter to apply it. A mistake is reported with its
position and changes nothing. The same text is what the viewer keeps in
`[viewer] filter` and restores at the next start.
