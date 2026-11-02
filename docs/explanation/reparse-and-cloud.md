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

### Tagging at ingest

`ReadDirectoryChangesExW` reports a modification with the attributes after the change
and no reason mask, so `classify` has nothing to compare. `ChurnCache` keeps
the last attributes per cloud file (direct-mapped, 2048 slots, allocated when
the first cloud file shows up). The RDCW decoder sets `Flags::CLOUD_CHURN`
on a modify whose attribute delta is exactly a flip of the sync-state bits.
A first sighting, a data write (attributes unchanged), a create, a delete or
a cache collision is never tagged: the row is shown, never hidden by mistake.

## Opening files without waking them

Watching must never download a cloud file. `DirHandle::open` is where capture
opens a directory, and it applies `io::open::check` first: the access may not
include data bits (write, append, execute, the generic rights) and the flags
must carry `FILE_FLAG_OPEN_NO_RECALL`, which is now on by default. A caller that
turns it off gets an error, not a handle. `cargo xtask check-deps` fails on a
`CreateFileW` call anywhere in the workspace outside the directory helper and
the attribute-only opener of `peekbuddy_win::fs`.

## What the link tests show

`fs_serial_links.rs` runs the real engine on a tree with real NTFS junctions:

- a junction to a sibling folder is one row (the link object); files and
  directories created behind it are not logged through the root;
- the reconcile walk lists a junction as an entry and does not enter it, on the
  baseline and on the walk after a restart;
- a junction to its own root neither loops nor produces a path through it;
- each kind of link is one object with its kind: a junction, a directory
  symlink and an app-execution alias (a file with the `APPEXECLINK` tag,
  built by `create_app_exec_link`) each have one creation row, and a row
  carries `reparse_tag`, so `reparse::link_kind` names it. Nothing behind a
  link is logged through it;
- two junctions to one directory, and a junction back to the root, with
  `follow_link_target` on, add that directory once and the loop never.

## Following link targets (opt-in)

By default a link is a leaf. A watch with `follow_link_target = true` also
watches where its directory links point. At start the engine searches the
tree for symlinks and junctions (`links.rs`), resolves each, and adds the
target as a watch of its own (`derived_from` names the watch whose link it
is; the id is `<id>.link<n>`). Such a watch shows up in the tray, can be
paused, and is never written to `settings.toml`.

`LinkTargets` in `peekbuddy_core::reparse::follow` decides, by file identity
(volume serial number and file id, so two links, a short name and a long name
all give one identity):

- a target that is watched already is a duplicate and added once;
- a target inside a watched root, or one that contains a watched root (a link
  to a parent, a link back into the tree), would log twice or loop and is
  skipped;
- a link that cannot be resolved (the compatibility junctions of a user
  profile deny access by design) is skipped and counted.

The engine log gets one line, `links: followed N, duplicate N, overlapping N,
unreadable N`. At most 256 targets are added. The search runs once at start:
a link created later is followed at the next start.

## The default view hides churn and counts it

`Filter::default_view()` hides rows with `Flags::CLOUD_CHURN` (`flags & 1 = 0`
in the page query) and `Reader::hidden` counts what a filter hides, for the
"N hidden" line. The rows stay in the log; the page query without a `hide`
flag, the export and a raw read return them.

## The classifier is checked three ways

- `fs_lab_fixtures.rs` replays every recorded RDCW and USN row of the lab
  (report 0006, 329 rows) through `classify`: none is churn, as the lab had
  no cloud provider, and every tag it saw is a junction or a symlink;
- `reparse_alloc.rs` counts heap allocations of `classify` and `link_kind`
  over a mix of eight changes, repeated: zero (budget
  `alloc.reparse_per_change`);
- the criterion bench `reparse/classify_8` has a budget row of its own
  (`bench.reparse.classify_8`).
