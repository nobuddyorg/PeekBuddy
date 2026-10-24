# Filters

The filter engine (`peekbuddy-core::filter`, #40) decides, for every event,
whether it is stored and whether views show it. Rules are compiled once and
evaluated on the capture thread without allocating.

## Rules

A rule has a scope, a pattern, a polarity and an action.

| Part | Values |
| --- | --- |
| Scope | `Global` (every watch root) or `Watch(root)` |
| Pattern | `Glob`, `Regex`, `Ext` (extensions), `Kinds` (event kinds), `Flags` (flag bits) |
| Polarity | `Exclude` or `Include` (re-include) |
| Action | `HideInView` (default) or `NeverRecord` |

**The last matching rule wins.** An event no rule matches is stored and
visible.

| Last match | Stored | Visible |
| --- | --- | --- |
| none, or an `Include` rule | yes | yes |
| `Exclude` + `HideInView` | yes | no |
| `Exclude` + `NeverRecord` | **no** | no |

`HideInView` is store-then-hide: the event is in the log and a view can show
it again. `NeverRecord` is a counted loss that only a user chooses: it drops
the event at ingest and counts it per rule (`FilterSet::dropped_by_rule`,
`FilterSet::stats`). The UI and exports must label those counts. No preset
uses it. An `Include` rule cannot use it.

## Glob dialect

Patterns are gitignore-style and relative to the watch root.

| Pattern | Meaning |
| --- | --- |
| `*` | any run of characters, not across a separator |
| `?` | one character, not a separator |
| `**` | any number of directories, across separators |
| `/x` | anchored to the watch root |
| `x/y` | anchored too: a separator inside the pattern anchors it |
| `x` | no separator: matches at any depth |
| `x/` | directories only |
| `!x` | re-includes (same as an `Include` rule; needs an exclude rule) |
| `[abc]` `[a-z]` `[!abc]` | a class; separators never match inside it |
| `[*]` | a literal `*`: there is no backslash escaping |

`/` and `\` are both separators, in patterns and in paths. A separator ends a
segment, so a class cannot contain one.

`**` at the start (`**/x`) matches zero or more directories, in the middle
(`a/**/b`) zero or more, at the end (`a/**`) everything inside (but not `a`
itself), alone (`**`) everything. `**/**` is the same as `**`. `a**b` treats
the `**` as two `*`.

A pattern also matches everything below a path it matches: `build` hides
`src\build\out.o`, because an ancestor of a path is always a directory. A
directory-only pattern (`build/`) matches the directory itself only for a
directory event, and always matches events below it.

| Pattern | Path | Match |
| --- | --- | --- |
| `*.tmp` | `dir\a.tmp` | yes (any depth) |
| `/a.txt` | `d\a.txt` | no (anchored) |
| `d/*.tmp` | `d\sub\a.tmp` | no (`*` stops at `\`) |
| `a/**/b` | `a\x\y\b` | yes |
| `build/` | `build` (a file) | no |
| `build/` | `x\build\y.o` | yes |
| `[!abc].txt` | `d.txt` | yes |

## Case

Names compare case-insensitively by default, with the ordinal upper-case
table of the OS (the same as NTFS, see
[ADR 0009](../adr/0009-name-fidelity.md)). Roots whose directories are
case-sensitive never fold (`CompileOptions::overrides`).

- Globs and extensions: the pattern and the candidate are folded with the
  table, so they match exactly when the names are equal under it. Class
  members are folded one by one.
- Regexes: the candidate is folded and the regex is compiled case-insensitive
  (Unicode simple folding), which can differ from the OS table for a few
  characters.

The delivered name and the resolved long name are both tested
(`FilterSet::eval_with_resolved`). A name with a lone surrogate is matched
through its lossy UTF-8 form (U+FFFD); exact matching on such names is not
possible. A name longer than 32,767 units is not tested.

## Regex

`Regex` rules use the linear-time `regex-automata` engine and are searched
anywhere in the path (anchor with `^` and `$`). They are compiled to fully
built DFAs with explicit limits: 2 MiB of NFA, 2 MiB of DFA per scope and
case mode, 8 MiB of working memory while building. A pattern over a limit is
rejected with a readable error naming the rule.

## Compiling and evaluating

```rust
let set = FilterSet::compile(&rules)?;       // may allocate
let mut scratch = Scratch::new();            // once per thread
let verdict = set.eval(&event, name, &mut scratch); // never allocates
```

`verdict.record` says whether to store the event, `verdict.visible` whether
views show it and `verdict.matched_rule` which rule decided.
`FilterSet::memory_usage` reports the bytes held by the DFAs.

## Presets

Presets are TOML files embedded in the binary and versioned with the release
(`FORMAT_VERSION`). They only hide in views.

| Preset | Hides |
| --- | --- |
| `temp-lock-swap` | `~$*`, `*.tmp`, `*.swp`, `.~lock.*#` |
| `thumbs-desktop` | `Thumbs.db`, `desktop.ini` |
| `git-internals` | `.git/` |
| `build-dirs` | `node_modules/`, `target/`, `build/` |
| `browser-caches` | `Cache/`, `Code Cache/`, `GPUCache/`, `cache2/` |
| `search-defender` | Windows Search index files, `Windows Defender/Scans/` |
| `cloud-churn` | events flagged `cloud_churn` |

A preset file:

```toml
version = 1
name = "temp-lock-swap"
title = "Temporary, lock and swap files"

[[rule]]
glob = "*.tmp"
```

Each `[[rule]]` has exactly one of `glob`, `regex`, `ext` (a list) or `flags`
(a list of `cloud_churn`, `self_churn`, `temp`, `short_dup`).
