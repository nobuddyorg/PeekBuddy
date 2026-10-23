# Dependency policy

The rules behind every dependency change. The decision is
[ADR 0010](../adr/0010-dependency-policy.md) (F20 in #1); this page is the
reference for how it is enforced.

## Rules

- `windows-sys` is pinned exactly (`=0.61.2`). Every bump is a reviewed PR
  (#83); Dependabot ignores it.
- No `notify` crate: capture uses ReadDirectoryChangesExW directly
  (cargo-deny bans it).
- crates.io only: no git or other registries (cargo-deny `[sources]`).
- Licences: MIT, Apache-2.0, Apache-2.0 WITH LLVM-exception, BSD-2-Clause,
  BSD-3-Clause, ISC, Zlib, Unicode-3.0. MPL-2.0 only through a per-crate
  exception in `deny.toml` with a reason.
- Duplicate versions warn, they do not fail (windows-sys / windows
  duplicates are expected for now).
- Every new direct dependency states its binary-size impact in the PR, from
  the size report (#28).

## Where it is enforced

| Check | Where | Gate |
| --- | --- | --- |
| `cargo deny check bans licenses sources` | `ci.yml` job `cargo-deny` | required (in `ci-ok`) |
| `cargo audit` (RustSec advisories) | `audit.yml`, daily | opens or updates one `security` issue; never blocks PRs |
| cargo-machete (unused deps) | `ci.yml` job `lint` | required |

Advisories are deliberately not a PR gate: a new advisory must not block an
unrelated PR. The `rustsec/audit-check` action is not used (its last release
runs on Node 20, which hosted runners removed on 2026-09-23).

## Updates

Dependabot (`.github/dependabot.yml`) checks `cargo`, `rust-toolchain`,
`github-actions` and `pre-commit` daily, with a 7-day cooldown (14 days for
majors on cargo and rust-toolchain). `auto-merge.yml` enables auto-merge,
which still waits for `ci-ok`, only for:

| Ecosystem | Auto-merged | Manual |
| --- | --- | --- |
| cargo, crate `0.x` | patch | minor, major |
| cargo, crate `>= 1.0` | patch, minor | major |
| rust-toolchain | patch, minor | major |
| github-actions, pre-commit | patch, minor | major |

A toolchain bump can bring new clippy lints that fail under deny-warnings;
such a PR stays open until fixed. `rust-version` and `clippy.toml` `msrv` are
bumped by hand.

## Release artefacts

`cargo xtask dist-extras` writes `target/dist-extras/THIRD_PARTY_LICENSES.html`
(cargo-about 0.9.2, `about.toml`, `dist/about.hbs`) and a CycloneDX SBOM
`peekbuddy.cdx.json` (cargo-cyclonedx 0.5.9). The release pipeline (#73)
ships both in the zip and builds with cargo-auditable 0.7.7.

Local tools:

```sh
cargo install --locked cargo-deny@0.20.2 cargo-audit@0.22.2 cargo-cyclonedx@0.5.9
cargo install --locked cargo-about@0.9.2 --features cli
```
