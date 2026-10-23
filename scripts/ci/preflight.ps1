<#
.SYNOPSIS
    Fails fast when a hosted Windows runner cannot run PeekBuddy's UI and tray tests.
.DESCRIPTION
    Asserts an interactive console session (session id != 0), a visible
    window station (WSF_VISIBLE, which .NET reports as
    Environment.UserInteractive) and a running explorer.exe. Prints image
    version, Defender state, timezone, resolution and runner size. Spike #34;
    reused by #11, #27 and #71. Budget: <= 10 s, so no Add-Type compilation.
#>
[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$started = Get-Date

$failures = [System.Collections.Generic.List[string]]::new()

$sessionId = (Get-Process -Id $PID).SessionId
if ($sessionId -eq 0) { $failures.Add('process runs in session 0 (no interactive desktop)') }

# UserInteractive is GetUserObjectInformation(GetProcessWindowStation(), UOI_FLAGS) & WSF_VISIBLE.
$interactive = [Environment]::UserInteractive
if (-not $interactive) { $failures.Add('window station is not visible (WSF_VISIBLE not set)') }

$explorer = @(Get-Process -Name explorer -ErrorAction SilentlyContinue)
if ($explorer.Count -eq 0) { $failures.Add('explorer.exe is not running (no tray)') }

$defender = 'unknown'
try {
    $mp = Get-MpComputerStatus
    $defender = "realtime=$($mp.RealTimeProtectionEnabled) tamper=$($mp.IsTamperProtected)"
} catch { $defender = 'unavailable' }

$resolution = 'unknown'
try {
    $video = Get-CimInstance -ClassName Win32_VideoController | Select-Object -First 1
    $resolution = "$($video.CurrentHorizontalResolution)x$($video.CurrentVerticalResolution)"
} catch { $resolution = 'unavailable' }

$os = Get-CimInstance -ClassName Win32_OperatingSystem
$info = [ordered]@{
    image        = "$env:ImageOS $env:ImageVersion"
    os           = "$($os.Caption) build $($os.BuildNumber)"
    architecture = $env:PROCESSOR_ARCHITECTURE
    cpus         = [Environment]::ProcessorCount
    memoryGiB    = [math]::Round($os.TotalVisibleMemorySize / 1MB, 1)
    session      = $sessionId
    interactive  = $interactive
    explorer     = $explorer.Count
    defender     = $defender
    timezone     = (Get-TimeZone).Id
    resolution   = $resolution
}
$info.GetEnumerator() | ForEach-Object { Write-Output ('{0,-13} {1}' -f $_.Key, $_.Value) }
Write-Output ('{0,-13} {1:N1} s' -f 'preflight', ((Get-Date) - $started).TotalSeconds)

if ($failures.Count -gt 0) {
    foreach ($failure in $failures) { Write-Output "::error title=preflight::$failure" }
    exit 1
}
