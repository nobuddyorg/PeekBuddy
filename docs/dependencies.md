# Dependency watchlist

Upstream changes that will break or improve PeekBuddy. Each row has a
trigger; when it fires, the migration is one PR (Dependabot's or a hand-made
one, no separate issue) with the same-runner A/B numbers (warn +15 %, fail
+50 %), the binary-size and `cargo tree -d` deltas, and an ADR if behaviour
changes. Dependabot merges routine patch updates; 0.x minors and majors are
manual. Review the table monthly and whenever Dependabot opens a manual-only
PR. Cargo resolves a release only once it is 7 days old
(`registry.global-min-publish-age`), so a trigger fires a week after the
upstream release at the earliest.

| Item | Pin today | Target | Trigger | Status |
| --- | --- | --- | --- | --- |
| Rust toolchain | 1.100.0 | next stable | released every six weeks | done in the 1.100 migration PR |
| windows / windows-sys | windows-sys =0.61.2 | 0.100 | tray-icon, muda, eframe, windows-service and tauri-winrt-notification have moved | open |
| egui, eframe, egui_table | =0.37.0, =0.11.0 | next egui release | about every 6 weeks, manual | open |
| rusqlite / SQLite | 0.41.1 | 0.43.0 (published 2026-11-15) | Dependabot, manual | 0.41 done in #410; 0.43 resolvable from 2026-11-22 |
| winit | via eframe | 0.31 | eframe depends on a stable 0.31 | 0.31.0-beta.4 only (checked 2026-11-21) |
| glow | via eframe (0.17) | follows eframe | eframe re-exports glow; Dependabot ignores it | no change |
| Tracy | tracy-client-sys =0.30.1 (Tracy 0.14.1) | next protocol | Tracy GUI of the playbook moves | open |
| notify 9 | not used (F20) | reference only | none | no change |

Last review 2026-11-21, on crates.io: egui and eframe 0.37.0, windows-sys
0.61.2, tray-icon 0.26.1 and tracy-client-sys 0.30.1 are still the newest
releases, winit 0.31 is in beta, rusqlite 0.43.0 is out.

## Rust 1.100

Done in one PR, because the new keys warn on 1.99 and
`CARGO_BUILD_WARNINGS=deny` turns warnings into failures:

- bump `rust-toolchain.toml`,
- replace cargo-machete with `[lints.cargo] unused_dependencies` and remove
  it from CI and prek,
- add `registry.global-min-publish-age = "7 days"` to `.cargo/config.toml`.

## windows and windows-sys 0.100

Lowercase SDK-header module names replace the `Win32_*` features. MSRV is
1.95 (windows) and 1.88 (windows-sys). The goal is one windows-sys version
in the tree, then cargo-deny `multiple-versions = "deny"` for `windows-*`.

## winit 0.31

Ripples through eframe, egui-winit and tray-icon. Viewer only: the engine
has no winit (F1). Re-run the startup and frame-time measurements.

## egui and eframe

Exact pins, manual upgrade. Check egui_table compatibility, refresh the
kittest snapshots, and check that the `App::logic` and `App::ui` usage
still matches.

## rusqlite

Re-run the storage benches (#35) and the durability tests (#57). Verify the
FTS5 trigram behaviour and `LIBSQLITE3_FLAGS`.

## Tracy

A protocol bump needs the matching Tracy GUI: update the profiling playbook
(`docs/how-to/profile.md`) in the same PR.
