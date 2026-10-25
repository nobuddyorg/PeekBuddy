<#
.SYNOPSIS
    Runs the rdcw-bench scenarios (spike #32) on the owner's local NTFS disk.
.DESCRIPTION
    Needs no Rust toolchain and no elevation. Download the
    `rdcw-bench-exe-windows-2025` artifact of the rdcw-bench workflow, unzip
    it and pass the path of rdcw-bench.exe. Scratch files go to a fresh folder
    under the temp directory and are deleted at the end; nothing else changes
    and no cloud, network or removable location is touched. Runs the info
    class x buffer matrix, the root lifecycle, the throughput cells with five
    repetitions each (for the coefficient of variation), a few burst cells and
    the pool cost. It also records file name lengths of the user profile as
    aggregate numbers only (no names are stored): reparse points, which
    includes OneDrive placeholders, are skipped. Takes about 12 minutes. Close
    other heavy programs first. Writes rdcw-bench-report.json next to the
    script unless -OutFile is given.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Exe,
    [string]$WorkDir = (Join-Path ([IO.Path]::GetTempPath()) "rdcw-bench-$([guid]::NewGuid())"),
    [int]$Reps = 5,
    [int]$NameSampleLimit = 300000,
    [string]$OutFile = (Join-Path $PSScriptRoot 'rdcw-bench-report.json')
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$started = Get-Date
New-Item -ItemType Directory -Force -Path $WorkDir | Out-Null
$rows = Join-Path $WorkDir 'rows.jsonl'
$bench = Join-Path $WorkDir 'rdcw-bench.exe'
Copy-Item $Exe $bench
$dir = Join-Path $WorkDir 'watch'
New-Item -ItemType Directory -Force -Path $dir | Out-Null

function Invoke-Bench([string[]]$BenchArgs, [int]$TimeoutSec, [string]$Tag) {
    $argLine = ($BenchArgs | ForEach-Object { if ($_ -match '\s') { '"' + $_ + '"' } else { $_ } }) -join ' '
    $out = Join-Path $WorkDir 'out.txt'
    $err = Join-Path $WorkDir 'err.txt'
    $env:BENCH_LABEL = 'owner'
    $p = Start-Process -FilePath $bench -ArgumentList $argLine -PassThru -NoNewWindow -RedirectStandardOutput $out -RedirectStandardError $err
    $null = $p.Handle # keeps ExitCode readable in Windows PowerShell 5.1
    if (-not $p.WaitForExit($TimeoutSec * 1000)) {
        $p.Kill()
        Add-Content $rows ('{"cell":"timeout","tag":"' + $Tag + '"}')
    } elseif ($p.ExitCode -ne 0) {
        $msg = ([string](Get-Content $err -Raw)) -replace '[\r\n"\\]+', ' '
        Add-Content $rows ('{"cell":"exit","tag":"' + $Tag + '","code":' + $p.ExitCode + ',"stderr":"' + $msg.Substring(0, [math]::Min(400, $msg.Length)) + '"}')
    }
    if (Test-Path $out) { Get-Content $out | Where-Object { $_.StartsWith('{') } | Add-Content $rows }
    Remove-Item $out, $err -ErrorAction SilentlyContinue
}

function Get-NameStats {
    $opts = [IO.EnumerationOptions]::new()
    $opts.RecurseSubdirectories = $true
    $opts.IgnoreInaccessible = $true
    $opts.AttributesToSkip = [IO.FileAttributes]::ReparsePoint -bor [IO.FileAttributes]::System
    $lengths = [Collections.Generic.List[int]]::new()
    $roots = @($env:USERPROFILE, $env:ProgramData, $env:ProgramFiles) | Where-Object { $_ -and (Test-Path $_) }
    $deadline = (Get-Date).AddSeconds(120)
    foreach ($root in $roots) {
        try {
            foreach ($entry in [IO.Directory]::EnumerateFileSystemEntries($root, '*', $opts)) {
                $lengths.Add([IO.Path]::GetFileName($entry).Length)
                if ($lengths.Count -ge $NameSampleLimit -or (Get-Date) -gt $deadline) { break }
            }
        } catch { }
        if ($lengths.Count -ge $NameSampleLimit -or (Get-Date) -gt $deadline) { break }
    }
    if ($lengths.Count -eq 0) { return [ordered]@{ n = 0 } }
    $sorted = $lengths.ToArray() | Sort-Object
    $at = { param($q) $sorted[[math]::Min($sorted.Count - 1, [int][math]::Floor(($sorted.Count - 1) * $q))] }
    $sum = 0L; foreach ($l in $sorted) { $sum += $l }
    return [ordered]@{
        n = $sorted.Count; meanUtf16Units = [math]::Round($sum / $sorted.Count, 2)
        p50 = (& $at 0.5); p90 = (& $at 0.9); p99 = (& $at 0.99); max = $sorted[-1]
        roots = @($roots | ForEach-Object { Split-Path $_ -Leaf })
    }
}

try {
    Invoke-Bench @('matrix', '--dir', $dir) 300 'matrix'
    Invoke-Bench @('lifecycle', '--dir', $dir) 120 'lifecycle'
    foreach ($mode in 'single', 'double') {
        foreach ($size in 65536, 1048576) {
            foreach ($rate in 20000, 50000, 100000) {
                1..$Reps | ForEach-Object {
                    Invoke-Bench @('perf', '--dir', $dir, '--size', "$size", '--rate', "$rate", '--secs', '5', '--mode', $mode) 60 "perf-$mode-$size-$rate-$_"
                }
            }
        }
    }
    foreach ($size in 65536, 1048576) {
        foreach ($stall in 0, 50) { Invoke-Bench @('burst', '--dir', $dir, '--size', "$size", '--stall-ms', "$stall", '--ops', '50000') 120 "burst-$size-$stall" }
    }
    foreach ($n in 1, 10, 100) { Invoke-Bench @('pool', '--dir', $dir, '--size', '1048576', '--n', "$n") 60 "pool-$n" }

    $names = Get-NameStats
    $os = Get-CimInstance -ClassName Win32_OperatingSystem
    $cpu = Get-CimInstance -ClassName Win32_Processor | Select-Object -First 1
    $drive = (Get-Item $WorkDir).PSDrive.Name
    $part = Get-Partition -DriveLetter $drive -ErrorAction SilentlyContinue
    $disk = if ($part) { Get-PhysicalDisk | Where-Object { $_.DeviceId -eq $part.DiskNumber } | Select-Object -First 1 }
    $defender = try { (Get-MpComputerStatus).RealTimeProtectionEnabled } catch { $null }
    $results = if (Test-Path $rows) { @(Get-Content $rows | ForEach-Object { $_ | ConvertFrom-Json }) } else { @() }
    $report = [ordered]@{
        script        = 'rdcw-bench.ps1'
        schemaVersion = 1
        startedUtc    = $started.ToUniversalTime().ToString('o')
        os            = [ordered]@{ caption = $os.Caption; version = $os.Version; build = $os.BuildNumber }
        architecture  = $env:PROCESSOR_ARCHITECTURE
        cpu           = $cpu.Name
        logicalCpus   = [Environment]::ProcessorCount
        dataDrive     = $drive
        fileSystem    = (Get-Volume -DriveLetter $drive).FileSystem
        disk          = if ($disk) { [ordered]@{ name = $disk.FriendlyName; media = "$($disk.MediaType)"; bus = "$($disk.BusType)" } } else { $null }
        defenderRealTime = $defender
        reps          = $Reps
        nameLengths   = $names
        results       = $results
        durationSec   = [math]::Round(((Get-Date) - $started).TotalSeconds, 1)
    }
    $report | ConvertTo-Json -Depth 8 | Set-Content -Path $OutFile -Encoding UTF8
    Write-Host "Wrote $OutFile"
} finally {
    Remove-Item $WorkDir -Recurse -Force -ErrorAction SilentlyContinue
}
