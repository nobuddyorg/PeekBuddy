# Engine and viewer protocol

The engine (always on) and the viewer (on demand) talk over one local named
pipe. The viewer reads segments from SQLite on its own; the pipe carries
commands, watch state, counters and, later, the live tail. Code: frames and
messages in `peekbuddy-core::ipc` (no Win32), the pipe in
`peekbuddy-win::pipe`, the engine side in `peekbuddy-engine::ipc_server`.

## The pipe

- Name: `\.\pipe\PeekBuddy-<16 hex digits>`, the FNV-1a hash of the user's SID
  string. An engine started with an explicit `--data-dir` (tests, the perf
  harness) uses `\.\pipe\PeekBuddy-test-<hash of the directory>` instead, so
  isolated engines can run side by side.
- Created with `FILE_FLAG_FIRST_PIPE_INSTANCE`, `PIPE_REJECT_REMOTE_CLIENTS`,
  byte mode, overlapped I/O, at most 4 instances (2 are served at a time).
- DACL `D:P(A;;GA;;;<user SID>)(A;;GA;;;SY)`: the current user and SYSTEM, no
  inheritance, no low-integrity label. Low-integrity and other-user processes
  get `ERROR_ACCESS_DENIED`.
- The server also checks the SID of each client process. The client opens with
  `SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION` and checks that the server
  process runs the expected exe.
- If the name is already taken (another engine of the same user in another
  session, or a squatter) the engine does **not** connect to it: it writes the
  reason to `logs\ipc-error.log` and exits, so there is never a second writer.
- Idle cost: the server threads wait on events. No viewer connected means no
  wakeups from the pipe.

## Frames

```text
u32 LE length | u16 LE type | payload
```

`length` counts the type and the payload (so it is at least 2). A frame,
length prefix included, is at most 1 MiB; a length outside 2 to 1 MiB - 4 is
refused from the first four bytes, before any payload is read. Strings are
`u32 LE byte length | UTF-8`. A decode error closes the connection: a broken
stream cannot be resynchronised.

## Messages (protocol version 1)

| Type | Message | Direction | Payload |
| --- | --- | --- | --- |
| 1 | `Hello` | both | `u32 protocol_version`, string `build` |
| 2 | `VersionMismatch` | engine to viewer | `u32 engine_version` |
| 3 | `ShowViewer` | to engine | none; the second launch forwards this |
| 4 | `Shutdown` | to engine | none |
| 5 | `EngineStopping` | engine to viewer | none; sent when the engine stops |
| 6 | `Selftest` | both | bytes; echoed back (tests) |
| 7 | `Counters` | engine to viewer | six `u64`: events received, rows persisted, overflows, gaps, ring high-water, last batch size |
| 8 | `WatchState` | engine to viewer | `u32 index`, `u8 state` (0 live, 1 paused, 2 gap, 3 lost) |
| 9 | `Resync` | engine to viewer | `u64 from_id`: live-tail frames were dropped, re-query from this row |
| 10 | `Command` | viewer to engine | `u8 kind` then fields: 0 pause all, 1 resume all, 2 set watch paused (string id, `u8`), 3 add watch (string path), 4 remove watch (string id), 5 apply settings (string TOML) |
| 11 | `CommandResult` | engine to viewer | `u8 ok`, string message |

Types 12 and up are reserved: the live tail (committed rows, at most 30
batches/s and 2000 rows per batch, #52 and #65) and the segment retirement
handshake (#54).

A viewer starts with `Hello`. The engine answers `Hello` when the versions
match and `VersionMismatch` otherwise (an old engine still running after an
update): the viewer then asks the user to restart PeekBuddy.

What works today: `Hello`, `ShowViewer`, `Shutdown`, `Selftest`,
`EngineStopping`, and `Command::ApplySettings`, which validates the text with
the settings validator, writes it atomically when it is valid and returns the
problems (with line and column) when it is not. The other commands answer
`CommandResult { ok: false }` until pause (#63) and watch management (#68)
exist. Counters, watch state and resync are defined and encoded but not sent
yet: they need the writer (#52).

## Back-pressure (design, applies when the live tail exists)

The writer hands batches to the pipe thread with a bounded try-push. A slow or
stuck viewer loses live-tail frames and gets one `Resync` marker; capture and
the writer never wait for it. A write to a client that does not read times out
after 5 s and drops the client.

## Version history

| Version | Change |
| --- | --- |
| 1 | First version: the messages above. |
