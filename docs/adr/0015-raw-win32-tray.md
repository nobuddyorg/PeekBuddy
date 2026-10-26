# 0015. Build the tray on raw Win32

- Status: Accepted
- Date: 2026-10-26
- Implemented or verified by: #60

## Context and problem statement

The engine needs a notification-area icon with a context menu that survives
explorer restarts and costs nothing at idle (F6). Two ways: the `tray-icon`
0.26.1 and `muda` 0.21.2 crates on the engine's message loop, or
`Shell_NotifyIconW` and `TrackPopupMenuEx` directly on the pinned
`windows-sys` (ADR 0010).

## Decision drivers

- No new dependencies in the always-running process; every dependency is
  binary size and an update to track.
- Full control of what the loop does: no polling, no timers.
- The menu is small and fixed.

## Considered options

1. `tray-icon` + `muda`.
2. Raw `Shell_NotifyIconW` and popup menus in `peekbuddy-win::tray`.

## Decision outcome

Chosen option 2. The tray is about 300 lines in `peekbuddy-win::tray` plus a
pure model (`tray_model`) that is tested on Linux. The crates would also need
their own event plumbing (`set_event_handler`) on top of the window procedure
the engine already owns, and they depend on `crossbeam-channel`.

**What was not measured.** The issue asks to build both and compare exe size,
private bytes and ready time before choosing. Only option 2 was built. On the
owner PC (Intel N150) the release exe is 1.5 MB with settings and tray; engine
ready (window up, tray added) takes about 80 ms against the provisional 100 ms
budget, of which most is `Shell_NotifyIconW` waiting for explorer. The crate
route is rejected on dependency grounds, not on numbers; the owner may ask for
the measurement.

## Consequences

- Good: no new dependencies; the identity of the icon is `hWnd` + `uID`, so a
  portable exe can move; `TaskbarCreated` handling is ours and tested.
- Bad: icons are drawn procedurally (`tray_model::render_icon`) until the
  brand assets of #88 exist; the owner-draw menu look of `muda` is not
  available (the plain Win32 menu is used).

## Evidence

- Shell_NotifyIconW, checked 2026-10-26: <https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shell_notifyiconw>
- NOTIFYICONDATAW (version 4 callbacks), checked 2026-10-26: <https://learn.microsoft.com/en-us/windows/win32/api/shellapi/ns-shellapi-notifyicondataw>
- tray-icon 0.26.1, muda 0.21.2, checked 2026-10-26: <https://crates.io/crates/tray-icon>
