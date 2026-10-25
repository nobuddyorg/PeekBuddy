# 0005. Choose the viewer renderer by measurement

- Status: Accepted for arm64; x64 proposed (spike #33 reported, one owner run left)
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

Proposed: on x64, unify on wgpu/DX12 if the owner run measures it within about
15 MB of glow; otherwise use glow on x64. The hosted runners cannot decide it
(no OpenGL 2.0), and wgpu on WARP shows 295 to 304 MB private bytes, which
says nothing about a real adapter. See report 0003.

## Consequences

- Good: arm64 works on devices without OpenGL.
- Bad: until the owner run, two x64 configurations stay possible; the exe is
  13.6 MB with wgpu against 6.5 MB with glow, over the 5 MB target either way.
- The engine role of the single exe loads `dxgi`, `opengl32`, `glu32` and
  `dxcore` through static imports of the renderer crates (report 0003).

## Evidence

- eframe 0.37.0, checked 2026-10-22: <https://crates.io/crates/eframe/0.37.0>
- The 31 MB (glow) versus 142 MB (wgpu) RSS figures come from a single egui
  0.33/0.34 report on unknown hardware: **still unverified**; the hosted
  runners measured wgpu on WARP only (working set 55 to 69 MB, private bytes 295
  to 304 MB). Spike #33, report `docs/spikes/0003-viewer-renderer.md`.
