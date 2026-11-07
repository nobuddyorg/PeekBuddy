# Dependency watchlist

Upstream changes that will break or improve PeekBuddy. Each row has a
trigger; when it fires, the migration gets its own issue and PR with the
same-runner A/B numbers (warn +15 %, fail +50 %), the binary-size and
`cargo tree -d` deltas, and an ADR if behaviour changes. Dependabot merges
routine patch updates; 0.x minors and majors are manual. Review the table
monthly and whenever Dependabot opens a manual-only PR.

| Item | Pin today | Target | Trigger | Status |
| --- | --- | --- | --- | --- |
| Rust toolchain | 1.99.1 | 1.100 | released 2026-11-12 | open |
| windows / windows-sys | windows-sys =0.61.2 | 0.100 | tray-icon, muda, eframe, windows-service and tauri-winrt-notification have moved | open |
| winit | via eframe | 0.31 | eframe depends on a stable 0.31 | open |
| egui, eframe, egui_table | =0.37.0, =0.11.0 | next egui release | about every 6 weeks, manual | open |
| rusqlite / SQLite | 0.41.1 | next minor | Dependabot, manual | done in #410 |
| glow | via eframe (0.17) | follows eframe | eframe re-exports glow; Dependabot ignores it | no change |
| Tracy | tracy-client-sys =0.30.1 (Tracy 0.14.1) | next protocol | Tracy GUI of the playbook moves | open |
| notify 9 | not used (F20) | reference only | none | no change |

## Rust 1.100

One PR, because the new keys warn on 1.99 and `CARGO_BUILD_WARNINGS=deny`
turns warnings into failures:

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
