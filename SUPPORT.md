# Supported platforms

| OS | Architecture | Status | CI-tested |
| --- | --- | --- | --- |
| Windows 11 | x64 | supported | yes, `windows-2025` (Server 2025, build 26100) |
| Windows 11 | arm64 | supported | yes, `windows-11-vs2026-arm` |
| Windows 10 22H2 (19045) | x64 | best-effort | no |
| Windows 10 before 22H2 | any | unsupported | no |
| Any 32-bit Windows | x86 | unsupported | no |
| macOS, Linux | any | unsupported | no |

Best-effort means: it compiles, newer APIs are gated at runtime, and bugs are
fixed on user reports. There is no Windows 10 test machine.

Windows 10 ESU dates are not used as policy input; the floor rests on API
availability and testability ([ADR 0013](docs/adr/0013-supported-windows-and-cpu-baseline.md)).

## CPU baseline

- x64: the default `x86_64-pc-windows-msvc` target with runtime SIMD detection.
  `x86-64-v2` is used only if a benchmark shows at least 5 % gain (#28).
- arm64: the default `aarch64-pc-windows-msvc` target.

## Runtime feature gates

| Feature | Needs | Fallback |
| --- | --- | --- |
| `ReadDirectoryNotifyFullInformation` | Windows 11 22H2+ | `ReadDirectoryNotifyExtendedInformation` (Windows 10 1709+), then `ReadDirectoryChangesW` |
| `DWMWA_USE_IMMERSIVE_DARK_MODE` (20) | documented 22000+, works on 19041+ | light title bar |
| `DWMWA_WINDOW_CORNER_PREFERENCE` (33) | 22000+ | default corners |
| `DWMWA_SYSTEMBACKDROP_TYPE` (38, Mica) | 22621+ | solid background |
| Tray-overflow first-run hint | Windows 11 | not shown |
| Unprivileged USN journal reading | per spike #31 | RDCW only |

`E_INVALIDARG` from a DWM attribute means "unsupported" and is never an error.
