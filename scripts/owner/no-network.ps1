<#
.SYNOPSIS
    Checks that PeekBuddy makes no network connection while it runs (#77).
.DESCRIPTION
    The v1 promise is: no telemetry, no update check, no account, no network.
    Two things back it: `cargo xtask check-release` fails CI when the exe
    names a network library, and this script watches a real run. It starts the
    engine (with its tray icon) and the viewer on a temporary data directory,
    and for -Minutes (default 10) looks every second at the TCP connections,
    TCP listeners and UDP endpoints owned by every PeekBuddy process. Any one
    found is a failure. Needs no elevation (an ETW network trace would; this
    sees what a process holds open, not a packet that was sent and closed
    between two looks, which is why the import check is the other half).

    Makes no persistent change: temporary directories, processes ended.
    Writes no-network-<date>.json next to the script unless -OutFile is given.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Exe,
    [int]$Minutes = 10,
    [string]$OutFile
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$started = Get-Date
$Exe = (Resolve-Path -LiteralPath $Exe).Path
if (-not $OutFile) {
    $here = if ($PSScriptRoot) { $PSScriptRoot } else { Split-Path -Parent $MyInvocation.MyCommand.Path }
    $OutFile = Join-Path $here ("no-network-{0}.json" -f $started.ToString('yyyy-MM-dd'))
}
$work = Join-Path ([IO.Path]::GetTempPath()) "no-network-$([guid]::NewGuid())"
New-Item -ItemType Directory $work | Out-Null

$engine = Start-Process -FilePath $Exe -ArgumentList @('--headless', '--tray', '--data-dir', "`"$work`"") -PassThru -WindowStyle Hidden
Start-Sleep -Seconds 5
# Hidden: the viewer window is the part that renders and may talk to a GPU
# driver or a font service; it must not talk to the network either.
$viewer = Start-Process -FilePath $Exe -ArgumentList @('--viewer', '--data-dir', "`"$work`"") -PassThru
$found = @{}
$looks = 0
$maxProcesses = 0
try {
    $end = (Get-Date).AddMinutes($Minutes)
    while ((Get-Date) -lt $end) {
        $ids = @(Get-Process -Name peekbuddy -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
        $maxProcesses = [math]::Max($maxProcesses, $ids.Count)
        if ($ids.Count -gt 0) {
            $tcp = @(Get-NetTCPConnection -ErrorAction SilentlyContinue | Where-Object { $ids -contains $_.OwningProcess })
            $udp = @(Get-NetUDPEndpoint -ErrorAction SilentlyContinue | Where-Object { $ids -contains $_.OwningProcess })
            foreach ($c in $tcp) { $found["tcp $($c.State) $($c.LocalAddress):$($c.LocalPort) -> $($c.RemoteAddress):$($c.RemotePort) pid $($c.OwningProcess)"] = $true }
            foreach ($u in $udp) { $found["udp $($u.LocalAddress):$($u.LocalPort) pid $($u.OwningProcess)"] = $true }
        }
        $looks++
        Start-Sleep -Seconds 1
    }
} finally {
    Get-Process -Name peekbuddy -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $Exe } | Stop-Process -Force -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}

$report = [ordered]@{
    script        = 'no-network.ps1'
    schemaVersion = 1
    startedUtc    = $started.ToUniversalTime().ToString('o')
    exe           = $Exe
    minutes       = $Minutes
    looks         = $looks
    maxProcesses  = $maxProcesses
    endpointsSeen = @($found.Keys)
    passed        = ($found.Count -eq 0 -and $looks -gt 0 -and $maxProcesses -ge 2)
}
$report | ConvertTo-Json -Depth 4 | Set-Content -Path $OutFile -Encoding utf8
Write-Output ("{0} looks, up to {1} PeekBuddy processes, {2} network endpoints seen: {3}" -f $looks, $maxProcesses, $found.Count, $(if ($report.passed) { 'pass' } else { 'FAIL' }))
foreach ($e in $found.Keys) { Write-Output "  $e" }
Write-Output "Report written to $OutFile"
if (-not $report.passed) { exit 1 }
