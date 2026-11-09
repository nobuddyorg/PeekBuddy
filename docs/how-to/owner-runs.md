# Owner runs

Some checks need real hardware: the owner's Windows 11 x64 PC (owner answer
O4). Agents never hand the owner open-ended manual work; they hand over a
script.

## Conventions for `scripts/owner/*.ps1`

- Runs in Windows PowerShell 5.1 or PowerShell 7 without a Rust toolchain.
- Finishes in at most 15 minutes.
- No elevation unless a step needs it, and then the script says why at the
  top. No persistent changes to the machine.
- Writes one JSON report (`<script>-report.json` next to the script by
  default, `-OutFile` to change it) with `script` and `schemaVersion` fields.
- Unsigned builds need Smart App Control off (best guess B9); scripts that
  run `peekbuddy.exe` check this first and stop with a clear message.

## How a run works

1. The PR that needs a run gets the `needs-owner-run` label and names the
   script in its description.
2. The owner runs it, for example:

   ```powershell
   powershell -ExecutionPolicy Bypass -File scripts\owner\hello.ps1
   ```

3. The owner attaches the JSON report to the PR.

Runs are batched: at most one owner run per week unless a release is due.

## Scripts

| Script | What it reports | Used by |
| --- | --- | --- |
| [`hello.ps1`](../../scripts/owner/hello.ps1) | OS, build, architecture, CPU, memory, elevation, Smart App Control state | #86 (example) |
| [`viewer-bench.ps1`](../../scripts/owner/viewer-bench.ps1) | First frame, private bytes, frame times and idle wakeups of the egui viewer with wgpu/DX12 and glow on the real GPU (about 8 minutes; needs the `viewer-bench-exes-*` artifact of the viewer-bench workflow) | #33 |
| [`rdcw-bench.ps1`](../../scripts/owner/rdcw-bench.ps1) | ReadDirectoryChangesExW class and buffer matrix, root lifecycle, throughput with 5 repetitions, burst and pool cost on the owner's local NTFS disk, plus file-name length aggregates (about 15 minutes, unelevated; needs the `rdcw-bench-exe-windows-2025` artifact of the rdcw-bench workflow) | #32 |
| [`store-bench.ps1`](../../scripts/owner/store-bench.ps1) | Commit policy, checkpoint, readers, memory, size and 2M-row query numbers of the SQLite store on the owner's disk (about 15 minutes; needs the `store-bench-exes-windows-2025` artifact of the store-bench workflow) | #35 |
| [`autostart-check.ps1`](../../scripts/owner/autostart-check.ps1) | In three phases around a real sign-in: the Run value and Task Manager's StartupApproved flag, whether the autostarted engine runs with no viewer child, its CPU time and bytes read, and the Startup impact you read from Task Manager (about 10 minutes plus the sign-out; the only persistent change is the Run value, removed by `-Phase cleanup`) | #63 |
| [`perf-baremetal.ps1`](../../scripts/owner/perf-baremetal.ps1) | The absolute numbers of a release build on real hardware: a preflight (AC power, power plan, GPU and driver, Defender, busiest processes), then the perf harness (engine ready warm, startup CPU and disk read, idle CPU and wakeups, Private Bytes; with `-Driver` ingest, event latency, handle stability and viewer frame times over 1M rows) and the viewer's first frame, 3 runs each, medians and spread (about 12 minutes, unelevated; takes `-Exe`, `-Harness` and `-Driver`, see below) | #84 |

### Running `perf-baremetal.ps1`

1. Download the release `peekbuddy.exe`, and `xtask.exe` and
   `perf-driver.exe` from the `peekbuddy-perf-harness` artifact of the
   latest `perf-harness` run on main. Check all three
   ([verify a download](verify-download.md); `gh attestation verify <file>
   --repo nobuddyorg/PeekBuddy`).
2. Close other programs, plug the PC in, and run:

   ```powershell
   powershell -ExecutionPolicy Bypass -File scripts\owner\perf-baremetal.ps1 -Exe .\peekbuddy.exe -Harness .\xtask.exe -Driver .\perf-driver.exe
   ```

   Without `-Driver` the ingest, event latency and handle scenarios are
   skipped and named under `notMeasured`.

3. The script writes `perf-baremetal-<version>-<date>.json`. Attach it to the
   release checklist issue (#77), or commit it to `perf/owner-runs/`: the
   tests validate every file there against
   [`perf/owner-report.schema.json`](../../perf/owner-report.schema.json).

An everyday PC is noisy. Compare a run only with earlier runs on the same
machine; the preflight sits next to every number and its warnings (battery,
a busy process, power saver) say when a run is not comparable. What the
script does not measure yet is listed under `notMeasured` in the report.

A report committed to `perf/owner-runs/` on main is published by the
`owner-pc` workflow as the "owner-pc" series on gh-pages, one point per
budget row. Keep the default `-IdleSecs 60` for such a report: with a shorter
idle window a single timer wake rounds up to several wakes per minute and
fails `engine.idle.wakes`.
