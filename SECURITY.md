# Security policy

## Reporting a vulnerability

Report vulnerabilities privately through
[GitHub private vulnerability reporting](https://github.com/nobuddyorg/PeekBuddy/security/advisories/new).
Please do not open a public issue.

Include:

- the PeekBuddy version and the Windows version and architecture;
- what an attacker can do, and what they need first (for example "another
  local user account" or "a crafted file name in a watched folder");
- steps to reproduce, ideally with a minimal folder layout or file name;
- whether the issue is already public.

You get a first answer within 7 days. A confirmed issue is fixed in the next
release, credited in the release notes unless you prefer otherwise, and
published as a GitHub security advisory.

## Supported versions

Only the latest release receives fixes. PeekBuddy is not released yet.

## Scope

What PeekBuddy defends and what it does not is described in the
[threat model](docs/explanation/threat-model.md). An attacker who already runs
code as the same Windows user is out of scope.
