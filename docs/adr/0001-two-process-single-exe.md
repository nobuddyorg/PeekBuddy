# 0001. Ship one exe with an engine role and an on-demand viewer role

- Status: Accepted
- Date: 2026-10-22
- Implemented or verified by: #59, #61, #65, #9 (`xtask check-deps`), #27

## Context and problem statement

PeekBuddy lives in the tray all day, but it also needs a rich window that can
scroll millions of rows. A GPU-backed UI stack (eframe/egui with glow or wgpu)
costs tens of megabytes of resident memory and loads graphics drivers. Keeping
that stack loaded in the always-on process would blow the idle memory budget
(F8) for a window the user opens a few times a day.

## Decision drivers

- Idle engine footprint (F8) and wakeup budget (F6).
- The engine must keep capturing while the viewer is closed, crashes or hangs.
- One file to download, verify and autostart (M14).

## Considered options

1. One process: eframe in the tray process, window hidden when closed.
2. Two exes: `peekbuddy-engine.exe` and `peekbuddy-viewer.exe`.
3. One exe, two roles: engine by default, viewer with `--viewer` (F1).

## Decision outcome

Chosen option 3. `peekbuddy.exe` starts the engine role by default: a Win32
message loop with a hidden top-level window for the tray, `WM_QUERYENDSESSION`
/ `WM_ENDSESSION` and `TaskbarCreated`, the IOCP capture threads, the
reconciliation scanner and the single SQLite writer. `peekbuddy.exe --viewer`
starts the eframe/egui window with read-only SQLite connections and a named-pipe
client for the live tail and commands.

Each role takes a single-instance mutex `Local\PeekBuddy.{GUID}` (one GUID per
role). A second engine start exits; a second viewer start forwards "show
viewer" to the running viewer and exits.

The engine crates (`peekbuddy-core`, `peekbuddy-engine`) must not depend,
transitively, on `eframe`, `egui`, `winit`, `wgpu` or `glow`. `xtask check-deps`
enforces this in CI.

## Consequences

- Good: the always-on process never loads a GPU stack; closing the viewer frees
  all its memory; a viewer crash cannot stop capture.
- Good: still one file to ship, sign later and autostart.
- Bad: an IPC protocol (#61) and two startup paths to test.
- Bad: the viewer reads SQLite concurrently with the writer, so WAL and
  read-only connections are mandatory (#52).

## Evidence

- eframe 0.37.0, checked 2026-10-22: <https://crates.io/crates/eframe/0.37.0>
- egui RSS reports (glow about 31 MB, wgpu about 142 MB) come from a single
  egui 0.33/0.34 report on unknown hardware and are unverified until #33.
- Named pipes, checked 2026-10-22: <https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipes>
