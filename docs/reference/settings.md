# Settings reference

`settings.toml` lives in the data directory
(`%LOCALAPPDATA%\nobuddy\PeekBuddy\`, or `data\` next to the exe in portable
mode). It is versioned, validated strictly and written atomically. Hand edits
take effect on the next start: nothing watches the file.

## Rules

- `version` is required. This build reads and writes version **1**.
- Unknown keys are rejected with a position and a "did you mean" hint.
- A file with a **newer** `version` is read leniently (unknown keys ignored),
  never written, and reported by `--diagnostics`.
- An **older** file is migrated by pure `vN -> vN+1` steps. The original is
  kept once as `settings.v<N>.bak` before the migrated file is written.
- A **corrupt** file never stops the engine: it starts with no watches
  applied, the problem is shown by `--diagnostics` (and the tray, #60), and
  the file is not overwritten.
- A missing file is created on first start with nothing watched and
  `first_run = true`.
- Changes from the viewer (#61) are validated, applied and then written to
  `settings.toml.tmp`, flushed and swapped in with `ReplaceFileW`; a failure
  leaves the old file intact.

## Keys

| Key | Type | Default | Notes |
| --- | --- | --- | --- |
| `version` | integer | 1 | |
| `[general] autostart` | bool | false | the intent; the Run key is #63 |
| `[general] theme` | `system`, `light`, `dark` | `system` | |
| `[general] first_run` | bool | true | cleared by onboarding (#70) |
| `[store] max_age_days` | integer 1-36500 | 30 in a new file | no cap when absent; a segment whose newest event is older goes |
| `[store] max_total_mb` | integer >= 1 | 2048 in a new file | no cap when absent; the oldest segments go while the total is over |
| `[store] max_rows` | integer >= 1 | none | no cap when absent; counted over all segments |
| `[view] coalesce_ms` | integer 50-500 | 150 | |
| `[view] columns` | list of `time`, `kind`, `path`, `size`, `process`, `volume` | `time`, `kind`, `path` | no repeats, at least one |
| `[filters] exclude_globs` | list of globs | empty | gitignore style, see [filters](filters.md) |
| `[filters] exclude_regex` | list of regexes | empty | searched anywhere in the path |
| `[filters] presets` | list of preset ids | empty | embedded presets |
| `[[watch]] id` | `a-z0-9-_`, 1-64 | required | stable, unique |
| `[[watch]] path` | absolute path | required | stored as written, used as `\?\...` |
| `[[watch]] recursive` | bool | true | |
| `[[watch]] enabled` | bool | true | false means paused |
| `[[watch]] exclude_globs`, `presets` | lists | empty | this watch only |
| `[[watch]] buffer_kib` | integer 4-1024 (4-64 on UNC) | none | notification buffer override |
| `[[watch]] follow_link_target` | bool | false | opt-in (F18) |

A watch root may not be the data directory or inside it (ADR 0008) and two
watches may not name the same root (compared after normalisation, ignoring
case). Deduplication by volume serial and file id, so that a junction and its
target count once, arrives with the capture layer (#45, #49).

## Where it lives in the code

Schema, parsing, validation and migrations: `peekbuddy-core::settings`
(pure, tested on Linux). Atomic file replacement: `peekbuddy-win::atomic`.
Start-up handling and `--diagnostics`: `peekbuddy-engine::settings`.
