# Command line

`peekbuddy.exe` is one program with several roles. The role is chosen by the
flags; a plain start runs the engine with its tray icon. Flags of different
roles exclude each other, and an unknown flag is an error: the program prints
the reason and exits with code 2.

## Roles

| Command | What it does |
| --- | --- |
| `peekbuddy.exe` | The tray-resident engine: watches the folders of the settings and records changes. |
| `peekbuddy.exe --viewer` | Opens the viewer window on the log. A second launch raises the open window and exits with 0. |
| `peekbuddy.exe --diagnostics [--full]` | Prints the diagnostics text, one line per fact (PeekBuddy and Rust versions, SQLite, Windows build, architecture, the viewer's renderer, the data directory and mode, whether the settings are valid, whether the engine runs, the watches, Defender's real-time state) and exits. `--full` adds the watched paths and the user's folder names, which are left out by default so the text can be shared. |
| `peekbuddy.exe --ipc-ping [--data-dir <dir>]` | Connects to the running engine's pipe, says hello and exits: 0 when the engine answered, otherwise the Windows error code (5 is "access denied"), 1 when there is none. |

## Flags

| Flag | Applies to | Meaning |
| --- | --- | --- |
| `--headless` | engine | Runs without a tray icon and never touches the autostart value in the registry; used by tests and the harness. |
| `--tray` | engine, with `--headless` | Shows the tray icon of a headless engine. |
| `--data-dir <dir>` | engine, viewer, ping | Uses `<dir>` for settings, logs and segments instead of the user's data directory, and gives the engine and viewer of that directory their own pipe and window title. Only accepted with `--headless`, `--perf-probe`, `--viewer` or `--ipc-ping`: a normal engine never takes another directory. |
| `--perf-probe <seconds>` | engine | Starts the engine, leaves it idle for the given seconds and writes the milestones (process start, settings loaded, store opened, engine ready) and counters as one JSON line. |
| `--viewer-probe <seconds>` | viewer | Opens the viewer, leaves it alone for the given seconds and writes the first frame time, the idle frame count and the renderer as one JSON line ([viewer](viewer.md#measuring---viewer-probe)). |
| `--viewer-probe-scroll` | viewer, with `--viewer-probe` | Scrolls the log with the wheel for the probe's seconds instead of leaving it alone, and reports the frame times ([viewer](viewer.md#scrolling---viewer-probe-scroll)). |
| `--perf-probe-out <file>` | either probe | Writes the probe's JSON to `<file>` instead of standard output. |
| `--full` | `--diagnostics` | See above. |

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | The role ran and ended normally. |
| 1 | The role failed (the reason is in `logs\app.log`, `logs\viewer.log` or, for a pipe that is taken, `logs\ipc-error.log`); for an error that has a Windows error code, that code, clamped to a byte (`--ipc-ping` says 5 for access denied). |
| 2 | The command line was refused. |
| 3 | A viewer probe never saw a window (no desktop). |

The probes and `--data-dir` exist for the tests and the performance harness
([perf harness](../../PERFORMANCE.md)); a user never needs them.
