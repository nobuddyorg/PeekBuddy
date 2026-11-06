# Changelog

All notable changes to PeekBuddy are in this file. It is generated from the
commit messages by release-plz and git-cliff.

## [0.1.0-rc.1] - 2026-11-06

### Build and CI

- Add cargo workspace skeleton and lint policy (#90)
- Add a nightly kill soak on both runners (#263)

### Features

- Add the Tracy profiling build (#108)
- **engine:** Add the engine process skeleton (#173)
- **engine:** Add --perf-probe and --diagnostics (#174)
- **engine:** Release the ETW provider on shutdown (#175)
- **core:** Add versioned settings.toml (#177)
- **engine:** Add the tray icon and menu (#178)
- **ipc:** Add the engine-viewer pipe (#179)
- **store:** Add the writer pipeline (#180)
- **engine:** Add pause and autostart (#63) (#182)
- **engine:** Add the watch lifecycle (#48) (#185)
- **engine:** Record an abnormal exit at start (#171) (#186)
- **engine:** Emit the milestones (#162) (#198)
- **store:** Add day segments and retention (#54) (#201)
- Follow link targets, hide cloud churn (#285)
- Detect torn spill tails and store health (#286)
- **viewer:** Open the on-demand window (#323)
- **viewer:** Watches, settings and first-run (#328)
- **viewer:** Render x64 through glow (#343)
- **viewer:** Fall back to wgpu without OpenGL (#349)
- Tracy plots and frame marks (#163) (#362)
- Add an optional mimalloc allocator (#268) (#368)

### Fixes

- Make the nightly run green (#104)
- **engine:** Watch a lost root's remaining path (#312)
- Lockfile of the tracy features, close the viewer test (#364)

### Performance

- Add ingest and handles scenarios (#294)

### Tests

- Shorten the pipeline proptest (#184) (#195)
- **engine:** Relax the headless ready bound (#197)
- **engine:** Add the zero-loss PR tier (#50) (#208)
- **store:** Page from any cursor against a model (#221)
- **engine:** Add the kill soak smoke test (#216) (#225)
- **engine:** Add the junction fixture tests (#213) (#227)
- **engine:** Name the failed churn operations (#233)
- **engine:** Skip the tray test where icons fail (#282)
- Add the nightly zero-loss tier (#292)
- Pair renames, drain the spill before judging (#307)
- Log the zero-loss phases for the nightly (#329)
- Widen the second-engine limit for coverage (#356)
- Widen the second-engine limit on arm64 (#367)
