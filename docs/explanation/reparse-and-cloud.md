# Reparse points and cloud placeholders

Two decisions about reparse points are pure and live in
`peekbuddy_core::reparse`.

## What a tag is

`link_kind` maps a reparse tag to a kind, so the link object itself can be
logged with a classification.

| Tag | Kind | Followed? |
| --- | --- | --- |
| `0xA0000003` | mount point or junction | never |
| `0xA000000C` | symlink | never |
| `0xA000001D` | WSL symlink | never |
| `0x8000001B` | app-exec link | never |
| `0x9000001C` | projected file system | it is a file |
| `0x9000xx1A` | cloud placeholder | it is a file |

A watch never follows a link out of its root; the link is logged when it is
created, deleted or retargeted, and the tag says what it is. Cloud and
projected placeholders are files that happen to carry a tag.

## Cloud churn

A sync client flips attributes and rewrites reparse data all day. Those rows
stay in the log (raw is truth) but carry `Flags::CLOUD_CHURN`, and the default
view hides them with a visible counter. `classify` decides, for a cloud tag
only, when one of these holds:

- the USN reasons are a subset of basic-info change, reparse-point change and
  close;
- the reasons include data and the source is the client replication
  management bit;
- the attribute delta is exactly a flip of recall-on-data-access, offline,
  sparse, pinned or unpinned.

A record that creates, deletes, renames, changes security or hard links, or
that writes data without the replication source, is never churn, whatever the
attributes say. Keying on "is a reparse point" would hide real writes to a
hydrated file, so the rules key on the reason set, the source and the
delta. The label (hydrated, dehydrated, pinned, unpinned, in sync,
converted) comes from the same delta.

The rules are the ones in issue #49. The lab matrix (#36) could not measure
`OneDrive` on a hosted runner, so the rule set is not widened beyond them
until an owner run supplies the missing cells.

## Opening files without waking them

Watching must never download a cloud file. `DirHandle::open` is where capture
opens a directory, and it applies `io::open::check` first: the access may not
include data bits (write, append, execute, the generic rights) and the flags
must carry `FILE_FLAG_OPEN_NO_RECALL`, which is now on by default. A caller that
turns it off gets an error, not a handle. `cargo xtask check-deps` fails on a
`CreateFileW` call anywhere in the workspace outside the directory helper and
the attribute-only opener of `peekbuddy_win::fs`.

