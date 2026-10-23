# 0014. Welcome issues through forms; accept PRs only on accepted issues

- Status: Accepted
- Date: 2026-10-23
- Implemented or verified by: #7, #18

## Context and problem statement

Newer Buddies say "no outside PRs, issues or feature requests"; the portal
README says contributions are welcome. PeekBuddy needs one policy. Field
reports from real Windows setups (OneDrive, Defender, Smart App Control, odd
file systems) are the cheapest way to find capture problems CI cannot
reproduce.

## Considered options

| Option | Pros | Cons |
| --- | --- | --- |
| A. Hobby project: no outside PRs or issues | Zero triage | Loses field reports |
| B. Contributions welcome | Community reputation | Review load |
| C. Issues via forms (bug, perf regression with numbers, feature); PRs only for issues the owner accepted | Keeps field reports; bounded review load | Differs from siblings |

## Decision outcome

Chosen option C (best guesses B2, B3). PRs from the owner's own Claude Code
sessions count as owner PRs. The README follows the family shape and the
privacy statement lives in [PRIVACY.md](../../PRIVACY.md).

## Consequences

- Good: real-world bug and perf reports.
- Bad: issue forms, a PR template and CONTRIBUTING.md must exist (#18).

## Evidence

- Diataxis, checked 2026-10-23: <https://diataxis.fr/>
