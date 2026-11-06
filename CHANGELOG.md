# Changelog

All notable changes to PeekBuddy are in this file. It is generated from the
commit messages by release-plz and git-cliff.

## [0.0.0] - 2026-11-06

### Build and CI

- Add cargo workspace skeleton and lint policy ([#90](https://github.com/nobuddyorg/PeekBuddy/pull/90))
- Add a nightly kill soak on both runners ([#263](https://github.com/nobuddyorg/PeekBuddy/pull/263))

### Features

- Add the Tracy profiling build ([#108](https://github.com/nobuddyorg/PeekBuddy/pull/108))
- **engine:** Add the engine process skeleton ([#173](https://github.com/nobuddyorg/PeekBuddy/pull/173))
- **engine:** Add --perf-probe and --diagnostics ([#174](https://github.com/nobuddyorg/PeekBuddy/pull/174))
- **engine:** Release the ETW provider on shutdown ([#175](https://github.com/nobuddyorg/PeekBuddy/pull/175))
- **core:** Add versioned settings.toml ([#177](https://github.com/nobuddyorg/PeekBuddy/pull/177))
- **engine:** Add the tray icon and menu ([#178](https://github.com/nobuddyorg/PeekBuddy/pull/178))
- **ipc:** Add the engine-viewer pipe ([#179](https://github.com/nobuddyorg/PeekBuddy/pull/179))
- **store:** Add the writer pipeline ([#180](https://github.com/nobuddyorg/PeekBuddy/pull/180))
- **engine:** Add pause and autostart ([#63](https://github.com/nobuddyorg/PeekBuddy/pull/63)) ([#182](https://github.com/nobuddyorg/PeekBuddy/pull/182))
- **engine:** Add the watch lifecycle ([#48](https://github.com/nobuddyorg/PeekBuddy/pull/48)) ([#185](https://github.com/nobuddyorg/PeekBuddy/pull/185))
- **engine:** Record an abnormal exit at start ([#171](https://github.com/nobuddyorg/PeekBuddy/pull/171)) ([#186](https://github.com/nobuddyorg/PeekBuddy/pull/186))
- **engine:** Emit the milestones ([#162](https://github.com/nobuddyorg/PeekBuddy/pull/162)) ([#198](https://github.com/nobuddyorg/PeekBuddy/pull/198))
- **store:** Add day segments and retention ([#54](https://github.com/nobuddyorg/PeekBuddy/pull/54)) ([#201](https://github.com/nobuddyorg/PeekBuddy/pull/201))
- Follow link targets, hide cloud churn ([#285](https://github.com/nobuddyorg/PeekBuddy/pull/285))
- Detect torn spill tails and store health ([#286](https://github.com/nobuddyorg/PeekBuddy/pull/286))
- **viewer:** Open the on-demand window ([#323](https://github.com/nobuddyorg/PeekBuddy/pull/323))
- **viewer:** Watches, settings and first-run ([#328](https://github.com/nobuddyorg/PeekBuddy/pull/328))
- **viewer:** Render x64 through glow ([#343](https://github.com/nobuddyorg/PeekBuddy/pull/343))
- **viewer:** Fall back to wgpu without OpenGL ([#349](https://github.com/nobuddyorg/PeekBuddy/pull/349))
- Tracy plots and frame marks ([#163](https://github.com/nobuddyorg/PeekBuddy/pull/163)) ([#362](https://github.com/nobuddyorg/PeekBuddy/pull/362))
- Add an optional mimalloc allocator ([#268](https://github.com/nobuddyorg/PeekBuddy/pull/268)) ([#368](https://github.com/nobuddyorg/PeekBuddy/pull/368))

### Fixes

- Make the nightly run green ([#104](https://github.com/nobuddyorg/PeekBuddy/pull/104))
- **engine:** Watch a lost root's remaining path ([#312](https://github.com/nobuddyorg/PeekBuddy/pull/312))
- Lockfile of the tracy features, close the viewer test ([#364](https://github.com/nobuddyorg/PeekBuddy/pull/364))

### Performance

- Add ingest and handles scenarios ([#294](https://github.com/nobuddyorg/PeekBuddy/pull/294))

### Tests

- Shorten the pipeline proptest ([#184](https://github.com/nobuddyorg/PeekBuddy/pull/184)) ([#195](https://github.com/nobuddyorg/PeekBuddy/pull/195))
- **engine:** Relax the headless ready bound ([#197](https://github.com/nobuddyorg/PeekBuddy/pull/197))
- **engine:** Add the zero-loss PR tier ([#50](https://github.com/nobuddyorg/PeekBuddy/pull/50)) ([#208](https://github.com/nobuddyorg/PeekBuddy/pull/208))
- **store:** Page from any cursor against a model ([#221](https://github.com/nobuddyorg/PeekBuddy/pull/221))
- **engine:** Add the kill soak smoke test ([#216](https://github.com/nobuddyorg/PeekBuddy/pull/216)) ([#225](https://github.com/nobuddyorg/PeekBuddy/pull/225))
- **engine:** Add the junction fixture tests ([#213](https://github.com/nobuddyorg/PeekBuddy/pull/213)) ([#227](https://github.com/nobuddyorg/PeekBuddy/pull/227))
- **engine:** Name the failed churn operations ([#233](https://github.com/nobuddyorg/PeekBuddy/pull/233))
- **engine:** Skip the tray test where icons fail ([#282](https://github.com/nobuddyorg/PeekBuddy/pull/282))
- Add the nightly zero-loss tier ([#292](https://github.com/nobuddyorg/PeekBuddy/pull/292))
- Pair renames, drain the spill before judging ([#307](https://github.com/nobuddyorg/PeekBuddy/pull/307))
- Log the zero-loss phases for the nightly ([#329](https://github.com/nobuddyorg/PeekBuddy/pull/329))
- Widen the second-engine limit for coverage ([#356](https://github.com/nobuddyorg/PeekBuddy/pull/356))
- Widen the second-engine limit on arm64 ([#367](https://github.com/nobuddyorg/PeekBuddy/pull/367))
