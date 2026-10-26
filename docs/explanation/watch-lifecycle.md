# What happens when a watch is lost

A competing watcher goes quiet when a folder is deleted, a USB disk is pulled,
BitLocker locks a volume or the laptop sleeps. PeekBuddy promises that a watch
is never dead without that being logged. Every change of a watch's state is
exactly one `WatchStateChanged` row with the old state and the reason in it,
and every state is visible in the viewer.

The state machine is `peekbuddy_core::lifecycle::Machine`: pure, one per
watch root, typed signals in, at most one transition and a few actions out. The
Windows side is `peekbuddy-engine::capture_host`, which joins the watcher, the
reconcile worker and the machines through a queue, so that no system callback
ever blocks.

## States

| State | Meaning | Handle | Viewer shows |
| --- | --- | --- | --- |
| Arming | being armed | open | live |
| Live | events are flowing | open | live |
| Overflowed | a gap happened, a reconcile is wanted | open | gap |
| Reconciling | the reconcile runs | open | gap |
| Suspended (user) | paused by the user, still armed (ADR 0016) | open | paused |
| Suspended (query remove) | the volume asked to be removed | closed | paused |
| Suspended (locked) | locked, dismounted or being checked | closed | paused |
| Lost | the watch is gone, with a reason | closed | lost |
| Restored | a lost watch came back, its reconcile is wanted | open | gap |

## Reasons

The reason code is in the low 16 bits of the row's `raw_mask`, the old state's
wire byte in bits 16 to 23. The codes are on disk and only ever appended.

| Code | Reason | Comes from |
| --- | --- | --- |
| 1 | armed | the first read was issued |
| 2 | arm failed | the root could not be opened or watched |
| 3, 4, 5 | overflow, ring full, parse error | a gap in the stream |
| 6, 7, 8 | reconcile started, done, cancelled | the reconcile worker |
| 9, 10 | user pause, user resume | the tray, the pipe, the settings |
| 11, 12 | query remove, query remove failed | `CM_NOTIFY_ACTION_DEVICEQUERYREMOVE` and its failure |
| 13 | device removed | remove pending or complete |
| 14, 15 | volume locked, volume dismounted | the volume's custom events |
| 16 | volume back | mount, unlock, a failed lock, an arriving volume |
| 17, 18 | root deleted, root moved | access denied or cleanup from the read, or the guard on the parent |
| 19 | root restored | the parent watch saw the root again |
| 20, 21 | device not ready, device gone | a probe after an unexpected error |
| 22 | verified ok | a probe found everything fine after an unexpected error |

## What does what

- **An unexpected completion status** is never guessed at: the host probes the
  root (`peekbuddy_win::probe`: open it, `FSCTL_IS_VOLUME_MOUNTED`, and if it
  cannot be opened, whether the volume's own root can). Fine means reopen and
  reconcile; not ready or gone means `Lost`.
- **A root that is deleted or renamed**: a deleted root ends a read with
  access denied; a renamed one would silently keep being watched under its new
  name, so every live root also has a **guard**, a small non-recursive watch of
  its parent, and the root's own name going away is `Lost(root deleted)` or
  `Lost(root moved)`.
- **A lost root coming back** is also an event: the nearest existing ancestor
  is watched, and an event on the way to the root's path makes the host look
  again. When the root opens, the watch is `Restored`, reopened and
  reconciled. No polling.
- **Volumes**: `CM_Register_Notification` on a handle in each watched volume
  reports query-remove, remove, lock, dismount, mount and BitLocker changes.
  A query-remove closes both the watch handle and the registration's own
  handle in answer, so "Safely Remove Hardware" works. Arrival of a volume
  interface restores lost watches. Unregistration happens on the host's worker
  thread, never inside a callback.
- **Power**: `PowerRegisterSuspendResumeNotification` in callback form. A
  suspend flushes the ring and commits; a resume verifies every volume and
  expects an overflow, which takes the normal gap path.
- **Session end**: `WM_QUERYENDSESSION` is answered at once; `WM_ENDSESSION`
  stops capture, drains the ring, commits and persists checkpoints (the
  shutdown steps in order). A test sends it with 1,000 events pending.

## Out of order

The machine accepts any signal in any order: a remove without a query, a
failed query without a query, repeated mounts. A signal that means nothing in
the current state is ignored without a row. While the user has a watch paused,
no volume event un-pauses it.

## Not done yet

A slow health timer for UNC roots (the one periodic timer the wake budget
allows), a `WatchState` message to a connected viewer, and the detach and
attach test on a virtual disk running on a schedule (it exists and is skipped
unless `PEEKBUDDY_VHD_TESTS=1` on an elevated machine).
