<#
.SYNOPSIS
    Runs the store-bench scenarios (spike #35) on the owner's disk.
.DESCRIPTION
    Needs no Rust toolchain and no elevation. Download the
    `store-bench-exes-windows-2025` artifact of the store-bench workflow,
    unzip it and pass the path of store-bench.exe. The data goes to a fresh
    directory under the temp folder (or -DataDir) and is deleted at the end;
    nothing else on the machine changes. Defender stays as it is (the report
    records its state): excluding the folder would be a persistent change.
    Takes 10 to 15 minutes; the query experiment runs with 2M rows. Close
    other heavy programs first. Writes store-bench-report.json next to the
    script unless -OutFile is given.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Exe,
    [string]$DataDir = (Join-Path ([IO.Path]::GetTempPath()) "store-bench-$([guid]::NewGuid())"),
    [int]$QueryRows = 2000000,
    [string]$OutFile = (Join-Path $PSScriptRoot 'store-bench-report.json')
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$started = Get-Date
$tmp = Join-Path ([IO.Path]::GetTempPath()) "store-bench-run-$([guid]::NewGuid())"
New-Item -ItemType Directory -Force -Path $tmp, $DataDir | Out-Null
$rows = Join-Path $tmp 'rows.jsonl'
$label = 'owner'

function Invoke-Bench([string[]]$BenchArgs, [int]$TimeoutSec, [string]$Tag) {
    $argLine = ($BenchArgs + @('--dir', $DataDir, '--label', $label) | ForEach-Object { if ($_ -match '\s') { '"' + $_ + '"' } else { $_ } }) -join ' '
    $out = Join-Path $tmp 'out.txt'
    $err = Join-Path $tmp 'err.txt'
    $p = Start-Process -FilePath $Exe -ArgumentList $argLine -PassThru -NoNewWindow -RedirectStandardOutput $out -RedirectStandardError $err
    $null = $p.Handle # keeps ExitCode readable in Windows PowerShell 5.1
    if (-not $p.WaitForExit($TimeoutSec * 1000)) {
        $p.Kill()
        Add-Content $rows ('{"experiment":"timeout","cell":"' + $Tag + '"}')
    } elseif ($p.ExitCode -ne 0) {
        $msg = ([string](Get-Content $err -Raw)) -replace '[\r\n"\\]+', ' '
        Add-Content $rows ('{"experiment":"exit","cell":"' + $Tag + '","code":' + $p.ExitCode + ',"stderr":"' + $msg.Substring(0, [math]::Min(400, $msg.Length)) + '"}')
    }
    if (Test-Path $out) { Get-Content $out | Where-Object { $_.StartsWith('{') } | Add-Content $rows }
    Remove-Item $out, $err -ErrorAction SilentlyContinue
}

try {
    foreach ($e in 'size', 'checkpoint', 'commit', 'readers') { Invoke-Bench @($e) 600 $e }
    foreach ($cache in 16000, 4000, 2000) { Invoke-Bench @('memory', '--cache-kib', "$cache") 300 "memory-$cache" }
    Invoke-Bench @('query', '--rows', "$QueryRows") 1200 'query'

    $os = Get-CimInstance -ClassName Win32_OperatingSystem
    $cpu = Get-CimInstance -ClassName Win32_Processor | Select-Object -First 1
    $drive = (Get-Item $DataDir).PSDrive.Name
    $part = Get-Partition -DriveLetter $drive -ErrorAction SilentlyContinue
    $disk = if ($part) { Get-PhysicalDisk | Where-Object { $_.DeviceId -eq $part.DiskNumber } | Select-Object -First 1 }
    $defender = try { (Get-MpComputerStatus).RealTimeProtectionEnabled } catch { $null }
    $excluded = try { @((Get-MpPreference).ExclusionPath) -contains $DataDir } catch { $null }
    $results = if (Test-Path $rows) { @(Get-Content $rows | ForEach-Object { $_ | ConvertFrom-Json }) } else { @() }
    $report = [ordered]@{
        script        = 'store-bench.ps1'
        schemaVersion = 1
        startedUtc    = $started.ToUniversalTime().ToString('o')
        os            = [ordered]@{ caption = $os.Caption; version = $os.Version; build = $os.BuildNumber }
        architecture  = $env:PROCESSOR_ARCHITECTURE
        cpu           = $cpu.Name
        memoryGiB     = [math]::Round($os.TotalVisibleMemorySize / 1MB, 1)
        dataDrive     = $drive
        fileSystem    = (Get-Volume -DriveLetter $drive).FileSystem
        disk          = if ($disk) { [ordered]@{ name = $disk.FriendlyName; media = "$($disk.MediaType)"; bus = "$($disk.BusType)" } } else { $null }
        defender      = [ordered]@{ realTime = $defender; dataDirExcluded = $excluded }
        queryRows     = $QueryRows
        results       = $results
        durationSec   = [math]::Round(((Get-Date) - $started).TotalSeconds, 1)
    }
    $report | ConvertTo-Json -Depth 8 | Set-Content -Path $OutFile -Encoding UTF8
    Write-Host "Wrote $OutFile"
} finally {
    Remove-Item $tmp, $DataDir -Recurse -Force -ErrorAction SilentlyContinue
}
