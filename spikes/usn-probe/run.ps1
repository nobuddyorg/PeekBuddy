# Spike #31 orchestrator: builds scratch volumes, a standard user, and runs the
# probe in every token on every volume. Needs an elevated session (hosted
# runners are admin). Writes one JSONL file per cell under -OutDir.
param(
    [Parameter(Mandatory)] [string] $Exe,
    [Parameter(Mandatory)] [string] $Label,
    [string] $OutDir = 'C:\usn-probe\out',
    [string] $Work = 'D:\usn-vhd'
)
$ErrorActionPreference = 'Continue'
$password = 'Pr0be-Spike!2026'
$userName = 'probeuser'
if (-not (Test-Path 'D:\')) { $Work = Join-Path $env:RUNNER_TEMP 'usn-vhd' }
New-Item -ItemType Directory -Force -Path $OutDir, $Work, 'C:\usn-probe\bin' | Out-Null
Copy-Item $Exe 'C:\usn-probe\bin\usn-probe.exe' -Force
$probe = 'C:\usn-probe\bin\usn-probe.exe'
icacls 'C:\usn-probe' /grant 'Everyone:(OI)(CI)RX' | Out-Null
icacls $OutDir /grant 'Everyone:(OI)(CI)M' | Out-Null

net user $userName $password /add /y | Out-Null
net localgroup Users $userName /add 2>$null | Out-Null
"standard user: $((net user $userName | Select-String 'Local Group Memberships').ToString().Trim())"

function New-Volume([string] $name, [string] $letter, [string] $fs) {
    $vhd = Join-Path $Work "$name.vhdx"
    $script = @"
create vdisk file="$vhd" maximum=4096 type=expandable
select vdisk file="$vhd"
attach vdisk
create partition primary
format fs=$fs quick label=$name
assign letter=$letter
"@
    $tmp = Join-Path $Work "$name.txt"
    $script | Set-Content $tmp -Encoding ascii
    diskpart /s $tmp | Out-Host
    if (Test-Path "${letter}:\") { return "${letter}:" }
    return $null
}

$volumes = @()
$volumes += [pscustomobject]@{ Name = 'system-ntfs'; Volume = 'C:' }
if (Test-Path 'D:\') { $volumes += [pscustomobject]@{ Name = 'data-ntfs'; Volume = 'D:' } }
foreach ($v in @(@('vhd-ntfs', 'V', 'ntfs'), @('vhd-refs', 'W', 'refs'), @('vhd-exfat', 'X', 'exfat'), @('vhd-fat32', 'Y', 'fat32'))) {
    $mounted = New-Volume $v[0] $v[1] $v[2]
    if ($mounted) {
        $volumes += [pscustomobject]@{ Name = $v[0]; Volume = $mounted }
        if ($v[2] -in 'ntfs', 'refs') { fsutil usn createjournal m=33554432 a=8388608 $mounted | Out-Host }
    } else {
        "volume $($v[0]) could not be created" | Out-Host
        '{"spike":"usn","cell":"volume","name":"' + $v[0] + '","ok":false}' | Add-Content (Join-Path $OutDir "$Label-volumes.jsonl")
    }
}

function Get-Frn([string] $path) {
    $line = (fsutil file queryfileid $path 2>$null | Out-String).Trim()
    if ($line -match '0x([0-9a-fA-F]+)$') { return $Matches[1] }
    return ''
}

foreach ($vol in $volumes) {
    $drive = $vol.Volume
    $gt = Join-Path $OutDir "$Label-$($vol.Name)-fsutil-query.txt"
    fsutil usn queryjournal $drive 2>&1 | Out-File $gt
    foreach ($mode in 'admin', 'restricted', 'user') {
        $cell = "$($vol.Name)-$mode"
        $root = "$drive\probe-$mode"
        $ok = "$root\ok"
        $denied = "$root\denied"
        Remove-Item $root -Recurse -Force -ErrorAction SilentlyContinue
        New-Item -ItemType Directory -Force -Path $ok, $denied | Out-Null
        icacls $ok /grant "${userName}:(OI)(CI)M" /grant 'Users:(OI)(CI)M' 2>$null | Out-Null
        # Only Administrators and SYSTEM may use the denied directory: the standard
        # user and the restricted token (Administrators deny-only) are locked out,
        # the orchestrator (full admin) is not.
        icacls $denied /inheritance:r /grant 'Administrators:(OI)(CI)F' /grant 'SYSTEM:(OI)(CI)F' 2>$null | Out-Null
        (Get-Frn $denied) | Set-Content "$ok\denied-dir.frn" -Encoding ascii
        $out = Join-Path $OutDir "$Label-$cell.jsonl"
        Remove-Item $out -ErrorAction SilentlyContinue
        $common = @('--label', "$Label/$cell", '--volume', $drive, '--ok-dir', $ok, '--out', $out)
        $argsList = switch ($mode) {
            'admin' { @('battery') + $common }
            'restricted' { @('restricted') + $common }
            'user' { @('user', $userName, $password) + $common }
        }
        $proc = Start-Process -FilePath $probe -ArgumentList ($argsList | ForEach-Object { '"' + $_ + '"' }) -PassThru -NoNewWindow
        $deadline = (Get-Date).AddSeconds(90)
        while (-not (Test-Path "$ok\ready.flag") -and -not $proc.HasExited -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 200 }
        if (Test-Path "$ok\ready.flag") {
            # Changes the probe user cannot read: files in the denied directory and a deny-read file next to the probe's files.
            1..5 | ForEach-Object {
                $f = "$denied\denied-$_.txt"
                Set-Content $f 'secret'
                Add-Content $f 'more'
                Rename-Item $f "$denied\denied-$_-renamed.txt"
                Remove-Item "$denied\denied-$_-renamed.txt"
            }
            1..3 | ForEach-Object {
                $f = "$ok\denied-file-$_.txt"
                Set-Content $f 'secret'
                icacls $f /inheritance:r /grant 'Administrators:F' 2>$null | Out-Null
                Add-Content $f 'more'
            }
        }
        Set-Content "$ok\done.flag" '1'
        if (-not $proc.WaitForExit(150000)) { $proc.Kill(); "probe $cell timed out" | Out-Host }
        if ($mode -eq 'admin') {
            fsutil usn readjournal $drive minver=2 maxver=3 2>&1 | Select-Object -Last 40 | Out-File (Join-Path $OutDir "$Label-$($vol.Name)-fsutil-readjournal-tail.txt")
        }
        $childLog = [IO.Path]::ChangeExtension($out, 'child.log')
        if (Test-Path $childLog) { "--- $cell child log ---"; Get-Content $childLog | Select-Object -First 30 }
        if (-not (Test-Path $out)) {
            '{"spike":"usn","cell":"no-output","name":"' + $cell + '","exit":' + $proc.ExitCode + '}' | Add-Content $out
        }
    }
}
Get-ChildItem $OutDir -Filter "$Label-*.jsonl" | ForEach-Object { Get-Content $_.FullName } | Set-Content (Join-Path $OutDir "$Label-all.jsonl")
