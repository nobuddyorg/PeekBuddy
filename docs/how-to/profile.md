# Profile PeekBuddy

How to answer "why is it slow" and "why does it wake up" the same way every
time (#29). Budgets and gates are in [PERFORMANCE.md](../../PERFORMANCE.md);
this page is about tools.

Every recipe ends with a `Last verified` line. A recipe is dated only when
someone ran it; otherwise it says `not yet executed`. Recipes older than six
months are flagged in the release checklist (#77).

Elevation: samply, `cargo flamegraph`, WPR kernel providers and Tracy
sampling or context-switch capture need an elevated shell or process. The
PeekBuddy engine itself never runs elevated in v1.

## Which tool for which question

| Question | Tool | Section |
| --- | --- | --- |
| Where does CPU time go? | samply, `cargo flamegraph` | [1](#1-quick-cpu-sampling) |
| What wakes a thread, what is the disk doing at startup? | WPR + WPA, PerfView | [2](#2-wpr-wpa-and-perfview) |
| Which zone or span is slow, live? | Tracy | [3](#3-tracy) |
| Do commercial profilers help? | Superluminal, VTune | [4](#4-superluminal-and-vtune) |
| Where do allocations come from? | counting allocator, Tracy, dhat | [5](#5-heap) |
| Is the viewer smooth? | PresentMon, Tracy frames | [6](#6-viewer-frame-pacing) |
| Is Defender in the numbers? | exclusion on/off | [7](#7-defender-on-or-excluded) |
| Can I claim a win? | checklist | [8](#8-checklist-before-claiming-a-perf-win) |

## Tool versions

These match the workspace pins and the CI install steps. Bump them together
with the code.

| Tool | Version | Source |
| --- | --- | --- |
| samply | 0.13.1 | `cargo install samply --version 0.13.1 --locked` |
| cargo-flamegraph | 0.6.14 | `cargo install flamegraph --version 0.6.14 --locked` |
| Tracy GUI | 0.14.1 | wolfpld/tracy release 0.14.1 (matches `tracy-client-sys` 0.30.1) |
| PerfView | 4.0.0 | microsoft/perfview release v4.0.0 |
| PresentMon | 2.6.0 | GameTechDev/PresentMon release v2.6.0 |
| hyperfine | 2.0.0 | `cargo install hyperfine --version 2.0.0 --locked` |
| dhat | 0.3.3 | dev-dependency of a throwaway test binary |
| WPR and WPA | in the OS (WPR) and the Windows Performance Toolkit (WPA, xperf) | ADK or Microsoft Store |

## 1. Quick CPU sampling

Use a profiling build so symbols exist: `cargo build --profile profiling`
(release optimisations, line tables, separate PDB; see PERFORMANCE.md).

samply records through xperf and asks for administrator rights.

```powershell
# elevated shell
samply record --windows-symbol-server https://msdl.microsoft.com/download/symbols `
  target\profiling\peekbuddy.exe
```

`cargo flamegraph` uses the blondie backend and needs an elevated shell. It
prefers dtrace when one is on `PATH`; remove it from `PATH` to force blondie.

```powershell
# elevated shell
cargo flamegraph --profile profiling --bin peekbuddy
```

Look at: the widest frames under the thread you care about, and whether the
engine thread tree is mostly idle. A flat profile with the engine at ~0 %
is the goal.

Last verified: not yet executed.

## 2. WPR, WPA and PerfView

`perf/peekbuddy.wprp` carries two profiles (#21): `PeekBuddy.Light` (the
provider, CPU sampling, context switches) and `PeekBuddyStartup` (adds disk
I/O, file I/O and hard faults). WPR reads `Name.DetailLevel`, which is why
the startup profile is not called `PeekBuddy.Startup`.

Elevated shell:

```powershell
wpr -start perf\peekbuddy.wprp!PeekBuddy.Light -filemode
# run PeekBuddy, do the thing
wpr -stop out.etl
```

**60 s idle procedure.** Start the engine with its roots armed and no file
activity, start the `PeekBuddy.Light` trace, wait 60 s, stop. In WPA open
CPU Usage (Precise) and filter to `peekbuddy.exe`: count `CSwitch` rows with
`NewThread` in PeekBuddy. The budget (`engine.idle.wakes`) is at most two
wakeups a minute and no periodic timers (`engine.idle.periodic_timers`);
expect close to zero context switches.

**Startup procedure.** Use `PeekBuddyStartup`, start the trace, launch the
exe, stop after "engine ready". The `Milestone` events of the
`NoBuddy-PeekBuddy` provider mark the phases; disk I/O and hard faults show
what the startup reads.

**PerfView "Thread Time (with ReadyThread)".** Collect with PerfView 4.0.0
(`Collect`, tick `Thread Time`, `ThreadTime with ReadyThread`) to see who
readied a waiting thread. Use it when the idle trace shows wakes and you
need the waker. PerfView runs elevated.

WPA comes from the Microsoft Store or the ADK. Symbols: set
`_NT_SYMBOL_PATH` to `srv*C:\symbols*https://msdl.microsoft.com/download/symbols`
and add the folder holding the PDB next to the exe.

Last verified: not yet executed. CI records the `etw_smoke` example with
`PeekBuddy.Light` on every PR (job `etw smoke (windows x64)`), which covers
the `wpr` start and stop commands but not WPA or PerfView.

## 3. Tracy

Developer-only; no Tracy code ships (PERFORMANCE.md, "Profiling with Tracy").

```powershell
cargo xtask profile --tracy [--alloc] [--role engine|viewer]
```

The Tracy GUI must be exactly 0.14.1: client and GUI versions have to match
(`tracy-client-sys` 0.30.1 is the 0.14.1 protocol). The client is built with
`ondemand`: nothing is buffered until the GUI connects, so start the GUI and
press Connect. It listens on localhost only. Sampling and context-switch
capture need the PeekBuddy process to run elevated; zones and plots do not.

To look at engine and viewer together, start both roles from the profiling
build and connect the GUI to each in turn (one client per process; run a
second GUI instance for the second process).

Last verified: not yet executed. `profiling.yml` builds the profiling exe
and starts both roles in CI; there is no headless capture check yet because
the engine does not produce batches (#59, #52).

## 4. Superluminal and VTune

Notes only; nothing in CI uses them.

- Superluminal: commercial. Rust support, licence terms, price and trial are
  unverified: check the vendor site.
- Intel VTune: hardware-event sampling works best on Intel CPUs. Licence
  terms, Rust support and the current version are unverified: check the
  vendor site.

Last verified: not applicable.

## 5. Heap

- **Counting allocator.** `peekbuddy-alloc` counts allocations globally and
  per thread; wrap a section in `AllocScope` and assert on the delta. The
  allocation budgets (`alloc.*` in `perf/budgets.toml`) are tests, for
  example `crates/peekbuddy-core/tests/capture_alloc.rs`.
- **Tracy `ProfiledAllocator`.** `cargo xtask profile --tracy --alloc`
  records call stacks (depth 8) for every allocation in the Tracy memory
  view.
- **dhat 0.3.3** answers "where do the bytes come from". It is
  experimental: backtraces are much slower on Windows and it wants a
  single-threaded test binary. Add it to a throwaway test, not to the
  workspace:

```rust
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

#[test]
fn heap() {
    let _profiler = dhat::Profiler::new_heap();
    // code under test
}
```

The result is `dhat-heap.json`; open it in the dhat viewer.

Last verified: not yet executed.

## 6. Viewer frame pacing

PresentMon 2.6.0 needs no administrator rights since 2.3.1. It sees
swap-chain presents, so the wgpu DX12 viewer (ADR 0005) is captured.

```powershell
PresentMon.exe --process_name peekbuddy.exe --output_file frames.csv --terminate_on_proc_exit
```

(Flag names changed between PresentMon releases: check `PresentMon.exe --help`
for the installed version.) Compare `MsBetweenPresents` with the frame
budgets in `perf/budgets.toml`. With Tracy, mark frames with `frame_mark` and
read the frame histogram.

Measuring idle: egui repaints a full frame on every pointer move
(emilk/egui#8695). Move the mouse out of the window, and expect very
different numbers over RDP or on the WARP software adapter that hosted
runners use.

Last verified: not yet executed.

## 7. Defender on or excluded

Defender scans `.db` and `-wal` files and the exe itself, so every result
states its state. The x64 hosted image turns it off; `windows-11-vs2026-arm`
always has it on (Tamper Protection). Never compare across the two.

```powershell
# elevated shell: read the state
Get-MpPreference | Select-Object -ExpandProperty ExclusionPath
(Get-MpComputerStatus).RealTimeProtectionEnabled

# add an exclusion for the test directory, remove it afterwards
Add-MpPreference -ExclusionPath C:\pbperf
Remove-MpPreference -ExclusionPath C:\pbperf
```

Do not switch real-time protection off for a measurement; an exclusion for
the work directory is enough and is undone by the second command.

Last verified: not yet executed.

## 8. Checklist before claiming a perf win

- Release or profiling build, never debug.
- AC power and the "Best performance" power mode; no other heavy processes.
- At least three runs, medians. Outside timing: `hyperfine -N --warmup 3
  --runs 30 <exe> --perf-probe 0` (once #24 lands the probe). Same-job
  comparison: `cargo xtask bench-ab`, and `cargo xtask perf --ab` once #27
  lands.
- Cite the `perf/budgets.toml` row.
- Before and after numbers in the PR description, with the Defender state and
  the machine.

`hyperfine` itself: `hyperfine -N` runs the command without a shell, so
there is no `cmd.exe` in the timing.

```powershell
hyperfine -N --warmup 1 --runs 3 "cmd /c exit"
```

Last verified: hyperfine 2.0.0 command above, 2026-10-25, Windows 11 build
26200, x64.
