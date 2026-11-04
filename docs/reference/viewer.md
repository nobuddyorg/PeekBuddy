# Viewer

`peekbuddy.exe --viewer` is the on-demand window. It is a separate process so
the always-running engine never loads a GPU stack ([ADR 0001](../adr/0001-two-process-single-exe.md));
closing it gives all its memory back. The engine starts it from the tray; you
can also run it by hand.

## What starts and what ends it

- One viewer per session. The mutex `Local\PeekBuddy.{GUID}` decides; a second
  launch raises the named event `Local\PeekBuddy.Viewer.Show.{GUID}`, asks
  Windows to let the first window take the foreground, and exits 0. The first
  viewer waits on that event in a thread that sleeps in the kernel.
- The window closing ends the process with exit code 0 (within 2 s).
- Killing the viewer (`TerminateProcess`) leaves the engine capturing: the two
  share nothing but the pipe and the data directory. A test covers it.
- A renderer or window that cannot start is reported in a message box, written
  to `logs\viewer.log`, and the exit code is not 0. A silent exit never
  happens.

## Renderer

glow (OpenGL) on x64, wgpu on DX12 on arm64, which has no OpenGL driver
([ADR 0005](../adr/0005-viewer-renderer.md)). The `wgpu` cargo feature of
`peekbuddy` moves x64 to wgpu as well, for A/B runs. The wgpu instance is
built explicitly, so `WGPU_BACKEND`, `WGPU_POWER_PREF` and friends have no
effect. The adapter name (the OpenGL renderer string with glow), backend
(`OpenGL` or `Dx12`) and device type go to `logs\viewer.log` at the first
frame; `Cpu` means software rendering (WARP, or `GDI Generic` when no OpenGL
driver is installed), which is logged as a warning.

Missing glyphs draw as a box. egui 0.37 can also be told to panic on them,
which its test harness does on purpose; the viewer never does, because file
names contain any character. Shapes the viewer draws itself (status marks) are
painted, not typed, because the bundled fonts have no check or cross.

## Engine link

The viewer connects to the engine's pipe on its own thread ([IPC](ipc.md)):

| State | Meaning | What the user sees |
| --- | --- | --- |
| Connecting | the first attempt runs | nothing |
| Connected | hello answered with the same protocol version | "Engine running" |
| Gone | no engine, or it stopped or died | a banner with **Start engine**; retried every second |
| Mismatch | the engine speaks another protocol version | a banner asking to quit PeekBuddy from the tray and start it again, with **Check again** |

Browsing the log works in every state: the viewer reads the segments itself.
A state is reported once per change, so an idle viewer paints nothing. While
connected the thread asks the pipe whether the engine is still there twice a
second (`PeekNamedPipe`, which never blocks and costs the engine nothing);
the pipe handle is synchronous, so the thread never sits in a read.

## The grid

The grid is a virtualized table (`egui_table`, fixed row height, so scroll
maths is O(1)) over the whole log, newest row first. Reading never happens on
the UI thread: [`worker`](../../crates/peekbuddy-viewer/src/worker.rs) owns the
database connections, and a page that has not arrived is drawn as an ellipsis.

- **Pages are keyset pages.** The grid holds one *island* of rows (at most
  12,800, 64 pages) and the virtual index of its first row. Reaching an edge
  fetches the next page from the oldest or newest id held; a scrollbar drag or
  the End key far from the island *seeks*: event ids are one contiguous
  sequence (R2), so with no filter row `i` has id `newest - i`. With a filter
  that hides rows, the index is an estimate ("about N events") and the island
  lands near it; the rows in it are always right.
- **Memory does not depend on the log.** The island is trimmed to the cap
  around the view, however far the user scrolls. A test scrolls a 10M-row log
  and checks the rows held and the id of the row at every index.
- **Live tail.** The engine does not push committed rows to the viewer yet, so
  the worker follows the log itself: every 250 ms while rows keep arriving,
  backing off to 1 s when nothing happens, one small keyset query each. It
  wakes the UI only when rows arrive, so an idle viewer paints no frame. A
  batch that skips ids (a dropped frame, once the engine pushes) is refused and
  asked for again from the newest id seen.
