# 0003. Viewer renderer and footprint

- Issue: #33. Status: hosted-runner cells done (WARP, x64 and ARM64) and the
  owner-PC run done on one machine (Intel N150 iGPU, x64); cold start, VRAM
  and ARM64 hardware are still open.
- Harness: `spikes/viewer-bench` (eframe =0.37.0, egui_table =0.11.0), workflow
  `viewer-bench` (input `quick` for a 3-minute smoke run).
- Raw data: `docs/spikes/data/0003/` (one JSON line per run; the workflow log
  carries the same lines between `VIEWER-BENCH-BEGIN` and `-END`).
- Runs: 40181826845 (full, `windows-2025` Server 2025 26100 and
  `windows-11-vs2026-arm` Windows 11 26200) and 40182593944 (glow exit code).
- Every cell renders with WARP ("Microsoft Basic Render Driver", Dx12, Cpu):
  the runners have no GPU. Numbers that depend on the rasterizer (frame
  intervals, CPU time, wgpu private bytes) are informational only (F11, F12).

## What the hosted runners can answer

| Question | Answer |
| --- | --- |
| Q1 x64 private bytes glow vs wgpu | **not measurable here**: glow cannot create a context on the runners (below). wgpu/DX12/WARP: 278 MB at the first frame, 295 MB after the page, 304 MB after a live tail |
| Q2 first frame | wgpu x64 WARP 90 ms (4 of 5 runs, 274 ms for the first); ARM64 WARP 150 to 168 ms (414 ms for the first). Inside the warm budget (250 ms). Cold start and a real GPU: not measured |
| Q3 frame time at 10M rows | egui side (the `ui` pass) p50 0.4 to 1.3 ms, p99 0.8 to 2.0 ms, max 4.5 ms (x64) / 12.5 ms (ARM64) over 5 s to 30 s scrolls and a 5k rows/s tail; the 200-row page fetch (0 to 5 ms latency) does not show. Frame intervals (63 ms) are the WARP rasterizer, not the UI |
| Q4 pointer wiggle | UI p99 0.8 ms (x64), 0.4 ms (ARM64) and max 26 ms (x64, once); every move repaints (egui#8695), the hover-gate question needs a real GPU where the repaint cost is the rasterizer's |
| Q5 wakeups idle/minimized/hidden | 1.0 / 0.1 / 0.5 frames/s per phase over 10 s phases on x64 (0.9 / 0.1 / 0.3 on ARM64). The frames in the minimized and hidden phases are the harness's own phase timers; eframe ran no extra pass. ETW context-switch counts were not collected |
| Q6 ARM64 | wgpu/DX12 works on `windows-11-vs2026-arm` (WARP): same footprint as x64 (private bytes 280 to 303 MB, working set 55 to 69 MB, exe 11.7 MB). Real Adreno numbers: known gap |
| Q7 table widget | `egui_table` 0.11.0 with 10M rows and fixed row height: UI pass p99 <= 2.0 ms on both architectures (budget 16 ms). No fallback needed on these numbers |
| Q8 engine-role modules | **yes, the GPU libraries load.** The wgpu binary in the engine role (no window) loads `dxgi.dll`, `opengl32.dll`, `glu32.dll` and `dxcore.dll`; the glow binary `opengl32.dll`, `glu32.dll` and `dxcore.dll`. `d3d12.dll`, `d3d10warp.dll` and the shader compiler load only when a device is created. Private bytes of the engine role: 1.9 MB (wgpu) and 1.7 MB (glow) |

## Glow on the hosted runners

Every `first-frame` run of the glow build exits with code 1 and
`OpenGL(PainterError("egui_glow requires opengl 2.0+."))`: the hosted
Windows images offer only the GDI generic OpenGL 1.1 implementation. The
same failure is the reason arm64 cannot use glow. Consequently the x64
comparison of the issue (private bytes within about 15 MB of glow) cannot be
evaluated on a runner. The glow executable is 6.45 MB against 13.6 MB for wgpu on
x64 (11.7 MB on ARM64), a difference of 7.2 MB, which already uses the
single-exe budget (5 MB target, 8 MB cap) with the viewer alone.

## Memory

The wgpu build is at 278 MB private bytes when the first frame is drawn
(measured after eframe has created the device), 295 MB with the first page of
rows and at most 304 MB after scrolling 10M rows. The 80 MB viewer budget is
therefore not reachable with wgpu *on WARP*. WARP allocates its own heap, so
this says nothing about a real adapter; the budget stays provisional until the
owner run measures wgpu on a GPU. Working set stays at 55 to 69 MB.

## Owner PC (Intel N150, real GPU)

Machine: Windows 11 Pro 26200 x64, Intel N150 with the integrated "Intel(R)
Graphics" adapter (wgpu reports `Dx12 / IntegratedGpu`), run with
`scripts/owner/viewer-bench.ps1` on the executables of run 40183465603. Raw
data: `docs/spikes/data/0003/owner-n150.jsonl`. This is a low-end iGPU: treat
CPU time and first frame as a worst case for x64, not as the typical owner PC.
One machine, one run; the five first-frame runs are not warmed up in a
controlled way, so cold and warm are not separated.

| Metric | wgpu/DX12 | glow |
| --- | --- | --- |
| Exe size | 13.6 MB | 6.45 MB |
| Private bytes, first frame | 317 MB | 51 to 58 MB |
| Private bytes after scroll / tail / wiggle | 317 to 319 MB | 52 to 62 MB |
| Working set | 327 to 336 MB | 92 to 103 MB |
| First frame (5 runs) | 435, 437, 512, 522, 619 ms | 243, 250, 270, 362, 745 ms |
| First frame with data | 442 to 692 ms | 250 to 832 ms |
| UI pass p95 / max (scroll, tail) | 2.7 to 2.8 / 5.5 to 6.1 ms | 2.3 / 4.4 to 4.9 ms |
| Frame interval p95 / p99 / max (scroll) | 20.5 / 21.0 / 74 ms | 20.3 / 20.6 / 48 ms |
| Frame interval max (tail, wiggle) | 87 ms, 85 ms | 73 ms, 60 ms |
| CPU time, 30 s scroll | 6.0 s | 9.3 s |
| CPU time, 30 s tail / 20 s wiggle | 4.8 s / 2.5 s | 8.5 s / 4.8 s |
| Idle frames/s open / minimized / hidden | 1.19 / 0.10 / 0.30 | 1.09 / 0.10 / 0.30 |
| Engine-role private bytes | 1.8 MB | 1.6 MB |

Reading:

- The unify rule (wgpu within about 15 MB private bytes of glow) fails on
  this machine by a wide margin: about 260 MB. How much of the 317 MB is
  Intel user-mode driver and shader cache rather than wgpu itself was not
  separated (the module list shows `igd12umd64`, `igc64`, `d3d12core` and
  more). The same 80 MB budget problem as on WARP.
- wgpu first frame is above the 250 ms warm budget here (435 to 619 ms);
  glow mostly meets it (243 to 362 ms, one 745 ms outlier).
- The frame interval sits at about 20 ms for both renderers and every
  scenario, including `wiggle`, while the UI pass stays at 2 to
  3 ms. That looks like display or compositor pacing and not rendering cost;
  it was not diagnosed (refresh rate of the attached display not recorded).
  Max values of 48 to 87 ms are over the 33 ms limit in every scroll, tail
  and wiggle scenario; whether they come from the harness or the driver is
  open.
- The 5 ms page-fetch latency does not change the scroll frame times
  (p95 20.6 ms).
- Glow costs more CPU than wgpu in every animated scenario (scroll 9.3 s
  against 6.0 s), the opposite of the footprint ranking.
- Engine role: the same GPU libraries as on the runners load (`dxgi.dll`,
  `opengl32.dll`, `glu32.dll`, `dxcore.dll`), confirming decision 4.

## Decisions

1. **arm64: wgpu with the DX12 backend only.** Decided (glow cannot run there);
   ADR 0005.
2. **x64: unify rule not met, owner to decide.** The owner-PC run gives the
   glow number: about 260 MB less private bytes and 7 MB less exe size than
   wgpu, but more CPU and no ARM64 support. The code base keeps both cargo
   features (`wgpu` default, `glow` x64 only) and `#65` implements the viewer
   against the wgpu path first. Proposed for the owner to veto: ship glow on
   x64 and wgpu on ARM64 only if the 260 MB is confirmed as wgpu cost on a
   second GPU (for example a discrete or AMD adapter); otherwise unify on
   wgpu and recalibrate the memory budget.
3. **Table widget: `egui_table`**, no `TableBuilder` fallback and no
   `LVS_OWNERDATA` escape hatch (the UI pass is two orders of magnitude below
   the frame budget; the escape hatch triggers only on a real-GPU miss).
4. **Engine role loads GPU libraries**: F1 (an engine with no GPU footprint)
   is violated at the module level by a single exe with static imports of
   `dxgi`/`opengl32`/`glu32`/`dxcore`. The private-byte cost is small (1.9 MB)
   but the modules appear in the engine's process. #59/#65 either
   delay-load them (`/DELAYLOAD`) or accept it; to be decided with the engine
   skeleton.

## Budgets

| Row | Measurement | Action |
| --- | --- | --- |
| `viewer.first_frame.warm` (250 ms) | 90 ms x64, 160 ms ARM64 (WARP, 10M-row source, first 200 rows); owner N150: wgpu 435 to 619 ms, glow 243 to 362 ms (one 745 ms) | stays provisional; wgpu misses it on a low-end iGPU, recalibrate once a second GPU is measured |
| `viewer.first_frame.cold` (600 ms) | not measured | unchanged, owner PC only |
| `viewer.private_bytes.10m_rows` (80 MB) | 295 to 304 MB on WARP; owner N150: wgpu 317 to 319 MB, glow 51 to 62 MB | stays provisional; the ARM64 limit from #33 cannot be derived without an Adreno device |
| `viewer.frame.p95/p99/max` | UI pass only: p95 <= 1.5 ms, p99 <= 2.0 ms, max 12.5 ms on WARP; owner N150: UI p95 2.3 to 2.8 ms, frame interval p95 20.5 ms, max 48 to 87 ms | stays provisional; the interval is about 20 ms for both renderers and its cause is undiagnosed |
| exe size | wgpu 13.6 MB (x64), 11.7 MB (ARM64); glow 6.45 MB (x64) | over the 5 MB / 8 MB budget for the single exe with only the viewer in it; #28 and #65 own the trade-off |

## Not measured

- Cold start (first run after boot or after clearing the shader cache) and
  VRAM on the owner PC; a second, stronger GPU; the display refresh rate
  behind the 20 ms frame interval; real Adreno hardware.
- ETW context-switch counts per phase, the steady-state allocations per frame
  (the K constant for the viewer, #23), Tracy frame marks.
- The hover-gate decision (needs a GPU adapter where a repaint costs the GPU).

## Harness notes

- `scripts/owner/viewer-bench.ps1` crashed on Windows PowerShell 5.1 when the
  temp path had a space (unquoted `Start-Process` arguments, then a null
  `ExitCode` and a `Substring` on empty stderr). Fixed; the run above used the
  fixed script.
- eframe repaints only on events: the `idle` and `first-frame` scenarios need
  a helper thread that wakes the loop, and a watchdog per run (the first run
  of this spike hung for 52 minutes without a result row).
- Hosted runners print results into the job log; the artifact host is not
  reachable from the agent containers.
