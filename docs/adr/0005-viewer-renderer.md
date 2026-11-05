# 0005. Choose the viewer renderer by measurement

- Status: Accepted (x64 decided on report 0003, 2026-11-01)
- Date: 2026-10-22
- Implemented or verified by: #33, #65

## Context and problem statement

eframe can render through glow (OpenGL) or wgpu. Snapdragon Windows devices
have no native OpenGL driver, so arm64 cannot use glow. On x64 glow is
reported to need far less memory than wgpu, but that report is unverified.

## Decision drivers

- Viewer memory budget (80 MB with 10M rows, x64).
- One renderer is cheaper to test than two.
- arm64 must work without OpenGL.

## Considered options

1. glow on x64, wgpu (DX12 only) on arm64.
2. wgpu with DX12 only on both architectures.

## Decision outcome

Decided: arm64 always uses wgpu with the DX12 backend only (confirmed on
`windows-11-vs2026-arm`: wgpu/DX12 runs; glow fails with "egui_glow requires
opengl 2.0+" wherever only the GDI OpenGL 1.1 implementation exists, which is
the case on the hosted images and on Snapdragon without the Store pack).

Decided (2026-11-01, on the x64 run of report 0003, Intel N150): the viewer
starts on wgpu/DX12 on both architectures, because one renderer is cheaper to
build and test and the viewer skeleton (#65) must not wait for a second
backend. The measured gap is far beyond the 15 MB rule (wgpu uses about
260 MB more private bytes and the exe is 7 MB larger), so x64 moves to glow
once the viewer exists, behind a cargo feature, with arm64 staying on wgpu.
That switch is its own issue; until then the viewer memory budget is
calibrated on wgpu and tightened when glow lands. A second GPU and a cold-start
run sharpen the numbers but do not change the decision (#84).

Implemented in #65: the viewer builds its wgpu instance itself with
`Backends::DX12`, empty instance flags and default backend options, so
`WGPU_BACKEND`, `WGPU_POWER_PREF` and the other `WGPU_*` variables cannot
change the production renderer (eframe's defaults read them). The power
preference is `LowPower`, which picks the integrated GPU on a machine with
two. `crates/peekbuddy/tests/viewer.rs` runs the exe with `WGPU_BACKEND=vulkan`
and checks that the adapter is still DX12.

Implemented in #252: x64 renders through glow, arm64 through wgpu/DX12, as
above. The `peekbuddy-viewer` crate declares `eframe` once per architecture
(glow on x86_64, wgpu on aarch64), so neither build carries the other
renderer. The `wgpu` cargo feature (`peekbuddy/wgpu`) adds wgpu on x64 and
selects it, for A/B runs. `opengl32.dll` is delay-loaded like `dxgi.dll`, and
the engine-modules test still passes. Measured on the owner PC (Intel N150,
x64, release build, empty store, viewer idle after the first frame, 3 runs
each): glow 43 to 44 MB private bytes and first frame 195 to 362 ms; wgpu 298
to 301 MB and 424 to 1010 ms; exe 9.3 MB against 12.3 MB. With a 10M-row log
open (831 MB of segments; first page, idle, 3 runs each): glow 47 to 62 MB
private bytes, wgpu 302 to 303 MB. The page cache is bounded (12,800 rows). A
scripted run that pressed End, Home, Page Down and Page Up in the open
viewer (window messages, 2 runs each) peaked at 46 to 51 MB with glow and 301
MB with wgpu, so the limit is tightened from 80 to 64 MB.

Hosted `windows-2025` x64 runners have no OpenGL 2.0 driver (report 0003),
and a user can be in the same position (a Remote Desktop session without GPU
acceleration, a bare VM). The x64 exe therefore carries both renderers: it
starts with glow and, when glow cannot start (no OpenGL 2.0), starts again
with wgpu/DX12 in the same process; the reason goes to `logs/viewer.log`. Only
when both fail does the user see the error box. This costs exe size, not
memory: the release exe is 12.1 MB (glow only: 9.3 MB, wgpu only: 12.3 MB), and
wgpu is not initialised when glow works. CI exercises the fallback, because
its x64 runners cannot open glow, and the owner's PC exercises glow. The
`wgpu` cargo feature skips glow, for A/B runs.

## Consequences

- Good: arm64 works on devices without OpenGL.
- Bad: until the glow switch, x64 pays about 260 MB more private bytes; the
  exe is 13.6 MB with wgpu against 6.5 MB with glow, over the 5 MB target
  either way.
- The engine role of the single exe loads `dxgi`, `opengl32`, `glu32` and
  `dxcore` through static imports of the renderer crates (report 0003).

## Evidence

- eframe 0.37.0, checked 2026-10-22: <https://crates.io/crates/eframe/0.37.0>
- The 31 MB (glow) versus 142 MB (wgpu) RSS figures come from a single egui
  0.33/0.34 report on unknown hardware: **still unverified**; the hosted
  runners measured wgpu on WARP only (working set 55 to 69 MB, private bytes 295
  to 304 MB). Spike #33, report `docs/spikes/0003-viewer-renderer.md`.
