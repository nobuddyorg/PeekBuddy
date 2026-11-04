# Release

This is the maintainer's runbook. The automated pipeline (release PR, tag,
attested builds; issue #73) is not in the repository yet; until it lands this
page holds the checklist every release has to pass, and the steps to check an
unsigned build.

## Release checklist

Open the issue from the *Release checklist* template
(`.github/ISSUE_TEMPLATE/release_checklist.md`) for every release candidate and
release. Each item needs an evidence link: a workflow run, a dashboard, a PR or
a screenshot. Nothing is ticked without one.

The checklist covers, in order:

1. **Product**: the MUST features of the roadmap are done, spikes and ADRs
   agree with what shipped, the exe runs unelevated and opens no network
   connection during a 10-minute end-to-end session.
2. **Performance**: every row in `perf/budgets.toml` is calibrated; the
   deterministic rows are green on both runners; the timing rows are within
   budget on the nightly trend for 7 days on both architectures. A single PR
   wall-clock run is never evidence (ADR 0007).
3. **Correctness**: the zero-undetected-loss suite, durability and migration
   tests, and the UI suite with the UIA smoke on the release build are green.
4. **Quality gates**: mutation results reviewed, coverage targets met, CodeQL
   and Scorecard clean, `cargo audit` and cargo-deny clean, Miri and fuzzing
   green nightly.
5. **Release artifacts**: `SHA256SUMS`, attestations, SBOM, third-party
   licences and PDBs present; the release notes carry the *unsigned build*
   note; the owner's release smoke (below) is attached.
6. **Docs**: the tutorial, how-tos, reference and explanation pages are
   current; the README has current screenshots and the *Unsigned builds*
   section.
7. **Launch**: the site listing is ready, the announcement is drafted, and
   after the release is public the winget submission is made and any Defender
   false positive is reported.

## Owner release smoke

On the owner's Windows 11 x64 PC, with the release exe: the tray icon, the
viewer, autostart on and off, an export, the startup impact in Task Manager,
and a Defender scan of the release assets:

```powershell
& "$env:ProgramFiles\Windows Defender\MpCmdRun.exe" -Scan -ScanType 3 -File .\peekbuddy-<version>-x86_64-pc-windows-msvc.zip
```

If anything is flagged, submit it to Microsoft Security Intelligence
(<https://www.microsoft.com/en-us/wdsi/filesubmission>) before announcing the
release, and link the submission in the checklist.

## Checking an unsigned build

Follow [Verify a download](verify-download.md) on the release assets, and try a
tampered copy: both the checksum and the attestation must fail on it. Never
re-upload an asset under the same version: SmartScreen's reputation is per file
hash, and a replaced asset breaks every checksum already published. A fix is a
new patch version.
