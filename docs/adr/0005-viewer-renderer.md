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
