# 0009. Store names losslessly and follow NTFS case rules

- Status: Accepted
- Date: 2026-10-22
- Implemented or verified by: #39, #44, #53

## Context and problem statement

Windows file names are UTF-16 and may contain unpaired surrogates, which are
not valid Unicode and cannot be stored as UTF-8 text. NTFS compares names
case-insensitively with its own upcase table, and per-directory case
sensitivity exists too.

## Decision drivers

- The log must name exactly the file that changed.
- Text search and display need UTF-8.

## Considered options

1. Store lossy UTF-8 (replacement characters).
2. Store UTF-8 text plus an exact UTF-16 sidecar when needed (F17).

## Decision outcome

Chosen option 2. Every name is stored as UTF-8 text. When the UTF-16 name is
not valid Unicode, the exact UTF-16 code units are stored in a sidecar column
and the text column holds a marked lossy form. Comparisons follow NTFS case
rules (ordinal, case-insensitive by default, honouring per-directory case
sensitivity where reported). Short (8.3) names are resolved to long names when
possible and flagged when not.

## API (peekbuddy-core::name)

- `WideStr` and `WideName` hold `[u16]` units, the lossless in-core form.
  `peekbuddy-win` converts with `OsString::from_wide` / `encode_wide`;
  `OsStr::as_encoded_bytes` is never persisted or sent over IPC.
- `to_lossy_utf8(name, buf)` writes lossy UTF-8 into a caller buffer
  (`max_utf8_len` bytes always suffice) and reports `is_exact`.
- `encode_name_w` / `decode_name_w` carry the exact UTF-16LE sidecar, only
  for names that are not valid Unicode.
- `NameKind { Unknown, Long, Short, Both }` comes from the `FileNameFlags` of
  `FILE_NOTIFY_FULL_INFORMATION`; the other info classes give `Unknown`.
  `looks_like_short_name` is a shape check for `NAME~N.EXT`.
- `fold_upper`, `fold_upper_into` and `eq_ignore_case_ordinal(a, b, mode)`
  use a range-encoded upper-case table; `CaseMode::Sensitive` is for
  case-sensitive directories. Surrogate pairs are not folded.
- `escape_for_display` and `needs_escape` make lone surrogates and invisible
  format characters visible; `name_json` emits `name` plus `name_utf16` only
  when the name is not exact.

The table is read from the OS (`LCMapStringEx`, invariant, simple
upper-case; it lags the Rust Unicode data); the Windows test
`peekbuddy-win/tests/name.rs` compares it with the OS for every unit, and a
pinned checksum makes any table change a reviewed change.

## Consequences

- Good: no record ever names the wrong file.
- Bad: an extra nullable column and comparison code that is not
  `str::eq_ignore_ascii_case`.

## Evidence

- Naming files, paths and namespaces, checked 2026-10-22: <https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file>
- Case sensitivity, checked 2026-10-22: <https://learn.microsoft.com/en-us/windows/wsl/case-sensitivity>
