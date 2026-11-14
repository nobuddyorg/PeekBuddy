<#
.SYNOPSIS
    Absolute performance numbers of a PeekBuddy build on real hardware (#84).
.DESCRIPTION
    Hosted runners have no GPU and are noisy, so the absolute numbers of the
    F6/F7/F8 budgets come from this script on the everyday Windows 11 x64 PC,
    once per release. It needs no Rust toolchain, no elevation and makes no
    persistent changes. It takes about 12 minutes; leave the PC alone while
    it runs.

    Inputs: the release peekbuddy.exe and the perf harness (xtask.exe and
    perf-driver.exe, from the `peekbuddy-perf-harness` artifact of the CI run
    that built them). Check them against the attestation (`gh attestation
    verify`) or SHA256SUMS of the release before use
    (docs/how-to/verify-download.md). Without -Driver the ingest and handles
    scenarios are skipped and listed under `notMeasured`.

    What it does:
      1. Preflight, recorded and warned about, never refused: AC power, power
         plan, Windows build, GPU and driver, Defender, Smart App Control and
         the five busiest processes.
      2. The harness (`xtask perf`, scenarios startup, idle and memory, 3
         runs each): engine ready warm, startup CPU and disk read, idle CPU
         and wakeups, Private Bytes; with -Driver also ingest (the churn
         profiles, event latency), handle stability and the viewer's frame
         times while the log of 1M rows is scrolled in a real window (20 s).
         Ingest runs the latency-10k profile only.
      3. The viewer's first frame and idle repaints, 3 runs, on the real GPU.

    What it does not measure yet is listed in `notMeasured` of the report with
    the reason (cold start needs a standby-list flush; the 1M-row export
    needs a harness scenario that does not exist).

    Writes perf-baremetal-<version>-<date>.json next to the script unless
    -OutFile is given. Compare numbers only with earlier runs on the same
    machine: an everyday PC is noisy and the preflight data sits next to every
    number for that reason.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Exe,
    [Parameter(Mandatory)][string]$Harness,
    [string]$Driver,
    [string]$OutFile,
    [int]$Runs = 3,
    [int]$IdleSecs = 60
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$started = Get-Date
$Exe = (Resolve-Path $Exe).Path
$Harness = (Resolve-Path $Harness).Path
if ($Driver) { $Driver = (Resolve-Path $Driver).Path }
$work = Join-Path ([IO.Path]::GetTempPath()) "perf-baremetal-$([guid]::NewGuid())"
New-Item -ItemType Directory $work | Out-Null

function Get-Median([double[]]$Values) {
    if ($Values.Count -eq 0) { return $null }
    $sorted = $Values | Sort-Object
    $mid = [int][math]::Floor($sorted.Count / 2)
    if ($sorted.Count % 2 -eq 1) { return $sorted[$mid] }
    return ($sorted[$mid - 1] + $sorted[$mid]) / 2
}

function Get-Preflight {
    $warnings = New-Object System.Collections.Generic.List[string]
    $os = Get-CimInstance -ClassName Win32_OperatingSystem
    $cpu = Get-CimInstance -ClassName Win32_Processor | Select-Object -First 1
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $elevated = ([Security.Principal.WindowsPrincipal]::new($identity)).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)

    $battery = @(Get-CimInstance -ClassName Win32_Battery -ErrorAction SilentlyContinue)
    # BatteryStatus 2 is "on AC"; no battery at all is a desktop.
    $onAc = ($battery.Count -eq 0) -or ($battery | Where-Object { $_.BatteryStatus -eq 2 })
    if (-not $onAc) { $warnings.Add('The PC runs on battery; plug it in.') }

    $plan = 'unknown'
    try { $plan = ((powercfg /getactivescheme) -join ' ') -replace '^.*\((.*)\).*$', '$1' } catch { }
    if ($plan -match 'Power saver|Energiesparmodus') { $warnings.Add("Power plan is '$plan'.") }

    $gpus = @(Get-CimInstance -ClassName Win32_VideoController | ForEach-Object {
        [ordered]@{ name = $_.Name; driver = $_.DriverVersion }
    })

    $defender = 'unknown'
    try {
        $mp = Get-MpComputerStatus -ErrorAction Stop
        $defender = if ($mp.RealTimeProtectionEnabled) { 'real-time-on' } else { 'real-time-off' }
    } catch { }

    $sac = 'not-present'
    try {
        $state = Get-ItemPropertyValue -Path 'HKLM:\SYSTEM\CurrentControlSet\Control\CI\Policy' -Name 'VerifiedAndReputablePolicyState'
        $sac = switch ($state) { 0 { 'off' } 1 { 'enforce' } 2 { 'evaluation' } default { "$state" } }
    } catch { }
    if ($sac -eq 'enforce') { $warnings.Add('Smart App Control is on; an unsigned build will not start.') }

    # CPU use over two seconds, not since boot.
    $before = @{}
    Get-Process | ForEach-Object { try { $before[$_.Id] = $_.TotalProcessorTime.TotalSeconds } catch { } }
    Start-Sleep -Seconds 2
    $busy = @(Get-Process | ForEach-Object {
        try {
            $was = $before[$_.Id]
            if ($null -ne $was) {
                [pscustomobject]@{ name = $_.ProcessName; cpuPercentOfOneCore = [math]::Round(($_.TotalProcessorTime.TotalSeconds - $was) / 2 * 100, 1) }
            }
        } catch { }
    } | Sort-Object cpuPercentOfOneCore -Descending | Select-Object -First 5)
    foreach ($p in $busy) {
        if ($p.cpuPercentOfOneCore -ge 25) { $warnings.Add("'$($p.name)' uses $($p.cpuPercentOfOneCore) % of a core; close it for cleaner numbers.") }
    }

    return [ordered]@{
        os            = [ordered]@{ caption = $os.Caption; version = $os.Version; build = $os.BuildNumber }
        architecture  = $env:PROCESSOR_ARCHITECTURE
        cpu           = $cpu.Name
        logicalCpus   = [Environment]::ProcessorCount
        memoryGiB     = [math]::Round($os.TotalVisibleMemorySize / 1MB, 1)
        elevated      = $elevated
        onAcPower     = [bool]$onAc
        powerPlan     = $plan
        gpus          = $gpus
        defender      = $defender
        smartAppControl = $sac
        busiestProcesses = $busy
        warnings      = @($warnings)
    }
}

