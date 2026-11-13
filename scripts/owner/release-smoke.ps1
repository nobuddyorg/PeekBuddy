<#
.SYNOPSIS
    The owner's smoke run of a PeekBuddy release build on a real Windows PC
    (#77, release checklist).
.DESCRIPTION
    Needs no Rust toolchain, no elevation and makes no persistent change: every
    run uses a temporary data directory and ends the processes it started. It
    takes about two minutes. Close PeekBuddy and other heavy programs first.

    What it checks, each as a pass/fail field of the report:
      1. The file: SHA-256, size, version, the Authenticode state (an unsigned
         release says NotSigned; that is expected, see the README) and whether
         the file carries a Mark of the Web.
      2. The engine with a tray icon starts, answers `--ipc-ping`, shows its
         icon in the notification area (found through UI Automation; null when
         the shell does not answer, for example in a service session) and its
         CPU time and bytes read since start are small.
      3. The startup impact: the engine's own `--perf-probe` (ready time, CPU,
         disk read, private bytes) in a fresh data directory.
      4. The viewer opens, paints its first frame and closes with status 0
         (`--viewer-probe`), and which renderer it used.
      5. Microsoft Defender's custom scan (MpCmdRun) of the exe and, with
         -Assets, of the folder of release assets: threats found or not.

    Not covered here: autostart (it needs a real sign-in: `autostart-check.ps1`)
    and the export of a log (covered by the tests; try it by hand once).

    Writes release-smoke-<version>-<date>.json next to the script unless
    -OutFile is given.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Exe,
    [string]$Assets,
    [string]$OutFile
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$started = Get-Date
$Exe = (Resolve-Path -LiteralPath $Exe).Path
$work = Join-Path ([IO.Path]::GetTempPath()) "release-smoke-$([guid]::NewGuid())"
New-Item -ItemType Directory $work | Out-Null

function Get-Check([string]$Name, [bool]$Pass, $Detail) {
    [ordered]@{ name = $Name; pass = $Pass; detail = $Detail }
}

