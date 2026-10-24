# Spike #32 orchestrator: builds scratch volumes and an SMB loopback share, then
# runs every rdcw-bench experiment. Needs an elevated session (hosted runners
# are admin). -Rest skips the matrix, lifecycle and throughput sweeps and runs
# the remaining burst cells, the pool cost and the detach case only. Appends JSONL to $OutDir\<label>-all.jsonl.
param(
    [Parameter(Mandatory)] [string] $Exe,
    [Parameter(Mandatory)] [string] $Label,
    [string] $OutDir = 'C:\rdcw-bench\out',
    [string] $Work = 'D:\rdcw-vhd',
    [int] $Secs = 5,
    [switch] $Rest
)
$ErrorActionPreference = 'Continue'
if (-not (Test-Path 'D:\')) { $Work = Join-Path $env:RUNNER_TEMP 'rdcw-vhd' }
New-Item -ItemType Directory -Force -Path $OutDir, $Work, 'C:\rdcw-bench\bin' | Out-Null
Copy-Item $Exe 'C:\rdcw-bench\bin\rdcw-bench.exe' -Force
$bench = 'C:\rdcw-bench\bin\rdcw-bench.exe'
$all = Join-Path $OutDir "$Label-all.jsonl"

$os = Get-CimInstance Win32_OperatingSystem
([ordered]@{ spike = 'rdcw'; label = $Label; cell = 'os'; caption = $os.Caption; build = $os.BuildNumber; arch = $os.OSArchitecture; cpus = [Environment]::ProcessorCount } | ConvertTo-Json -Compress) | Tee-Object -FilePath $all -Append

function Invoke-Bench([string] $fs, [string[]] $benchArgs) {
    $env:BENCH_LABEL = "$Label/$fs"
    $out = & $bench @benchArgs 2>&1
    $out | ForEach-Object { $_ } | Tee-Object -FilePath $all -Append | Out-Host
}

function New-Volume([string] $name, [string] $letter, [string] $fs) {
    $vhd = Join-Path $Work "$name.vhdx"
    @"
create vdisk file="$vhd" maximum=2048 type=expandable
select vdisk file="$vhd"
attach vdisk
create partition primary
format fs=$fs quick label=$name
assign letter=$letter
"@ | Set-Content (Join-Path $Work "$name.txt") -Encoding ascii
    diskpart /s (Join-Path $Work "$name.txt") | Out-Host
    if (Test-Path "${letter}:\") { return "${letter}:" }
    return $null
}

$targets = @()
$targets += [pscustomobject]@{ Fs = 'ntfs-system'; Dir = 'C:\rdcw-bench\work' }
foreach ($v in @(@('ntfs', 'V', 'ntfs'), @('refs', 'W', 'refs'), @('exfat', 'X', 'exfat'), @('fat32', 'Y', 'fat32'))) {
    $m = New-Volume "vhd-$($v[0])" $v[1] $v[2]
    if ($m) { $targets += [pscustomobject]@{ Fs = $v[0]; Dir = "$m\bench" } }
    else { '{"spike":"rdcw","cell":"volume","name":"' + $v[0] + '","ok":false}' | Tee-Object -FilePath $all -Append }
}

# SMB loopback: a share over the system directory.
$smbRoot = 'C:\rdcw-bench\smb'
New-Item -ItemType Directory -Force -Path $smbRoot | Out-Null
try {
    New-SmbShare -Name rdcwshare -Path $smbRoot -FullAccess Everyone -ErrorAction Stop | Out-Null
    $targets += [pscustomobject]@{ Fs = 'smb'; Dir = '\\localhost\rdcwshare' }
} catch {
    '{"spike":"rdcw","cell":"smb","ok":false,"error":"' + ($_.Exception.Message -replace '["\\]', ' ') + '"}' | Tee-Object -FilePath $all -Append
}

foreach ($t in $targets) {
    if ($Rest) { break }
    New-Item -ItemType Directory -Force -Path $t.Dir -ErrorAction SilentlyContinue | Out-Null
    Invoke-Bench $t.Fs @('matrix', '--dir', $t.Dir)
    if ($t.Fs -in 'ntfs', 'refs', 'ntfs-system') { Invoke-Bench $t.Fs @('lifecycle', '--dir', $t.Dir) }
}

# Throughput, burst and pool numbers on the fast local volume (and SMB for the burst matrix).
$perfDir = ($targets | Where-Object Fs -eq 'ntfs').Dir
if (-not $perfDir) { $perfDir = 'C:\rdcw-bench\work' }
foreach ($mode in 'single', 'double') {
    if ($Rest) { break }
    foreach ($size in 65536, 262144, 1048576, 4194304) {
        foreach ($rate in 5000, 20000, 50000, 100000, 200000) {
            Invoke-Bench 'ntfs' @('perf', '--dir', $perfDir, '--size', $size, '--rate', $rate, '--secs', $Secs, '--mode', $mode)
        }
    }
}
$burstSizes = if ($Rest) { @(1048576, 4194304) } else { @(65536, 262144, 1048576, 4194304) }
$burstOps = if ($Rest) { @(50000) } else { @(10000, 50000, 200000) }
foreach ($size in $burstSizes) {
    foreach ($stall in 0, 10, 50, 100) {
        foreach ($ops in $burstOps) {
            Invoke-Bench 'ntfs' @('burst', '--dir', $perfDir, '--size', $size, '--stall-ms', $stall, '--ops', $ops)
        }
    }
}
foreach ($n in 1, 10, 100) { Invoke-Bench 'ntfs' @('pool', '--dir', $perfDir, '--size', 65536, '--n', $n) }
foreach ($n in 1, 10, 100) { Invoke-Bench 'ntfs' @('pool', '--dir', $perfDir, '--size', 1048576, '--n', $n) }

# Q5: detach the VHD under an armed watch.
$ntfsV = $targets | Where-Object Fs -eq 'ntfs'
if ($ntfsV) {
    $flagDir = 'C:\rdcw-bench\flags'
    New-Item -ItemType Directory -Force -Path $flagDir | Out-Null
    Remove-Item "$flagDir\armed.flag" -ErrorAction SilentlyContinue
    $env:BENCH_LABEL = "$Label/ntfs"
    $job = Start-Job { param($b, $d, $f, $l) $env:BENCH_LABEL = $l; & $b detach --dir $d --flag-dir $f 2>&1 } -ArgumentList $bench, $ntfsV.Dir, $flagDir, "$Label/ntfs"
    $deadline = (Get-Date).AddSeconds(30)
    while (-not (Test-Path "$flagDir\armed.flag") -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 200 }
    "detaching vhd-ntfs" | Out-Host
    "select vdisk file=`"$(Join-Path $Work 'vhd-ntfs.vhdx')`"`ndetach vdisk" | Set-Content (Join-Path $Work 'detach.txt') -Encoding ascii
    diskpart /s (Join-Path $Work 'detach.txt') | Out-Host
    Wait-Job $job -Timeout 90 | Out-Null
    Receive-Job $job | Tee-Object -FilePath $all -Append | Out-Host
    Remove-Job $job -Force
}
