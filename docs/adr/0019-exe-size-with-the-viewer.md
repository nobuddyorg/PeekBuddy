# 0019. Raise the exe size budget for the viewer

- Status: Accepted
- Date: 2026-11-03
- Implemented or verified by: #65, #28, #33

## Context and problem statement

PeekBuddy ships as one exe with an engine role and an on-demand viewer role
(ADR 0001). The size budget of the roadmap, a 5 MB target and an 8 MB cap
for the whole exe (`binary.size`, `binary.size.cap`, M14), was set before the
viewer renderer was measured. Spike #33 (report 0003) found the wgpu build
at 13.6 MB on x64 and 11.7 MB on ARM64 and the glow build at 6.45 MB, and
handed the trade-off to #65 and #28 (ADR 0005, "Consequences").

The viewer of #65 (eframe and egui 0.37.0, wgpu limited to DX12 with
`dx12`, `parking_lot`, `std` and `wgsl`, one embedded font) gives a release
exe of 11.47 MB on `windows-2025` with the release profile of #28.
`binary.size.cap` is `pr-strict`, so the required job `test (windows x64)`
fails with the viewer in the exe.

## Decision drivers

- One exe for engine and viewer: one download, one signature to verify
  (ADR 0001, ADR 0012).
- aarch64 has no OpenGL driver on Snapdragon X, so glow is not an option
  there (ADR 0005); a second renderer for x64 only does not shrink the arm64
  exe.
- A gate that is raised without a reason stops being a gate: the cap has to
  stay close enough to the real size that growth shows up in a PR.
- The size of the engine role in memory does not depend on the file size:
  the viewer code is not touched while the engine runs, and the GPU
  libraries are delay-loaded (report 0003, Q8; #65).

## Considered options

1. Keep 5 MB and 8 MB and split the viewer into a second exe.
2. Switch x64 to glow (6.45 MB) and keep wgpu for ARM64 only.
3. Raise the target to 12 MB and the cap to 14 MB for the single exe.
4. Shrink the viewer with `opt-level = "s"` for the GUI dependencies,
   `wgpu` feature trimming or a smaller font, then keep the old numbers.

## Decision outcome

Chosen option: "3", because options 1 and 2 give up decisions that are
recorded elsewhere (one exe, one renderer that works on both architectures)
and option 4 cannot be judged without measuring a build of each variant,
which needs Windows runners. The rows become:

| Row | Before | After |
| --- | --- | --- |
| `binary.size` (report-only) | 5 MB | 12 MB |
| `binary.size.cap` (`pr-strict`) | 8 MB | 14 MB |

The 12 MB target sits just above today's 11.47 MB so every further viewer
feature shows in the report, and the 14 MB cap leaves room for the rest of
the viewer (grid, filters, watch management, settings) without a second
loosening. Option 4 is a follow-up with numbers: each trial is one
`release-profile.yml` run, and a result that brings the exe under 10 MB
lowers the rows again.

## Consequences

- Good: the gate keeps failing a PR that adds more than about 2.5 MB; the
  viewer lands without a second exe or a second renderer.
- Bad: the download is about 11.5 MB instead of 5 MB, and the 5 MB claim of
  the roadmap no longer holds until option 4 is done.
- `PERFORMANCE.md` carries the new rows and a changelog line;
  `cargo xtask budgets` keeps the table and `perf/budgets.toml` in step.

## Evidence

- Report 0003, `docs/spikes/0003-viewer-renderer.md` (exe sizes, Q6 and Q8).
- Release exe of PR #323 on `windows-2025`: 11.47 MB (`cargo xtask size`,
  job `test (windows x64)`, 2026-11-03).
