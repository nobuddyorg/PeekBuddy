# 0011. Make the repository public under MIT before CI lands

- Status: Accepted
- Date: 2026-10-23
- Implemented or verified by: #4, #11, #16, #74

## Context and problem statement

The CI, perf-calibration, security-scanning and signing plans assume
public-repository resources. Runner size changes every timing calibration, so
visibility had to be settled before CI produced its first baseline.

## Decision drivers

- Free 4 vCPU / 16 GB Windows x64 and arm64 runners.
- Free CodeQL and SARIF upload.
- SignPath Foundation needs a public repository with visible reputation.

## Considered options

| Option | Pros | Cons | Cost/effort |
| --- | --- | --- | --- |
| A. Make public now (before CI lands) | 4 vCPU / 16 GB / 14 GB SSD Windows x64 and arm64 runners, free; CodeQL and zizmor SARIF upload free; SignPath eligibility; matches the other Buddies | Work visible from the first commit; hygiene files needed first | about 1 h checklist |
| B. Public at first release | Polish before exposure | Perf baselines recalibrated after the flip; code scanning unavailable until then; SignPath clock starts late | Recalibration of #26/#27 |
| C. Stay private | Nothing visible | 2 vCPU / 8 GB runners; arm64 billed; code scanning paid; no SignPath | Ongoing minutes cost |

## Decision outcome

Chosen option A (owner answer O2, 2026-10-23). The repository is public since
2026-10-23 under the MIT licence.

Checklist: `LICENSE` (MIT), `README.md` stub with the privacy statement,
`SECURITY.md`, secret scanning after the flip, a ruleset on `main` (owner),
Dependabot alerts and private vulnerability reporting (owner).

## Consequences

- Good: all wall-clock baselines assume a 4 vCPU / 16 GB runner.
- Bad: changing visibility later forces recalibration (ADR 0007).

## Evidence

- GitHub-hosted runner specs, checked 2026-10-22: <https://docs.github.com/en/actions/using-github-hosted-runners/about-github-hosted-runners>
- SignPath Foundation terms, checked 2026-10-22: <https://github.com/SignPath/fdn-website/blob/main/docs/terms.md>
