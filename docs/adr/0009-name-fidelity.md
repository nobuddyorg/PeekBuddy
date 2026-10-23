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

## Consequences

- Good: no record ever names the wrong file.
- Bad: an extra nullable column and comparison code that is not
  `str::eq_ignore_ascii_case`.

## Evidence

- Naming files, paths and namespaces, checked 2026-10-22: <https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file>
- Case sensitivity, checked 2026-10-22: <https://learn.microsoft.com/en-us/windows/wsl/case-sensitivity>