- **Follow mode.** At the newest row new rows appear at the top. Scrolling away
  stops following; new rows are then only counted ("5 new events"), so the view
  never moves under the pointer. Clicking the pill, or scrolling back to the
  top, follows again.
- **Rows.** Names are display-escaped (F17): a right-to-left override or a lone
  surrogate shows as `\u{202E}` or `\u{D800}`, and the row carries a `!`
  badge. The kind is a symbol plus a word, gap rows are tinted and say "Gap:",
  and times are local with UTC in the tooltip. Reconcile rows carry their
  detection time, so the time column can step back.
- **Compact view.** The Compact toggle folds renames, atomic saves and bursts
  of changes to one file behind their newest row, which shows `> 3` (the number
  of raw events in the group). Clicking it opens the group, and the older rows
  appear below. Nothing is hidden for good: every raw event is still a row (the
  copy command copies the rows of a selection, folded or not), and with every
  group open the grid shows exactly the raw rows. The groups come from the
  derived-view rules of `peekbuddy-core` and are found among the rows the grid
  holds, at most 12,800, so a group at the edge of that window can be folded
  only as far as the window sees it. The scrollbar keeps counting raw rows, so
  the total says "about". Folded rows have no height; the arrow keys skip
  them. The toggle is not saved between runs.
- **Keys.** Arrows, Page Up/Down, Home and End move the selection; Shift
  extends it; the view follows the cursor. The column chooser (Columns menu)
  is saved in `[view] columns`.

## Row actions and the details pane

Right-click a row (or select it and press the menu key) for its actions:

- **Copy path**, **Copy row** (or **Copy N rows**): the rows as tab-separated
  text with a header; `Ctrl+C` does the same. Tabs, line ends and control
  characters in a name are written as `\t`, `\n`, `\u{..}`, so every row has
  exactly the columns of the header; backslashes are left alone, so a copied
  path still pastes as a path.
