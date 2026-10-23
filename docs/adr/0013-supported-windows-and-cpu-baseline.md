# 0013. Support Windows 11; Windows 10 22H2 best-effort

- Status: Accepted
- Date: 2026-10-23
- Implemented or verified by: #5, #28, #59

## Context and problem statement

The OS floor decides which APIs may be called unconditionally, what the
manifest declares and what CI can test. Sources disagree on Windows 10
consumer ESU dates, so the policy rests on API availability and testability,
not on lifecycle dates.

## Decision drivers

- Every needed API exists on Windows 10 22H2 (19045).
- No Windows 10 test machine (owner answer O4); no hosted Windows 10 runner.

## Considered options

| Option | Pros | Cons |
| --- | --- | --- |
| A. Windows 11 only | Smallest matrix | Excludes Win10 22H2 users although APIs exist |
| B. Win11 supported (x64 + arm64, CI-tested) + Win10 22H2 best-effort | All APIs exist on 19045; Win11 features gated at runtime | Win10 bugs fixed best-effort |
| C. Win10 1709+ | Widest reach | Untestable; manifest features missing |

## Decision outcome

Chosen option B (best guess B1, F13). No 32-bit.

- **CPU baseline**: default `x86_64-pc-windows-msvc` target with runtime SIMD
  detection (`is_x86_feature_detected!`); raise to `-Ctarget-cpu=x86-64-v2`
  only if a benchmark in #28 shows at least 5 % gain. Windows 11 24H2 needs
  POPCNT/SSE4.2 but Windows 10 does not, so x86-64-v2 is not a safe floor.
  arm64: default `aarch64-pc-windows-msvc`.
- **Manifest** (embed-manifest 1.5.1, #59): `supportedOS` Windows 10/11
  `{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}`, `maxversiontested`, PerMonitorV2,
  longPathAware, UTF-8 active code page, asInvoker.
- **Runtime gates**: listed in [SUPPORT.md](../../SUPPORT.md).

## Consequences

- Good: no extra CI; Windows 10 users are not locked out.
- Bad: Windows 10 regressions are found by users.

## Evidence

- ReadDirectoryChangesExW, checked 2026-10-23: <https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-readdirectorychangesexw>
- DWMWINDOWATTRIBUTE, checked 2026-10-23: <https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/ne-dwmapi-dwmwindowattribute>
- embed-manifest 1.5.1, checked 2026-10-23: <https://crates.io/crates/embed-manifest/1.5.1>
