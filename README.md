<img src="docs/assets/peek.svg" alt="" width="96" height="96">

# PeekBuddy

Silently watches your folders and logs every change in real time 👀

[![CI](https://github.com/nobuddyorg/PeekBuddy/actions/workflows/ci.yml/badge.svg)](https://github.com/nobuddyorg/PeekBuddy/actions/workflows/ci.yml)
[![codecov](https://img.shields.io/codecov/c/github/nobuddyorg/PeekBuddy?logo=codecov&logoColor=white)](https://codecov.io/gh/nobuddyorg/PeekBuddy)
[![CodeQL](https://img.shields.io/badge/security-CodeQL-blue?logo=github)](https://github.com/nobuddyorg/PeekBuddy/security/code-scanning)
[![OpenSSF Scorecard](https://api.scorecard.dev/projects/github.com/nobuddyorg/PeekBuddy/badge)](https://scorecard.dev/viewer/?uri=github.com/nobuddyorg/PeekBuddy)
![cargo-deny](https://img.shields.io/badge/supply%20chain-cargo--deny-orange)
![cargo-mutants](https://img.shields.io/badge/mutation%20testing-cargo--mutants-8A2BE2)
![Rust 1.99.1](https://img.shields.io/badge/rust-1.99.1-b7410e?logo=rust&logoColor=white)
![Windows](https://img.shields.io/badge/platform-Windows%2011%20x64%20%7C%20arm64-0078D4?logo=windows&logoColor=white)
[![License: MIT](https://img.shields.io/github/license/nobuddyorg/PeekBuddy)](LICENSE)

## Motivation

Something rewrote your config file overnight, a build tool keeps touching a
folder it should not, or you simply want to know what an installer did.
Windows does not keep a readable history of file changes. PeekBuddy sits in
the tray, watches the folders you pick and keeps an ordered, searchable log of
every change, without admin rights and without sending anything anywhere.

## Features

Planned for v1 (see the [roadmap](https://github.com/nobuddyorg/PeekBuddy/issues/1)):

- Watch any number of folders, recursive or not, on local volumes and UNC
  shares.
- Zero undetected loss: when Windows drops notifications, PeekBuddy logs a
  visible gap and rescans the affected subtree.
- One row per raw change, with derived views for renames, atomic saves and
  coalescing that always show the raw count.
- Include and exclude filters with gitignore-style globs and regex.
- Pause and resume, globally and per watch.
- A fast viewer that stays smooth at millions of rows, with live tail and
  export to CSV and JSON Lines.
- Tray-first, opt-in autostart, light and dark theme, keyboard accessible.

## How it works

One `peekbuddy.exe` runs as a small tray engine that captures changes with
ReadDirectoryChangesExW and writes them to daily SQLite files. The viewer
window is the same exe started with `--viewer`, only while you look.
[Architecture](docs/architecture.md) says what exists, the
[ADRs](docs/adr/README.md) say why.

## Using PeekBuddy

PeekBuddy is not released yet. Tutorials and how-to guides follow with v1.

## System requirements

Windows 11 x64 or arm64; Windows 10 22H2 best-effort. See
[SUPPORT.md](SUPPORT.md).

## Performance

Performance budgets are enforced in CI. See `PERFORMANCE.md` (added with #20).

## Privacy

No login, no server, no tracking; the log never leaves your disk. PeekBuddy
makes no network connections. See [PRIVACY.md](PRIVACY.md). To report a
vulnerability, see [SECURITY.md](SECURITY.md) and the
[threat model](docs/explanation/threat-model.md).

## Unsigned builds

v1 ships unsigned ([ADR 0012](docs/adr/0012-code-signing.md)). Smart App
Control blocks unsigned executables where it is on, and SmartScreen warns on
first start. Every release ships `SHA256SUMS` and GitHub attestations so you
can verify the download (details follow with #87).

## Documentation

Docs live in [`docs/`](docs/index.md), organised by [Diátaxis](https://diataxis.fr):

- **Tutorials**: follow with v1
- **How-to**: follow with v1
- **Reference**: [Supported Windows versions](SUPPORT.md)
- **Explanation**: [Architecture](docs/architecture.md) · [Decisions](docs/adr/README.md) · [Threat model](docs/explanation/threat-model.md)

## Contributing

Issues are welcome through the issue forms: bug reports, performance
regressions with numbers, and feature requests. Pull requests are accepted
only for issues the owner has accepted; unsolicited pull requests are closed.

## License

This project is licensed under the MIT License.