- **Show in Explorer** (the item selected) and **Open containing folder**. A
  deleted item opens the nearest folder that still exists. Paths with the
  `\\?\` prefix are converted for the shell. Only Explorer is started: **no
  action opens, reads or runs the file**, so a cloud placeholder is never
  hydrated and a hostile file never runs. There is deliberately no "Open".
- **Filter to this folder / file / kind**: sets the filter bar.

The details pane on the right (the **Details** button toggles it, and the
choice is kept in `[viewer] details_hidden`) shows everything stored about the
selected row: the times in local, UTC and raw FILETIME form, the size,
attributes decoded to `FILE_ATTRIBUTE_*` names (including `RECALL_ON_OPEN`,
`PINNED` and `UNPINNED`), the reparse tag, the file id, and the raw value the
OS reported in words (`FILE_ACTION_*` for RDCW, `USN_REASON_*` for USN). A bit
or a value no table knows is shown as hex and never dropped. A name that had to
be escaped carries a warning, and its exact UTF-16 code units are one click
away.

Not in this version: the compact view (derived groups, #41; the grid shows one
row per stored record), ascending order, and saved column widths.

## Watches

The **Watches** tab next to **Log** lists every watch the engine reports, one
row each: a shape, the state as a word, the folder (without the `\\?\`
prefix), "this folder only" when it is not recursive, and **Pause** or
**Resume** and **Remove**. The states are the nine of the watch lifecycle
(F19) plus "Starts after restart" for a saved watch that is not armed yet;
each has a shape of its own (so colour is never the only signal) and a tooltip
that says what it means and what PeekBuddy does next. A paused watch also
says how many changes it counted but did not record. The tab shows how many
watches need attention (lost, overflowed, drive locked). See
[Gaps and reconciliation](../explanation/gaps-and-reconciliation.md).

**Watch a folder** takes a path or **Browse...**, which opens the native
folder dialog (`IFileOpenDialog`, folders only, several at once, the usual
keyboard) on a thread of its own, so the window keeps painting. The viewer
only sends the path; the engine decides (see [IPC](ipc.md)): the data
directory, the exports folder and anything in them are refused with a message,
a whole drive asks for a second click ("Add anyway"), a network folder or a
folder inside another watch is added with a word about it. **Remove** asks
first and says that the rows already recorded stay.

A new watch is saved at once and starts recording when the engine starts the
next time. The tab says so and offers **Restart engine now**: the viewer asks
the engine to shut down, waits for it to go and starts it again. The list is
asked for when the tab opens, after every command and every two seconds while
the tab is open; nothing asks while the Log is shown.

Not in this version: the engine's backend, filesystem and buffer details, the
events-per-minute sparkline, per-watch filters and the recursive toggle, a
note when a picked folder is itself a junction.

## Settings and the first-run screens

The **Settings** tab has the retention caps (days, megabytes, rows; empty means
no cap), the coalescing window of the compact view (50 to 500 ms; it only
changes how rows are grouped, never what is stored), the view presets (all of
them "hidden in view (recorded)"), the theme (applied at once after a save),
the autostart switch (off unless ticked; the engine's answer says when Task
Manager has switched the entry off), the data folder with **Open folder**, the
About section (wordmark, version, commit, target, the graphics adapter, the
licence) and **Copy diagnostics...**.

Saving sends the whole file to the engine as `ApplySettings`. The engine
validates, writes it atomically and answers; a refusal is shown under the form
and next to the field it names, and nothing changes. Without an engine the
viewer validates with the same code and writes the file itself. **Copy
diagnostics...** runs `peekbuddy --diagnostics` and shows the text in a
preview before anything can be copied, because it contains the watched paths;
nothing is sent anywhere.

The first-run screens (welcome and the privacy statement, folders and
presets, where the tray icon went, autostart, done) are shown when nothing is
watched and the `first_run` flag is still set. Leaving them, by finishing or
skipping, clears the flag. A measuring run (`--viewer-probe`) never shows
them. **Settings, Show the welcome screens again** brings them back with the
current choices.

Not in this version: the number of segments a lowered cap would delete (the
confirmation says that older segments go, without a count), the licence texts
inside the app (they ship as `THIRD_PARTY_LICENSES.html` in the release
download), a StartupApproved hint beyond the engine's message, and the
Windows 10 wording of the tray hint (not checked on both systems).

## Theme and title bar

`[general] theme` (`system` by default) maps to egui's theme preference and
follows Windows live. The title bar follows the resolved theme through
`DwmSetWindowAttribute(DWMWA_USE_IMMERSIVE_DARK_MODE = 20)`; Windows builds
that do not know the attribute answer `E_INVALIDARG`, which is not an error.
Mica is not used: it needs a transparent swap chain, and the measured gain is
nothing.

## Window geometry

The viewer remembers its window in `[viewer]` of `settings.toml`
([settings](settings.md)) and not through eframe's `persistence`, which writes
outside the data directory ([ADR 0008](../adr/0008-data-dir-and-self-exclusion.md)).
Positions are virtual-screen pixels, and the window is placed with
`SetWindowPos` rather than a logical size, because a logical size would be
converted with the wrong monitor's scale on a mixed-DPI desk. On start the
saved rectangle is kept if enough of it is on a monitor, shrunk and moved into
that monitor's work area if needed, and otherwise centred on the primary one.

## Measuring: `--viewer-probe`

`peekbuddy.exe --viewer --viewer-probe <seconds> [--perf-probe-out <file>]
[--data-dir <dir>]` opens the window, waits for it to settle, leaves it alone
for the given seconds, and closes it. It writes one JSON line:

```json
{"role":"viewer","frames":12,"first_frame_us":183000,"idle_seconds":10,"idle_frames":0,"adapter":"...","backend":"Dx12","device_type":"Cpu"}
```

`idle_frames` is the number of frames painted while nothing happened; it is 0.
`first_frame_us` is microseconds since process creation (F7). With
`--data-dir` the viewer belongs to the isolated engine of that directory.

## Known limitations

- A pointer move repaints the whole window ([egui#8695](https://github.com/emilk/egui/issues/8695)).
  The hover gate is not implemented: it needs a real GPU to show whether the
  repaint costs anything there. On WARP and over Remote Desktop it costs CPU.
- Hosted CI runners render through WARP, so frame times there are not
  representative (F12). The memory figures of the wgpu build are in
  [report 0003](../spikes/0003-viewer-renderer.md).
- File names with characters the bundled fonts lack draw as boxes. System font
  fallback is off, because eframe enumerates system fonts on a background
  thread that can block the first lookup of a missing glyph.
