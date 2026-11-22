# Changelog

All notable changes to PeekBuddy are in this file. It is generated from the
commit messages by release-plz and git-cliff.

## [1.1.1] - 2026-11-22

### Tests (1.1.1)

- Trace the shutdown stages in engine.log ([#581](https://github.com/nobuddyorg/PeekBuddy/pull/581))

## [1.1.0] - 2026-11-21

Fixes a data loss in 1.0.0: when a watched folder was missing at start, the
engine logged nothing for any watch (#561). Known limitations, unchanged since
1.0.0:

- The build is unsigned: Windows SmartScreen warns on the first run and Smart
  App Control blocks it. Check the download with `SHA256SUMS` and
  `gh attestation verify` (see the README).
- Watching a network share (UNC path) is not supported.
- Windows 10 is best effort; Windows 11 is the target.
- arm64 is validated in CI only, on no arm64 device.

### Features (1.1.0)

- **store:** Keep USN checkpoints in the store ([#554](https://github.com/nobuddyorg/PeekBuddy/pull/554))
- **engine:** Arm a watch added while running ([#558](https://github.com/nobuddyorg/PeekBuddy/pull/558))

### Fixes (1.1.0)

- Stay overflowed until the follow-up walk ([#510](https://github.com/nobuddyorg/PeekBuddy/pull/510))
- Read live rows from the start time of a walk ([#542](https://github.com/nobuddyorg/PeekBuddy/pull/542))
- Keep an engine without watches free of wakes ([#560](https://github.com/nobuddyorg/PeekBuddy/pull/560))
- **engine:** Close removed watches, register missing roots ([#561](https://github.com/nobuddyorg/PeekBuddy/pull/561))
- **tray:** List the watches as they are now ([#562](https://github.com/nobuddyorg/PeekBuddy/pull/562))
- Forget a snapshot whose folder changed ([#564](https://github.com/nobuddyorg/PeekBuddy/pull/564))

### Performance (1.1.0)

- Log commits of 100 ms or more ([#506](https://github.com/nobuddyorg/PeekBuddy/pull/506))
- Calibrate event size and session end ([#508](https://github.com/nobuddyorg/PeekBuddy/pull/508))
- Bound the memory of idle batches ([#509](https://github.com/nobuddyorg/PeekBuddy/pull/509))
- Report the slowest capture re-arm ([#514](https://github.com/nobuddyorg/PeekBuddy/pull/514))
- Checkpoint by the live log, at 4 MiB ([#520](https://github.com/nobuddyorg/PeekBuddy/pull/520))
- Run the ingest burst without the probe ([#543](https://github.com/nobuddyorg/PeekBuddy/pull/543))
- Intern scope names, pack the table slot ([#546](https://github.com/nobuddyorg/PeekBuddy/pull/546))
- Keep the export's segment open, cut row cost ([#559](https://github.com/nobuddyorg/PeekBuddy/pull/559))
- Put small batches into the last queued one ([#565](https://github.com/nobuddyorg/PeekBuddy/pull/565))
- Size the ring to the burst budget ([#571](https://github.com/nobuddyorg/PeekBuddy/pull/571))

### Tests (1.1.0)

- Date ingest rows by a newest-id probe ([#504](https://github.com/nobuddyorg/PeekBuddy/pull/504))
- Split ingest latency into two stages ([#507](https://github.com/nobuddyorg/PeekBuddy/pull/507))
- Show where a failed path falls among the walks ([#536](https://github.com/nobuddyorg/PeekBuddy/pull/536))
- Pin a root added while the engine runs ([#547](https://github.com/nobuddyorg/PeekBuddy/pull/547))
- Say where a slow second engine spends time ([#550](https://github.com/nobuddyorg/PeekBuddy/pull/550))
- Show engine.log when the engine hangs ([#568](https://github.com/nobuddyorg/PeekBuddy/pull/568))
- Count only children younger than the engine ([#569](https://github.com/nobuddyorg/PeekBuddy/pull/569))

## [1.0.0] - 2026-11-17

The first stable release. Known limitations:

- The build is unsigned: Windows SmartScreen warns on the first run and Smart
  App Control blocks it. Check the download with `SHA256SUMS` and
  `gh attestation verify` (see the README).
- Watching a network share (UNC path) is not supported.
- Windows 10 is best effort; Windows 11 is the target.
- arm64 is validated in CI only, on no arm64 device.
- 21 of the 60 performance budgets are still provisional: no run measures
  them yet (see `PERFORMANCE.md`).

### Features (1.0.0)

- Use the nobuddy.org logo as the app icon (#384)
- **viewer:** Save column widths and compact (#395)
- **viewer:** Ascending time order (#399)
- **viewer:** Save the sort order (#347) (#404)
- **store:** Add markers and schema 2 (#82) (#414)
- **core:** Fold raw rows into a net change (#82) (#412)
- **core:** Rule model and evaluator (#81) (#416)
- Balloon and command actions for rules (#81) (#420)
- Markers, changes since and stats (#82) (#419)
- **viewer:** Edit rules in the settings (#421)
- **core:** Cap rule toasts at six a minute (#422)
- Show rule notifications as toasts (#81) (#425)
- Bare-metal perf run for the owner PC (#427)
- Harness artifact for the owner PC (#84) (#428)
- Publish owner PC reports to gh-pages (#84) (#429)
- Owner-pc column in the budgets (#84) (#430)
- Viewer frame times over 1M rows (#84) (#431)
- Release smoke script for the owner PC (#77) (#457)

### Fixes (1.0.0)

- **engine:** Reconcile entries deleted in a gap (#377)
- **viewer:** Fill the width with the log table (#382)
- **engine:** Satisfy the beta map_unwrap_or lint (#385)
- **engine:** Bound the wait of a second launch (#389)
- Keep beta quiet about the unused mimalloc (#391)
- **viewer:** End lands on a shown row (#401) (#405)
- Drop every 50th event in the self-check (#407)
- Satisfy the beta map_unwrap_or lint (#435)
- Give the nightly paging test more time (#436)
- Wait for the reconcile rows, not the state (#437)
- Alert on the rolling median for query benches (#440)
- Mutation score with no shard artifacts (#453)
- Alert on two slow nights, not one (#403) (#455)
- Serialize the viewer window tests (#458) (#459)
- Default the owner script report path late (#461)
- Two flaky tests of the required job (#383) (#464)
- Shorten the owner PC ingest run (#84) (#463)
- Bump rustix to 1.1.6 for RUSTSEC-2026-0486 (#468)
- Find the last walk past a burst of rows (#475)
- Match bench ids in the trend A/B (#479)
- Read live rows from the start for a baseline (#482)
- Find the tray icon in the Windows 11 overflow (#495)

### Performance (1.0.0)

- Calibrate the 20 benchmark budgets (#493)
- Calibrate 15 budgets from nightly numbers (#494)

### Documentation (1.0.0)

- Use the real asset names in the verify steps (#375)
- Add the dependency watchlist (#83) (#406)
- Update the dependency watchlist (#83) (#415)
- Drop winget from the release plans (#460)
- Explain the empty Run value in the report (#462)
- Add reference pages for the CLI and kinds (#466)
- Add screenshots to the README and tutorial (#473)

### Tests (1.0.0)

- Settle the kill soak after a killed walk (#381)
- Recreate the file while waiting for a watch (#390)
- Add viewer layout snapshots and kittest (#394)
- **viewer:** Add a scripted-scroll frame bench (#396)
- Cover the watch state mapping end to end (#400)
- **core:** Catch the surviving rule mutants (#426)
- Examine viewer timeline and worker mutants (#441)
- UI Automation smoke of the viewer (#71) (#456)
- Check that PeekBuddy makes no network call (#472)
- Widen two start limits in mutation runs (#476)
- Drop split renames at a walk in the replay (#481)
- Wait for the walk and for the worker thread (#483)
- Pair renames by file id in the replay (#485)

### Build and CI (1.0.0)

- Run the profiling recipes on the runners (#369)
- Gate the nightly mutants on a score of 80 (#386)
- Ignore the audit notes of Linux-only crates (#387)
- Run mutants only at night, give it time (#392)
- Run the 10k events/s burst on hosted runners (#393)
- Check the viewer private bytes at 1M and 10M (#397)
- Skip two mutants that kill the runner (#408)
- Leave glow to eframe (#409)
- Examine a seventh of the mutants each night (#469)
- Confirm trend alerts with a paired A/B run (#470)
- Skip the mutation run when nothing changed (#471)
- Log memory and use two threads in core mutants (#480)
- Cap the memory of every mutant process (#492)

### Other (1.0.0)

- Remove CODEOWNERS (#380)
- Move to Rust 1.100 (#83) (#452)

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