function Invoke-Native([string]$Program, [string[]]$Arguments, [int]$TimeoutSec = 120) {
    # Arguments that hold spaces are quoted; stdout and stderr are discarded.
    $quoted = $Arguments | ForEach-Object { if ($_ -match '\s') { "`"$_`"" } else { $_ } }
    $process = Start-Process -FilePath $Program -ArgumentList $quoted -PassThru -WindowStyle Hidden
    if (-not $process.WaitForExit($TimeoutSec * 1000)) {
        $process.Kill()
        return [pscustomobject]@{ ExitCode = -1; TimedOut = $true }
    }
    [pscustomobject]@{ ExitCode = $process.ExitCode; TimedOut = $false }
}

function Find-TrayIcon {
    # The notification area (and its overflow) exposes each icon as a button
    # named after its tooltip. Null when the shell does not answer.
    try {
        Add-Type -AssemblyName UIAutomationClient
        $root = [System.Windows.Automation.AutomationElement]::RootElement
        $condition = New-Object System.Windows.Automation.PropertyCondition(
            [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
            [System.Windows.Automation.ControlType]::Button)
        foreach ($button in $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $condition)) {
            if ($button.Current.Name -like '*PeekBuddy*') { return $true }
        }
        return $false
    } catch { return $null }
}

$checks = New-Object System.Collections.Generic.List[object]

# 1. The file.
$item = Get-Item -LiteralPath $Exe
$signature = Get-AuthenticodeSignature -LiteralPath $Exe
$zone = $null
try { $zone = Get-Content -LiteralPath $Exe -Stream Zone.Identifier -ErrorAction Stop } catch { }
$version = $item.VersionInfo.ProductVersion
if (-not $version) { $version = 'unknown' }
$file = [ordered]@{
    path          = $Exe
    version       = $version
    sizeBytes     = $item.Length
    sha256        = (Get-FileHash -Algorithm SHA256 -LiteralPath $Exe).Hash.ToLowerInvariant()
    authenticode  = "$($signature.Status)"
    markOfTheWeb  = [bool]$zone
}
$checks.Add((Get-Check 'file.size_within_budget' ($item.Length -le 14MB) "$($item.Length) bytes (hard cap 14 MB)"))

# 2. The engine with a tray icon.
$data = Join-Path $work 'engine'
New-Item -ItemType Directory $data | Out-Null
$engine = Start-Process -FilePath $Exe -ArgumentList @('--headless', '--tray', '--data-dir', "`"$data`"") -PassThru -WindowStyle Hidden
try {
    $ping = $null
    for ($i = 0; $i -lt 60; $i++) {
        Start-Sleep -Milliseconds 500
        if ($engine.HasExited) { break }
        $ping = (Invoke-Native $Exe @('--ipc-ping', '--data-dir', $data) 30).ExitCode
        if ($ping -eq 0) { break }
    }
    $checks.Add((Get-Check 'engine.starts_and_answers_ping' (-not $engine.HasExited -and $ping -eq 0) "ping exit code $ping"))
    Start-Sleep -Seconds 5
    $engine.Refresh()
    $tray = Find-TrayIcon
    $checks.Add((Get-Check 'engine.tray_icon_in_notification_area' ($tray -eq $true) $(if ($null -eq $tray) { 'the shell did not answer UI Automation' } else { "found: $tray" })))
    $cim = Get-CimInstance Win32_Process -Filter "ProcessId = $($engine.Id)"
    $cpuMs = [math]::Round($engine.TotalProcessorTime.TotalMilliseconds)
    $readBytes = if ($cim) { [int64]$cim.ReadTransferCount } else { $null }
    $children = @(Get-CimInstance Win32_Process -Filter "ParentProcessId = $($engine.Id)").Count
    $checks.Add((Get-Check 'engine.no_viewer_child_and_low_cpu' ($children -eq 0 -and $cpuMs -lt 2000) "children $children, CPU $cpuMs ms, read $readBytes bytes"))
} finally {
    if (-not $engine.HasExited) { Stop-Process -Id $engine.Id -Force -ErrorAction SilentlyContinue }
}

# 3. The startup impact.
$probeOut = Join-Path $work 'engine-probe.json'
$probeData = Join-Path $work 'probe'
New-Item -ItemType Directory $probeData | Out-Null
$run = Invoke-Native $Exe @('--headless', '--data-dir', $probeData, '--perf-probe', '5', '--perf-probe-out', $probeOut) 120
$engineProbe = if (Test-Path $probeOut) { Get-Content $probeOut -Raw | ConvertFrom-Json } else { $null }
$checks.Add((Get-Check 'engine.perf_probe_runs' ($run.ExitCode -eq 0 -and $null -ne $engineProbe) "exit code $($run.ExitCode)"))

# 4. The viewer.
$viewerOut = Join-Path $work 'viewer-probe.json'
$viewerData = Join-Path $work 'viewer'
New-Item -ItemType Directory $viewerData | Out-Null
$run = Invoke-Native $Exe @('--viewer', '--viewer-probe', '5', '--perf-probe-out', $viewerOut, '--data-dir', $viewerData) 180
$viewerProbe = if (Test-Path $viewerOut) { Get-Content $viewerOut -Raw | ConvertFrom-Json } else { $null }
$painted = $null -ne $viewerProbe -and $null -ne $viewerProbe.first_frame_us
$checks.Add((Get-Check 'viewer.opens_paints_and_closes' ($run.ExitCode -eq 0 -and $painted) "exit code $($run.ExitCode), first frame $(if ($painted) { $viewerProbe.first_frame_us } else { 'none' }) us"))

# 5. Defender.
$scanPaths = @($Exe)
if ($Assets) { $scanPaths += (Resolve-Path -LiteralPath $Assets).Path }
$defender = [ordered]@{}
$mpcmd = Join-Path $env:ProgramFiles 'Windows Defender\MpCmdRun.exe'
foreach ($path in $scanPaths) {
    if (Test-Path $mpcmd) {
        # -DisableRemediation: report only, never quarantine or delete.
        $scan = Invoke-Native $mpcmd @('-Scan', '-ScanType', '3', '-File', $path, '-DisableRemediation') 600
        $defender[$path] = $scan.ExitCode
        $checks.Add((Get-Check "defender.scan:$([IO.Path]::GetFileName($path))" ($scan.ExitCode -eq 0) "MpCmdRun exit code $($scan.ExitCode) (0: no threat found)"))
    } else {
        $defender[$path] = $null
        $checks.Add((Get-Check "defender.scan:$([IO.Path]::GetFileName($path))" $false 'MpCmdRun.exe not found (Defender is not the active antivirus?)'))
    }
}

$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$elevated = ([Security.Principal.WindowsPrincipal]::new($identity)).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
$os = Get-CimInstance -ClassName Win32_OperatingSystem
$report = [ordered]@{
    script       = 'release-smoke.ps1'
    schemaVersion = 1
    startedUtc   = $started.ToUniversalTime().ToString('o')
    machine      = [ordered]@{ os = $os.Caption; build = $os.BuildNumber; architecture = $env:PROCESSOR_ARCHITECTURE; elevated = $elevated }
    file         = $file
    checks       = $checks.ToArray()
    enginePerfProbe = $engineProbe
    viewerProbe  = $viewerProbe
    defenderExitCodes = $defender
    allPassed    = -not ($checks | Where-Object { -not $_.pass })
    notChecked   = @(
        [ordered]@{ what = 'autostart after a real sign-in'; why = 'autostart-check.ps1 (three phases around a sign-out)' },
        [ordered]@{ what = 'export of a log'; why = 'covered by the tests; try it by hand once' }
    )
    durationSec  = [math]::Round(((Get-Date) - $started).TotalSeconds, 1)
}
if (-not $OutFile) { $OutFile = Join-Path $PSScriptRoot "release-smoke-$version-$($started.ToString('yyyy-MM-dd')).json" }
$report | ConvertTo-Json -Depth 10 | Set-Content -Path $OutFile -Encoding utf8
Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
foreach ($check in $checks) { Write-Output ('{0,-6} {1}  ({2})' -f $(if ($check.pass) { 'pass' } else { 'FAIL' }), $check.name, $check.detail) }
Write-Output "Report written to $OutFile"
if (-not $report.allPassed) { exit 1 }
