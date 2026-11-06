---
name: Release checklist
about: The checklist a release candidate or a release has to pass (maintainer use).
title: "Release checklist: v"
labels: ["area:release", "quality"]
---

<!-- Every item needs an evidence link: a workflow run, a dashboard, a PR or a
screenshot. Nothing is ticked without one. Runbook: docs/how-to/release.md -->

## Product

- [ ] All MUST features of the roadmap (#1) are done, or moved to #78 with the owner's sign-off:
- [ ] Spikes closed with results in `docs/spikes/`; ADRs agree with the final outcomes:
- [ ] Runs unelevated, no network connection during a 10-minute end-to-end session, data dir and export dir self-excluded:

## Performance

- [ ] Every row in `perf/budgets.toml` is `calibrated`, not `provisional`:
- [ ] Deterministic rows green on `windows-2025` and `windows-11-vs2026-arm`:
- [ ] Timing rows within budget on the nightly trend for 7 days on both architectures:

## Correctness

- [ ] Zero-undetected-loss suite green nightly 7 days in a row:
- [ ] Durability, crash-recovery and migration tests green:
- [ ] UI suite green, including the UIA smoke on the release build:

## Quality gates

- [ ] Mutation results reviewed, no unjustified survivors:
- [ ] Coverage targets met:
- [ ] CodeQL and Scorecard without open high findings; `cargo audit` and cargo-deny clean; Miri and fuzz nightly green:

## Release artifacts

- [ ] README and release notes state the SmartScreen prompt and the Smart App Control block; the verification steps were tested on the release assets (hash and `gh attestation verify` pass, a tampered copy fails both):
- [ ] `release-verify` green on both runners for the published assets (checksum, attestation, `--diagnostics` smoke run, Defender scan, `cargo audit bin`):
- [ ] `SHA256SUMS`, attestations, SBOM (`peekbuddy-v<version>.cdx.json`) and `THIRD_PARTY_LICENSES.html` (inside each zip) present:
- [ ] `CHANGELOG.md` generated and reviewed; release notes list the known limitations (unsigned, UNC watching, Windows 10 best-effort, arm64 validated in CI only):

## Docs

- [ ] Tutorial, how-tos, reference and explanation pages current; README screenshots current:

## Launch

- [ ] Site listing ready:
- [ ] Announcement drafted:
- [ ] After release: winget submission made, Defender false positives reported, SmartScreen behaviour noted:
