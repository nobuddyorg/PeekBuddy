# 0005. Choose the viewer renderer by measurement

- Status: Proposed (until spike #33 reports)
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

Proposed (F9): arm64 always uses wgpu with the DX12 backend only. On x64, unify
on wgpu/DX12 if spike #33 measures it within about 15 MB of glow; otherwise
use glow on x64.

## Consequences

- Good: arm64 works on devices without OpenGL.
- Bad: until #33 reports, two configurations stay possible.

## Evidence

- eframe 0.37.0, checked 2026-10-22: <https://crates.io/crates/eframe/0.37.0>
- The 31 MB (glow) versus 142 MB (wgpu) RSS figures come from a single egui
  0.33/0.34 report on unknown hardware: **unverified** until #33 measures.