Write-Output 'Preflight...'
$preflight = Get-Preflight
foreach ($w in $preflight.warnings) { Write-Warning $w }

Write-Output "Harness ($Runs runs, idle $IdleSecs s; several minutes)..."
$hr = [ordered]@{ exitCode = 0; scenarios = [ordered]@{}; log = '' }
# Native stderr must not stop the script: the harness reports through its exit code.
$ErrorActionPreference = 'Continue'
$scenarios = @('startup', 'idle', 'memory')
if ($Driver) {
    $scenarios += @('ingest', 'handles', 'viewer-frames')
    $env:PEEKBUDDY_PERF_DRIVER = $Driver
}
foreach ($scenario in $scenarios) {
    $out = Join-Path $work "harness-$scenario.json"
    # Ingest runs the profile that measures the event latency (about two minutes);
    # the burst and sustained profiles take 30 minutes on this kind of PC, and
    # its generator cannot reach their rate anyway (they say 'not exercised').
    $extra = if ($scenario -eq 'ingest') { @('--profile', 'latency-10k') } else { @() }
    $log = & $Harness perf --scenario $scenario --exe $Exe --runs $Runs --idle-secs $IdleSecs @extra --out $out 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { $hr.exitCode = $LASTEXITCODE }
    $hr.log += "== $scenario`n$log"
    $hr.scenarios[$scenario] = if (Test-Path $out) { Get-Content $out -Raw | ConvertFrom-Json } else { $null }
}
$ErrorActionPreference = 'Stop'
Remove-Item Env:PEEKBUDDY_PERF_DRIVER -ErrorAction SilentlyContinue

Write-Output "Viewer first frame ($Runs runs)..."
$viewerRuns = @()
foreach ($i in 1..$Runs) {
    $dir = Join-Path $work "viewer$i"
    New-Item -ItemType Directory $dir | Out-Null
    $out = Join-Path $dir 'probe.json'
    $p = Start-Process -FilePath $Exe -ArgumentList @('--viewer', '--viewer-probe', '5', '--perf-probe-out', "`"$out`"", '--data-dir', "`"$dir`"") -PassThru -Wait
    if ($p.ExitCode -eq 0 -and (Test-Path $out)) {
        $viewerRuns += (Get-Content $out -Raw | ConvertFrom-Json)
    } else {
        $viewerRuns += [pscustomobject]@{ role = 'viewer'; error = "exit code $($p.ExitCode)" }
    }
}
$firstFrames = @($viewerRuns | Where-Object { $_.PSObject.Properties.Name -contains 'first_frame_us' } | ForEach-Object { [double]$_.first_frame_us })

$version = (Get-Item $Exe).VersionInfo.ProductVersion
if (-not $version) { $version = 'unknown' }
$date = $started.ToString('yyyy-MM-dd')
if (-not $OutFile) { $OutFile = Join-Path $PSScriptRoot "perf-baremetal-$version-$date.json" }

$notMeasured = @(
    [ordered]@{ what = 'engine ready, cold'; why = 'needs a standby-list flush (elevated) and is only an approximation of a reboot' },
    [ordered]@{ what = 'export of 1M rows'; why = 'no harness scenario yet' }
)
if (-not $Driver) {
    $notMeasured += [ordered]@{ what = 'ingest, event latency p50/p99, handle stability, viewer frame times p95/p99/max scrolling 1M rows'; why = 'run without -Driver (perf-driver.exe of the peekbuddy-perf-harness artifact)' }
}

$report = [ordered]@{
    script        = 'perf-baremetal.ps1'
    schemaVersion = 1
    startedUtc    = $started.ToUniversalTime().ToString('o')
    exeVersion    = $version
    exeSha256     = (Get-FileHash -Algorithm SHA256 -Path $Exe).Hash.ToLowerInvariant()
    preflight     = $preflight
    runs          = $Runs
    harness       = $hr
    viewer        = [ordered]@{
        runs                = $viewerRuns
        firstFrameMedianUs  = Get-Median $firstFrames
        firstFrameMinUs     = if ($firstFrames.Count) { ($firstFrames | Measure-Object -Minimum).Minimum } else { $null }
        firstFrameMaxUs     = if ($firstFrames.Count) { ($firstFrames | Measure-Object -Maximum).Maximum } else { $null }
    }
    notMeasured   = $notMeasured
    durationSec   = [math]::Round(((Get-Date) - $started).TotalSeconds, 1)
}
$report | ConvertTo-Json -Depth 12 | Set-Content -Path $OutFile -Encoding utf8
Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
Write-Output "Report written to $OutFile"
