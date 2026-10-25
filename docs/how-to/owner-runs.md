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
