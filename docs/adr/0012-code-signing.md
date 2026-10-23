# 0012. Ship v1 unsigned; later use only free signing

- Status: Accepted
- Date: 2026-10-23
- Implemented or verified by: #6, #74, #85, #87, #77

## Context and problem statement

On Windows 11 with Smart App Control (SAC) enforcing, an unsigned exe without
cloud reputation is blocked, with no per-app bypass. A binary signed with an
RSA certificate from a CA in the Microsoft Trusted Root Program is allowed.
Signing costs money or depends on a foundation's discretion.

## Decision drivers

- Hobby project: never pay for signing (owner answer O6).
- Downloads must still be verifiable.

## Considered options

| Option | Cost | Status |
| --- | --- | --- |
| SignPath Foundation | free, discretionary | later path (#74) |
| Microsoft Store (MSIX, signed by the Store) | free | later, evaluated in #85 |
| Certum Open Source | from EUR 49/yr | excluded (O6) |
| Azure Artifact Signing | about USD 9.99/month | excluded (O6) |
| Commercial OV | USD 150-300/yr | excluded (O6) |
| EV | USD 400+/yr | excluded (O6) |
| Ship unsigned | free | v1 (O3) |

## Decision outcome

v1 ships unsigned (owner answer O3). README and release notes say plainly that
SAC blocks the exe and SmartScreen warns; downloads are verifiable through
`SHA256SUMS` and GitHub artifact attestations (#87).

After v1.0, once a few public releases exist, apply to SignPath Foundation for
the GitHub builds (#74) and evaluate a Microsoft Store MSIX channel (#85)
(best guess B4). If SignPath rejects, re-apply later. No paid fallback, ever.

Engineering rules for any future signing: sign every shipped PE, RSA-2048+ /
SHA-256, RFC 3161 timestamp, never change a file after signing, one publisher
identity, `signtool verify /pa /v` as a CI gate.

## Consequences

- Good: zero cost.
- Bad: SAC-enforcing machines cannot run v1 until signing lands; users must
  switch SAC off (reversible since KB5083769) or wait.

## Evidence

- SAC code signing, checked 2026-10-23: <https://github.com/MicrosoftDocs/windows-dev-docs/blob/docs/hub/apps/develop/smart-app-control/code-signing-for-smart-app-control.md>
- SignPath Foundation terms, checked 2026-10-23: <https://github.com/SignPath/fdn-website/blob/main/docs/terms.md>
- Free Microsoft Store registration, checked 2026-10-23: <https://blogs.windows.com/windowsdeveloper/2025/09/10/free-developer-registration-for-individual-developers-on-microsoft-store/>
