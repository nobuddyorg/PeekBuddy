# Contributing

Issues are welcome through the issue forms. Pull requests are accepted only
for issues the owner has accepted ([ADR 0014](docs/adr/0014-contribution-policy.md)).

## Workspace layout

| Crate | Owns | `unsafe` |
| --- | --- | --- |
| `crates/peekbuddy-core` | Event model, names, filters, derived views. Pure Rust, builds and tests on Linux, no `cfg(windows)`. | forbidden |
| `crates/peekbuddy-win` | `windows-sys` wrappers and OS-independent record parsers (these also compile on Linux for Miri). | allowed, every block needs `// SAFETY:` |
| `crates/peekbuddy-store` | SQLite day segments, writer thread, queries, export. | forbidden |
| `crates/peekbuddy-engine` | Capture threads, reconcile scanner, tray, pipe server. No GPU or UI crates. | forbidden |
| `crates/peekbuddy-viewer` | The egui viewer window. | forbidden |
| `crates/peekbuddy` | The single `peekbuddy.exe`: engine by default, `--viewer` for the viewer. | forbidden |
| `xtask` | Repository automation (`cargo xtask <command>`). | forbidden |
| `benches` | Micro-benchmarks. | forbidden |

Two checks keep the layout honest:

- `cargo xtask check-unsafe`: every crate root has `#![forbid(unsafe_code)]`
  except the allow-list in `xtask/src/unsafe_code.rs` (only `peekbuddy-win`).
- `cargo xtask check-deps`: `peekbuddy-core` and `peekbuddy-engine` never
  depend on `eframe`, `egui`, `winit`, `wgpu` or `glow` (ADR 0001).

## Lint policy

Warnings are errors in CI (`CARGO_BUILD_WARNINGS=deny`). `unwrap`, `expect`,
`panic`, `todo`, `dbg!` and printing macros are denied outside tests; indexing
and slicing are denied in `peekbuddy-core` and `peekbuddy-win`. Use
`#[expect(..., reason = "...")]`, never `#[allow]`.
