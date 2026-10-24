# Spike #31 orchestrator: builds VHD volumes, a standard user and a restricted
# token, runs usn-probe for every token on every volume and writes one JSONL
# file with every attempt. Runs on the hosted Windows runners (admin, UAC off).
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Exe,
    [string]$Label = 'local',
    [string]$Work = 'C:\usn-spike',
    [int]$Files = 250,
    [int]$TreeDirs = 100000
)
$ErrorActionPreference = 'Continue'
$Share = Join-Path $Work 'share'
$Bin = Join-Path $Share 'bin'
$Out = Join-Path $Share 'results.jsonl'
$Probe = Join-Path $Bin 'usn-probe.exe'
$UserName = 'usnstd'
$UserPw = 'UsnProbe!2026x'

New-Item -ItemType Directory -Force -Path $Bin | Out-Null
Copy-Item $Exe $Probe -Force
icacls $Share /grant 'Everyone:(OI)(CI)F' /T /C /Q | Out-Null
New-Item -ItemType File -Force -Path $Out | Out-Null

function Add-Row($obj) {
    ($obj | ConvertTo-Json -Compress -Depth 6) | Add-Content -Path $Out -Encoding utf8
}

function Capture($label, [scriptblock]$block) {
    try {
        $text = (& $block 2>&1 | Out-String).Trim()
        $lines = ($text -split "`r?`n") | Select-Object -First 80
        Add-Row @{ kind = 'text'; label = $label; lines = $lines }
    } catch {
        Add-Row @{ kind = 'text'; label = $label; error = "$_" }
    }
}

