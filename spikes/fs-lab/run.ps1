# Spike #36 orchestrator: builds the volumes and runs every scenario that a
# hosted runner can reproduce, three repetitions each. Output: one JSONL file
# per file system and scenario under -Out.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Exe,
    [Parameter(Mandatory)][string]$Out,
    [string]$Label = 'local',
    [string]$Work = 'C:\fslab-work',
    [int]$IdleSeconds = 600
)
$ErrorActionPreference = 'Continue'
New-Item -ItemType Directory -Force -Path $Work, $Out | Out-Null
$Summary = Join-Path $Out 'summary.jsonl'

function Add-Row($obj) { ($obj | ConvertTo-Json -Compress -Depth 5) | Add-Content -Path $Summary -Encoding utf8 }

function New-TestVolume($Name, $Fs, $SizeMB, $Letter) {
    $vhd = Join-Path $Work "$Name.vhdx"
    $script = "create vdisk file=`"$vhd`" maximum=$SizeMB type=expandable`nselect vdisk file=`"$vhd`"`nattach vdisk`ncreate partition primary`nformat fs=$Fs quick label=LAB$Name`nassign letter=$Letter"
    $dp = Join-Path $Work "dp-$Name.txt"
    Set-Content -Path $dp -Value $script -Encoding ascii
    $log = (diskpart /s $dp 2>&1 | Out-String)
    $ok = Test-Path "${Letter}:\"
    Add-Row @{ kind = 'volume'; name = $Name; fs = $Fs; ok = $ok; log = $log.Trim() }
    # A new volume has no change journal; USN needs one (NTFS and ReFS only).
    if ($ok -and $Fs -in 'ntfs', 'refs') { fsutil usn createjournal m=33554432 a=8388608 "${Letter}:" | Out-Null }
    return $ok
}

function New-DevDrive($Letter) {
    $vhd = Join-Path $Work 'devdrive.vhdx'
    Set-Content -Path (Join-Path $Work 'dp-dev.txt') -Value "create vdisk file=`"$vhd`" maximum=51200 type=expandable`nselect vdisk file=`"$vhd`"`nattach vdisk" -Encoding ascii
    diskpart /s (Join-Path $Work 'dp-dev.txt') | Out-Null
    try {
        $disk = Get-DiskImage -ImagePath $vhd | Get-Disk
        Initialize-Disk -Number $disk.Number -PartitionStyle GPT -ErrorAction Stop
        New-Partition -DiskNumber $disk.Number -UseMaximumSize -DriveLetter $Letter -ErrorAction Stop | Out-Null
        Format-Volume -DriveLetter $Letter -DevDrive -FileSystem ReFS -Confirm:$false -ErrorAction Stop | Out-Null
        fsutil usn createjournal m=33554432 a=8388608 "${Letter}:" | Out-Null
        Add-Row @{ kind = 'volume'; name = 'devdrive'; ok = $true }
        return $true
    } catch {
        Add-Row @{ kind = 'volume'; name = 'devdrive'; ok = $false; error = "$_" }
        return $false
    }
}

$os = Get-CimInstance Win32_OperatingSystem
Add-Row @{ kind = 'env'; label = $Label; caption = $os.Caption; version = $os.Version; ubr = (Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion').UBR; arch = $env:PROCESSOR_ARCHITECTURE }

# name -> @{ letter; scenarios; settle }
$ntfsAll = 'N1', 'N2', 'N3', 'N4', 'L1', 'L2', 'L3', 'L4', 'S1'
$vols = [ordered]@{}
$vols['c'] = @{ letter = 'C'; dir = 'C:\fslab-c'; scenarios = @('N2', 'N4', 'L1', 'L3', 'S1'); settle = 3 }
if (New-TestVolume 'ntfs' 'ntfs' 1024 'P') { $vols['ntfs'] = @{ letter = 'P'; dir = 'P:\fslab'; scenarios = $ntfsAll; settle = 3 } }
if (New-TestVolume 'refs' 'refs' 4096 'Q') { $vols['refs'] = @{ letter = 'Q'; dir = 'Q:\fslab'; scenarios = @('N2', 'N4', 'L1', 'L3', 'S1', 'D1'); settle = 35 } }
if (New-DevDrive 'T') { $vols['devdrive'] = @{ letter = 'T'; dir = 'T:\fslab'; scenarios = @('N2', 'S1', 'D1'); settle = 35 } }
if (New-TestVolume 'exfat' 'exfat' 512 'R') { $vols['exfat'] = @{ letter = 'R'; dir = 'R:\fslab'; scenarios = @('N2', 'N4', 'S1'); settle = 3 } }
if (New-TestVolume 'fat32' 'fat32' 512 'S') { $vols['fat32'] = @{ letter = 'S'; dir = 'S:\fslab'; scenarios = @('N1', 'N2', 'N4'); settle = 3 } }

foreach ($name in $vols.Keys) {
    $v = $vols[$name]
    New-Item -ItemType Directory -Force -Path $v.dir | Out-Null
    $fsOut = Join-Path $Out $name
    New-Item -ItemType Directory -Force -Path $fsOut | Out-Null
    foreach ($id in $v.scenarios) {
        $file = Join-Path $fsOut "$id.jsonl"
        $sw = [Diagnostics.Stopwatch]::StartNew()
        $p = Start-Process -FilePath $Exe -ArgumentList @('run', '--scenario', $id, '--dir', $v.dir, '--volume', "$($v.letter):", '--out', $file, '--settle-s', "$($v.settle)", '--label', "$Label/$name") -PassThru -NoNewWindow
        if (-not $p.WaitForExit(900000)) { $p.Kill($true); Add-Row @{ kind = 'timeout'; fs = $name; scenario = $id } }
        $lines = if (Test-Path $file) { (Get-Content $file | Measure-Object).Count } else { 0 }
        Add-Row @{ kind = 'scenario'; fs = $name; scenario = $id; exit = $p.ExitCode; seconds = [math]::Round($sw.Elapsed.TotalSeconds, 1); lines = $lines }
    }
}

# B1: idle observation on the system volume and the NTFS VHD.
foreach ($name in 'c', 'ntfs') {
    if (-not $vols.Contains($name)) { continue }
    $v = $vols[$name]
    $file = Join-Path (Join-Path $Out $name) 'B1.jsonl'
    New-Item -ItemType Directory -Force -Path (Split-Path $file) | Out-Null
    $p = Start-Process -FilePath $Exe -ArgumentList @('run', '--scenario', 'B1', '--dir', $v.dir, '--volume', "$($v.letter):", '--out', $file, '--settle-s', "$IdleSeconds", '--reps', '1', '--label', "$Label/$name") -PassThru -NoNewWindow
    if (-not $p.WaitForExit(($IdleSeconds + 600) * 1000)) { $p.Kill($true) }
    Add-Row @{ kind = 'scenario'; fs = $name; scenario = 'B1'; exit = $p.ExitCode; lines = (Get-Content $file | Measure-Object).Count }
}

foreach ($name in 'ntfs', 'refs', 'exfat', 'fat32', 'devdrive') {
    $vhd = Join-Path $Work "$name.vhdx"
    if (Test-Path $vhd) {
        Set-Content -Path (Join-Path $Work "dp-detach-$name.txt") -Value "select vdisk file=`"$vhd`"`ndetach vdisk" -Encoding ascii
        diskpart /s (Join-Path $Work "dp-detach-$name.txt") | Out-Null
    }
}
