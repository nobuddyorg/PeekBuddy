<#
.SYNOPSIS
    Runs the viewer-bench scenarios (spike #33) on a real GPU.
.DESCRIPTION
    Needs no Rust toolchain, no elevation and makes no persistent changes.
    Download the `viewer-bench-exes` artifact of the latest viewer-bench
    workflow run (it holds wgpu\viewer-bench.exe and, on x64, glow\viewer-bench.exe),
    unzip it and pass the paths. Takes about 8 minutes. Close other windows
    first: the scenarios open a 900x600 window and must not be disturbed.
    Writes viewer-bench-report.json next to the script unless -OutFile is given.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Wgpu,
    [string]$Glow,
    [string]$OutFile = (Join-Path $PSScriptRoot 'viewer-bench-report.json')
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$started = Get-Date
$rows = Join-Path ([IO.Path]::GetTempPath()) "viewer-bench-$([guid]::NewGuid()).jsonl"

function Invoke-Bench([string]$Exe, [string[]]$BenchArgs, [int]$TimeoutSec, [string]$Tag) {
    $err = "$rows.err"
    # Start-Process joins the array without quoting, and the temp path may contain spaces.
    $argLine = ($BenchArgs | ForEach-Object { if ($_ -match '\s') { '"' + $_ + '"' } else { $_ } }) -join ' '
    $p = Start-Process -FilePath $Exe -ArgumentList $argLine -PassThru -NoNewWindow -RedirectStandardError $err -RedirectStandardOutput "$rows.out"
    $null = $p.Handle # keeps ExitCode readable in Windows PowerShell 5.1
    if (-not $p.WaitForExit($TimeoutSec * 1000)) {
        $p.Kill($true)
        Add-Content $rows ('{"experiment":"timeout","label":"' + $Tag + '"}')
    } elseif ($p.ExitCode -ne 0) {
        $msg = (([string](Get-Content $err -Raw)) -replace '[\r\n"\\]+', ' ')
        Add-Content $rows ('{"experiment":"exit","label":"' + $Tag + '","code":' + $p.ExitCode + ',"stderr":"' + $msg.Substring(0, [math]::Min(400, $msg.Length)) + '"}')
    }
}

$variants = @(@{ name = 'wgpu'; exe = $Wgpu })
if ($Glow) { $variants += @{ name = 'glow'; exe = $Glow } }
foreach ($v in $variants) {
    $tag = "owner/$($v.name)"
    Invoke-Bench $v.exe @('--scenario', 'engine', '--secs', '3', '--label', $tag, '--out', $rows) 30 $tag
    1..5 | ForEach-Object { Invoke-Bench $v.exe @('--scenario', 'first-frame', '--label', "$tag/run$_", '--out', $rows) 90 "$tag/run$_" }
    foreach ($s in @(@('idle', 30), @('scroll', 30), @('tail', 30), @('wiggle', 20))) {
        Invoke-Bench $v.exe @('--scenario', $s[0], '--secs', "$($s[1])", '--label', $tag, '--out', $rows) 180 "$tag/$($s[0])"
    }
    Invoke-Bench $v.exe @('--scenario', 'scroll', '--secs', '20', '--latency-ms', '5', '--label', "$tag/latency5", '--out', $rows) 180 "$tag/latency5"
}

$os = Get-CimInstance -ClassName Win32_OperatingSystem
$gpu = @(Get-CimInstance -ClassName Win32_VideoController | ForEach-Object { $_.Name })
$results = if (Test-Path $rows) { @(Get-Content $rows | ForEach-Object { $_ | ConvertFrom-Json }) } else { @() }
$report = [ordered]@{
    script        = 'viewer-bench.ps1'
    schemaVersion = 1
    startedUtc    = $started.ToUniversalTime().ToString('o')
    os            = [ordered]@{ caption = $os.Caption; version = $os.Version; build = $os.BuildNumber }
    architecture  = $env:PROCESSOR_ARCHITECTURE
    gpus          = $gpu
    results       = $results
    durationSec   = [math]::Round(((Get-Date) - $started).TotalSeconds, 1)
}
$report | ConvertTo-Json -Depth 8 | Set-Content -Path $OutFile -Encoding utf8
Remove-Item "$rows*" -ErrorAction SilentlyContinue
Write-Output "Report written to $OutFile"