# --------------------------------------------------------------- environment
$os = Get-CimInstance Win32_OperatingSystem
$ubr = (Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion').UBR
Add-Row @{
    kind = 'env'; label = $Label; caption = $os.Caption; version = $os.Version; ubr = $ubr
    arch = $env:PROCESSOR_ARCHITECTURE; product_type = $os.ProductType
    image_version = $env:ImageVersion; image_os = $env:ImageOS
    user = [Environment]::UserName
    uac = (Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System').EnableLUA
}
Capture 'whoami /all' { whoami /all }

# ------------------------------------------------------------------- volumes
function New-TestVolume($Name, $Fs, $SizeMB, $Letter) {
    $vhd = Join-Path $Work "$Name.vhdx"
    $script = @"
create vdisk file="$vhd" maximum=$SizeMB type=expandable
select vdisk file="$vhd"
attach vdisk
create partition primary
format fs=$Fs quick label=USN$Name
assign letter=$Letter
"@
    $dp = Join-Path $Work "dp-$Name.txt"
    Set-Content -Path $dp -Value $script -Encoding ascii
    $log = (diskpart /s $dp 2>&1 | Out-String)
    $ok = Test-Path "${Letter}:\"
    Add-Row @{ kind = 'volume'; name = $Name; fs = $Fs; letter = $Letter; ok = $ok; log = $log.Trim() }
    return $ok
}

function New-DevDrive($Name, $Letter) {
    $vhd = Join-Path $Work "$Name.vhdx"
    $script = "create vdisk file=`"$vhd`" maximum=51200 type=expandable`nselect vdisk file=`"$vhd`"`nattach vdisk"
    $dp = Join-Path $Work "dp-$Name.txt"
    Set-Content -Path $dp -Value $script -Encoding ascii
    diskpart /s $dp | Out-Null
    try {
        $disk = Get-DiskImage -ImagePath $vhd | Get-Disk
        Initialize-Disk -Number $disk.Number -PartitionStyle GPT -ErrorAction Stop
        New-Partition -DiskNumber $disk.Number -UseMaximumSize -DriveLetter $Letter -ErrorAction Stop | Out-Null
        Format-Volume -DriveLetter $Letter -DevDrive -FileSystem ReFS -Confirm:$false -ErrorAction Stop | Out-Null
        Add-Row @{ kind = 'volume'; name = $Name; fs = 'devdrive'; letter = $Letter; ok = $true }
        return $true
    } catch {
        Add-Row @{ kind = 'volume'; name = $Name; fs = 'devdrive'; letter = $Letter; ok = $false; error = "$_" }
        return $false
    }
}

$volumes = [ordered]@{}
$volumes['c'] = @{ letter = 'C'; fs = 'ntfs'; journal = $true; path = 'C:\usn-spike\vol' }
if (New-TestVolume 'ntfs' 'ntfs' 1024 'P') { $volumes['ntfs'] = @{ letter = 'P'; fs = 'ntfs'; journal = $true; path = 'P:' } }
if (New-TestVolume 'refs' 'refs' 2048 'Q') { $volumes['refs'] = @{ letter = 'Q'; fs = 'refs'; journal = $true; path = 'Q:' } }
if (New-TestVolume 'exfat' 'exfat' 512 'R') { $volumes['exfat'] = @{ letter = 'R'; fs = 'exfat'; journal = $false; path = 'R:' } }
if (New-TestVolume 'fat32' 'fat32' 512 'S') { $volumes['fat32'] = @{ letter = 'S'; fs = 'fat32'; journal = $false; path = 'S:' } }
if (New-DevDrive 'devdrive' 'T') { $volumes['devdrive'] = @{ letter = 'T'; fs = 'devdrive'; journal = $true; path = 'T:' } }

New-Item -ItemType Directory -Force -Path 'C:\usn-spike\vol' | Out-Null
foreach ($name in $volumes.Keys) {
    $v = $volumes[$name]
    $root = if ($name -eq 'c') { $v.path } else { "$($v.letter):" }
    $v.root = $root
    if ($v.journal) {
        Capture "fsutil usn createjournal $name" { fsutil usn createjournal m=33554432 a=4194304 "$($v.letter):" }
        Capture "fsutil usn queryjournal $name" { fsutil usn queryjournal "$($v.letter):" }
        Capture "fsutil volumeinfo $name" { fsutil fsinfo volumeinfo "$($v.letter):\" }
        $v.allowed = Join-Path $root 'usn-allowed'
        $v.denied = Join-Path $root 'usn-denied'
        New-Item -ItemType Directory -Force -Path $v.allowed, $v.denied | Out-Null
        icacls $v.allowed /grant 'Everyone:(OI)(CI)F' /Q | Out-Null
        # The denied directory: only Administrators and SYSTEM. The standard user is
        # not in the ACL; the restricted token has Administrators as deny-only.
        icacls $v.denied /inheritance:r /grant 'Administrators:(OI)(CI)F' 'SYSTEM:(OI)(CI)F' /Q | Out-Null
        Capture "icacls denied $name" { icacls $v.denied }
        Set-Content -Path (Join-Path $v.allowed 'fileusn.txt') -Value 'x'
    } else {
        $v.allowed = $null
    }
}
Capture 'fsutil usn queryjournal exfat' { fsutil usn queryjournal 'R:' }
Capture 'fsutil usn queryjournal fat32' { fsutil usn queryjournal 'S:' }

# ---------------------------------------------------------------- the user
net user $UserName $UserPw /add /y | Out-Null
Add-Row @{ kind = 'user'; name = $UserName; exists = [bool](Get-LocalUser -Name $UserName -ErrorAction SilentlyContinue) }

# ----------------------------------------------------------------- running
function Start-Probe($Token, [string[]]$ProbeArgs) {
    switch ($Token) {
        'admin' { $argList = $ProbeArgs }
        'restricted' { $argList = @('spawn-restricted', '--timeout-s', '240', '--', $Probe) + $ProbeArgs }
        'standard' { $argList = @('spawn-as', '--user', $UserName, '--password', $UserPw, '--timeout-s', '240', '--', $Probe) + $ProbeArgs }
    }
    return Start-Process -FilePath $Probe -ArgumentList $argList -PassThru -NoNewWindow -WorkingDirectory $Share
}

function Wait-Probe($p, [int]$Seconds) {
    if (-not $p.WaitForExit($Seconds * 1000)) {
        $p.Kill($true)
        return $false
    }
    return $true
}

function Run-Admin([string[]]$ProbeArgs) {
    $p = Start-Process -FilePath $Probe -ArgumentList $ProbeArgs -PassThru -NoNewWindow -WorkingDirectory $Share
    [void](Wait-Probe $p 600)
}

function Invoke-Cell($Token, $VolName, $Handle, $Fsctl, $Mode) {
    $v = $volumes[$VolName]
    $id = "$Token-$VolName-$Handle-$Fsctl-$Mode"
    $cell = Join-Path $Share "cells\$id"
    New-Item -ItemType Directory -Force -Path $cell | Out-Null
    $tag = "$Token/$VolName/$Handle"
    Run-Admin @('workload', '--mode', 'prepare', '--volume', "$($v.letter):", '--cell', $cell, '--label', "$tag", '--out', $Out)
    $common = @('--volume', "$($v.letter):", '--handle', $Handle, '--fsctl', $Fsctl, '--cell', $cell, '--label', $tag, '--out', $Out)
    $p = Start-Probe $Token (@($Mode) + $common)
    if ($Mode -eq 'overlapped') {
        [void](Wait-Probe $p 120)
        return
    }
    $deadline = (Get-Date).AddSeconds(60)
    while (-not (Test-Path (Join-Path $cell 'ready.flag')) -and -not $p.HasExited -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 200 }
    if (Test-Path (Join-Path $cell 'ready.flag')) {
        Run-Admin @('workload', '--mode', 'run', '--volume', "$($v.letter):", '--cell', $cell, '--allowed', $v.allowed, '--denied', $v.denied, '--files', "$Files", '--label', $tag, '--out', $Out)
    }
    if (-not (Wait-Probe $p 200)) { Add-Row @{ kind = 'timeout'; cell = $id } }
}

$tokens = 'admin', 'restricted', 'standard'
foreach ($token in $tokens) {
    # Who is this token? One row per token, from inside the token.
    $p = Start-Probe $token @('info', '--label', $token, '--out', $Out)
    [void](Wait-Probe $p 60)

    foreach ($name in $volumes.Keys) {
        $v = $volumes[$name]
        $cell = Join-Path $Share "cells\start-$token-$name"
        New-Item -ItemType Directory -Force -Path $cell | Out-Null
        Run-Admin @('workload', '--mode', 'prepare', '--volume', "$($v.letter):", '--cell', $cell, '--label', "$token/$name", '--out', $Out)
        $attempt = @('attempt-all', '--volume', "$($v.letter):", '--start', (Join-Path $cell 'start.json'), '--label', "$token/$name", '--out', $Out)
        if ($v.allowed) { $attempt += @('--file', (Join-Path $v.allowed 'fileusn.txt')) }
        $p = Start-Probe $token $attempt
        if (-not (Wait-Probe $p 240)) { Add-Row @{ kind = 'timeout'; cell = "attempt-$token-$name" } }
    }

    foreach ($name in $volumes.Keys) {
        $v = $volumes[$name]
        if (-not $v.journal) { continue }
        $handles = if ($name -eq 'c') { @('guid-dir') } else { @('guid-dir', 'traverse', 'backup-root') }
        foreach ($h in $handles) { Invoke-Cell $token $name $h 'read-unpriv' 'watch' }
        if ($token -eq 'admin') { Invoke-Cell $token $name 'device-read' 'read' 'watch' }
        if ($name -ne 'c') { Invoke-Cell $token $name 'guid-dir' 'read-unpriv' 'overlapped' }
    }
}

# Minimum rights for the GUID-path handle (standard user, NTFS VHD).
if ($volumes.Contains('ntfs')) {
    foreach ($access in 'generic-read', 'list-directory', 'read-attributes', 'traverse', 'synchronize', 'none') {
        $cell = Join-Path $Share "cells\access-$access"
        New-Item -ItemType Directory -Force -Path $cell | Out-Null
        Run-Admin @('workload', '--mode', 'prepare', '--volume', 'P:', '--cell', $cell, '--label', "access/$access", '--out', $Out)
        $p = Start-Probe 'standard' @('attempt-all', '--volume', 'P:', '--access', $access, '--start', (Join-Path $cell 'start.json'), '--label', "standard/access-$access", '--out', $Out)
        [void](Wait-Probe $p 120)
    }
}

# Stretch (question 7): directory FRN set for a large tree.
if ($volumes.Contains('ntfs')) {
    $treeRoot = 'P:\tree'
    New-Item -ItemType Directory -Force -Path $treeRoot | Out-Null
    icacls $treeRoot /grant 'Everyone:(OI)(CI)M' /Q | Out-Null
    Run-Admin @('tree', '--dir', $treeRoot, '--dirs', "$TreeDirs", '--fanout', '100', '--label', 'tree', '--out', $Out)
    foreach ($token in 'admin', 'standard') {
        $cell = Join-Path $Share "cells\frnset-$token"
        New-Item -ItemType Directory -Force -Path $cell | Out-Null
        Run-Admin @('workload', '--mode', 'prepare', '--volume', 'P:', '--cell', $cell, '--label', "frnset/$token", '--out', $Out)
        $fsctl = if ($token -eq 'admin') { 'read' } else { 'read-unpriv' }
        $handle = if ($token -eq 'admin') { 'device-read' } else { 'guid-dir' }
        $p = Start-Probe $token @('frnset', '--dir', $treeRoot, '--volume', 'P:', '--handle', $handle, '--fsctl', $fsctl, '--start', (Join-Path $cell 'start.json'), '--label', "frnset/$token", '--out', $Out)
        if (-not (Wait-Probe $p 600)) { Add-Row @{ kind = 'timeout'; cell = "frnset-$token" } }
    }
}

# Ground truth from fsutil on the NTFS and ReFS volumes.
foreach ($name in 'ntfs', 'refs') {
    if ($volumes.Contains($name)) {
        $l = "$($volumes[$name].letter):"
        Capture "fsutil usn readjournal $name" { fsutil usn readjournal $l minver=2 maxver=3 }
    }
}

# ------------------------------------------------------------------ cleanup
foreach ($name in 'ntfs', 'refs', 'exfat', 'fat32', 'devdrive') {
    $vhd = Join-Path $Work "$name.vhdx"
    if (Test-Path $vhd) {
        Set-Content -Path (Join-Path $Work "dp-detach-$name.txt") -Value "select vdisk file=`"$vhd`"`ndetach vdisk" -Encoding ascii
        diskpart /s (Join-Path $Work "dp-detach-$name.txt") | Out-Null
    }
}
net user $UserName /delete | Out-Null
Copy-Item $Out (Join-Path $Work "usn-probe-$($Label).jsonl") -Force
Write-Host "wrote $Out"
