# 0004. GitHub-hosted runner capability probe

- Issue: #34. Status: complete for the checks below on `windows-2025` and
  `windows-11-vs2026-arm`; the UIA, `egui_kittest` and private-runner rows are
  not measured.
- Harness: `spikes/runner-probe` (Rust probe and `probe.ps1`), workflow
  `runner-probe`.
- Raw data: `docs/spikes/data/0004/` (one JSON object per check, from run
  40227851713, the first run where every step passes).
- Runs on three different days: 39934653303, 39954189939 and 39962290020
  (23 Oct), 40128643899 (24 Oct), 40186273096 (25 Oct, morning) and
  40227851713 (25 Oct, evening). The first five ended red; for 40186273096 I verified that the only
  failing step was the hyperfine check on hyperfine 2.0.0 output (below), and
  the issue comments record the other checks of the earlier runs. The data file holds the last
  run only, and the ARM64 column below was read from that run alone.

## Environment

| | `windows-2025` | `windows-11-vs2026-arm` |
| --- | --- | --- |
| Image | `win25-vs2026/20261021.278.1` | `win11-vs2026-arm64/20261022.194.1` |
| Build | 26100 | 26200 |
| CPU | Intel Xeon Platinum 8573C (run 40227851713); AMD EPYC 9V45 in run 40186273096 | Cobalt 100 |
| Logical CPUs, RAM | 4, 16 GB | 4, 16 GB |
| Free on `C:` | 25.8 GB | 123.6 GB |
| Time zone | UTC | **Pacific Standard Time** |
| Defender real-time / tamper | **off** / off, 2 exclusions | **on** / on, 2 exclusions |
| Drive `D:` | present, no USN journal (error 1179) | not present |

The x64 CPU model changes between runs of the same label (AMD in one run,
Intel in the next), so any threshold in a performance gate must come from the
same-runner A/B (F11), not from absolute numbers.

## What the runners can do

| Q | Check | `windows-2025` | `windows-11-vs2026-arm` |
| --- | --- | --- | --- |
| 1 | Session | session 2 on the console, window station visible (`dwFlags=0x1`), `explorer.exe` running | same |
| 2 | Tray `Shell_NotifyIconW(NIM_ADD)` | works, `TaskbarCreated` registered | **fails** with `E_FAIL` (0x80004005); `TaskbarCreated` registers |
| 3 | Visible window, `SetForegroundWindow` | both work | window shows, `SetForegroundWindow` **fails** |
| 3 | UIA and `egui_kittest` snapshot | not measured | not measured |
| 4 | Display | 1024x768 by default; `Set-DisplayResolution 1920 1080` works | 1024x768; the cmdlet is **absent** |
| 5 | Service | `sc.exe create`, `query`, `delete` work; `start` fails with 1053 because the test binary is not a service | same |
| 5 | Named pipe | `FILE_FLAG_FIRST_PIPE_INSTANCE` rejects a second instance, a client connects | same |
| 6 | VHD with NTFS, ReFS, FAT32 and exFAT, drive letter assigned | all work (ReFS needs at least 4 GiB, 1 GiB failed) | all work |
| 6 | `fsutil usn queryjournal` | `C:` active, `D:` not active | `C:` active |
| 6 | SMB loopback share | works | works |
| 7 | Token | elevation type 1, integrity high (0x3000) | same |
| 7 | Standard local user via `CreateProcessWithLogonW` | works, medium integrity | same |
| 8 | Defender | real-time off | real-time **on**, tamper protection on |
| 10 | `wpr` 10 s CPU and context-switch trace | works, 83 MB | works, 263 MB |
| 11 | Environment | see table above | see table above |
| 12 | `cargo-nextest` 0.9.149, `cargo-deny` 0.20.2, `cargo-mutants` 27.1.0, `hyperfine` 2.0.0 through `install-action` | all install | all install (`aarch64`) |
| 13 | Fixed CPU loop, 30 runs | mean 2.8 ms, CV 0.52 % | mean 1.5 ms, CV **11.1 %** |
| 13 | `hyperfine -N` of a trivial exe | mean 10 ms, CV 5.4 % | mean 13 ms, CV 3.5 % |

GPU (Q9) is answered in report 0003: every cell renders on WARP, there is no
GPU on either runner.

### Unelevated children (needed by #31 and #13)

Same on both runners.

| Child token | Result |
| --- | --- |
| `DISABLE_MAX_PRIVILEGE` only (still high integrity, admin) | works |
| `+ LUA_TOKEN` | fails, `0xC0000142` |
| Administrators deny-only | fails, `0xC0000142` |
| Medium integrity only | works |
| deny-only + medium (inherited or empty `lpDesktop`) | fails, `0xC0000142` |
| **deny-only + medium + `DETACHED_PROCESS`** (`cmd.exe` and a Rust exe) | **works** |
| Standard local user through `CreateProcessWithLogonW` | works, medium integrity |

The blocker is the console, not the desktop: a child whose Administrators
group is deny-only must be created with `DETACHED_PROCESS`. `peekbuddy-win`
reintroduces the unelevated-child helper with that recipe.

## What follows for the code base

- **Tray, foreground and UI tests on ARM64 will not pass on the hosted
  runner.** `NIM_ADD` and `SetForegroundWindow` fail there. Tests that need the
  tray (#60) or the foreground window must be skipped or mocked behind the
  `peekbuddy-win` traits on ARM64, with the reason recorded (never silently
  skipped, see AGENTS.md); x64 covers them. Real ARM64 hardware is a known gap
  (owner answer O4).
- **Fix display size in tests, not through the cmdlet:** x64 can switch to
  1920x1080, ARM64 cannot. Render tests must not depend on the resolution.
- **Performance gates need the same-runner A/B.** The x64 CPU model changes
  between runs, and the ARM64 CPU loop has a CV above 10 %; use paired runs on
  one runner as F11 says and a wider threshold on ARM64.
- **Defender differs between the runners:** off on x64, on on ARM64. Any I/O
  number from a hosted runner states which one it came from (report 0005
  already does).
- **Time zone:** ARM64 is Pacific, x64 UTC. Tests that format local time must
  not assume either (actions/runner-images#15007 is the known issue).
- **Services:** a real service test needs a real service binary; `sc.exe`
  create, query and delete are enough for the installer tests.
- **Disks:** `diskpart` VHDs work for all four file systems, ReFS at 4 GiB or
  more. `D:` exists only on x64 and has no USN journal; create one with
  `fsutil usn createjournal`.

## Harness note: hyperfine 2.0.0

The probe's hyperfine check failed in every run before the last one for two
reasons, both fixed in `probe.ps1`:

- `hyperfine -N` splits the command with shell-word rules, so a path with
  backslashes (`C:\Windows\System32\whoami.exe`) becomes a program that does
  not exist; use the bare name.
- The JSON export is `schema_version` 2: the statistics are under
  `results[0].summary.time_wall_clock`, not `results[0].mean`.

The script also ended with hyperfine's exit code, which turned a failed check
into a failed step; it now ends with `exit 0`, because a probe reports.

## Not measured

- UIA (`uiautomation` crate) finding a window, and an `egui_kittest` snapshot.
- Private runners (2 vCPU / 8 GB, F14): only the public runners were probed.
- Whether the ARM64 tray failure is the image or the missing notification area
  of the session; not diagnosed.
