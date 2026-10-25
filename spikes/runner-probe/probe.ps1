<#
.SYNOPSIS
    Spike #34: PowerShell half of the runner probe.
.DESCRIPTION
    Emits one JSON line per check ({check, label, ok, detail}) to stdout.
    Every check is wrapped so one failure never stops the others. Cleanup of
    VHDs, the SMB share, the test user and the test service happens in
    -Cleanup mode, which the workflow runs with if: always().
#>
[CmdletBinding()]
param(
    [string]$Label = 'local',
    [string]$WorkDir = (Join-Path $env:RUNNER_TEMP 'probe'),
    [switch]$Cleanup
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path $WorkDir | Out-Null

function Emit([string]$Check, [bool]$Ok, [string]$Detail) {
    [ordered]@{ check = $Check; label = $Label; ok = $Ok; detail = $Detail } |
        ConvertTo-Json -Compress
}

function Probe([string]$Check, [scriptblock]$Body) {
    try {
        # Only the last object is the result; cmdlets may emit other output.
        $result = @(& $Body)[-1]
        Emit $Check $result.ok "$($result.detail)"
    } catch {
        Emit $Check $false "exception: $($_.Exception.Message)"
    }
}

function Invoke-DiskPart([string[]]$Lines) {
    $script = Join-Path $WorkDir 'diskpart.txt'
    Set-Content -Path $script -Value $Lines -Encoding ascii
    $out = & diskpart.exe /s $script 2>&1 | Out-String
    [pscustomobject]@{ code = $LASTEXITCODE; out = $out }
}

$share = 'PeekBuddyProbe'
$user = 'pbprobe'
$service = 'PeekBuddyProbeSvc'

if ($Cleanup) {
    foreach ($fs in 'ntfs', 'refs', 'fat32', 'exfat') {
        $vhd = Join-Path $WorkDir "$fs.vhdx"
        if (Test-Path $vhd) { Invoke-DiskPart @("select vdisk file=`"$vhd`"", 'detach vdisk') | Out-Null }
    }
    Remove-SmbShare -Name $share -Force -ErrorAction SilentlyContinue
    & net.exe user $user /delete 2>&1 | Out-Null
    & sc.exe stop $service 2>&1 | Out-Null
    & sc.exe delete $service 2>&1 | Out-Null
    Get-Process -Name wpr -ErrorAction SilentlyContinue | Stop-Process -Force
    exit 0
}

# 1. Session
Probe 'session.qwinsta' {
    $q = & qwinsta.exe 2>&1 | Out-String
    @{ ok = ($q -match 'Active'); detail = ($q -replace '\s+', ' ').Trim() }
}
Probe 'session.explorer' {
    $e = @(Get-Process -Name explorer -ErrorAction SilentlyContinue)
    @{ ok = ($e.Count -gt 0); detail = "count=$($e.Count) session=$(($e | Select-Object -First 1).SessionId)" }
}

# 4. Display
Probe 'display.default_resolution' {
    Add-Type -AssemblyName System.Windows.Forms
    $b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    @{ ok = $true; detail = "$($b.Width)x$($b.Height)" }
}
Probe 'display.set_resolution_cmdlet' {
    $cmd = Get-Command Set-DisplayResolution -ErrorAction SilentlyContinue
    if (-not $cmd) { return @{ ok = $false; detail = 'Set-DisplayResolution not present' } }
    Set-DisplayResolution -Width 1920 -Height 1080 -Force
    Add-Type -AssemblyName System.Windows.Forms
    $b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    @{ ok = ($b.Width -eq 1920); detail = "after=$($b.Width)x$($b.Height)" }
}

# 5. Service control (create/query/delete; start needs a real service binary)
Probe 'service.sc_create_query_delete' {
    $create = & sc.exe create $service binPath= "$env:SystemRoot\System32\cmd.exe /c exit 0" start= demand 2>&1 | Out-String
    $query = & sc.exe query $service 2>&1 | Out-String
    $start = & sc.exe start $service 2>&1 | Out-String
    $delete = & sc.exe delete $service 2>&1 | Out-String
    @{ ok = ($create -match 'SUCCESS' -and $query -match 'STATE' -and $delete -match 'SUCCESS'); detail = "start: $(($start -replace '\s+', ' ').Trim())" }
}

# 6. Disks
foreach ($fs in 'ntfs', 'refs', 'fat32', 'exfat') {
    Probe "disk.vhd_$fs" {
        $vhd = Join-Path $WorkDir "$fs.vhdx"
        $r = Invoke-DiskPart @(
            "create vdisk file=`"$vhd`" maximum=4096 type=expandable",
            "select vdisk file=`"$vhd`"",
            'attach vdisk',
            'create partition primary',
            "format fs=$fs quick label=PB$($fs.ToUpper().Substring(0, 3))",
            'assign'
        )
        $vol = Get-Volume -FileSystemLabel "PB$($fs.ToUpper().Substring(0, 3))" -ErrorAction SilentlyContinue
        $letter = if ($vol) { $vol.DriveLetter } else { '' }
        Invoke-DiskPart @("select vdisk file=`"$vhd`"", 'detach vdisk') | Out-Null
        @{ ok = ($r.code -eq 0 -and $letter); detail = "exit=$($r.code) letter=$letter" }
    }
}
foreach ($drive in 'C:', 'D:') {
    Probe "disk.usn_$($drive.TrimEnd(':').ToLower())" {
        if (-not (Test-Path "$drive\")) { return @{ ok = $false; detail = 'drive not present' } }
        $out = & fsutil.exe usn queryjournal $drive 2>&1 | Out-String
        @{ ok = ($LASTEXITCODE -eq 0); detail = (($out -split "`n" | Select-Object -First 3) -join ' ').Trim() }
    }
}
Probe 'disk.smb_loopback' {
    $dir = Join-Path $WorkDir 'share'
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    New-SmbShare -Name $share -Path $dir -FullAccess 'Everyone' | Out-Null
    Set-Content -Path "\\localhost\$share\probe.txt" -Value 'probe'
    $ok = Test-Path (Join-Path $dir 'probe.txt')
    Remove-SmbShare -Name $share -Force
    @{ ok = $ok; detail = "\\localhost\$share" }
}

# 7. Privilege: standard user via CreateProcessWithLogonW (Start-Process -Credential)
Probe 'privilege.standard_user_logon' {
    $plain = "Pb!" + [guid]::NewGuid().ToString("N").Substring(0, 10)
    $add = & net.exe user $user $plain /add 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { return @{ ok = $false; detail = "net user /add: $(($add -replace '\s+', ' ').Trim())" } }
    $secure = ConvertTo-SecureString $plain -AsPlainText -Force
    $cred = [pscredential]::new("$env:COMPUTERNAME\$user", $secure)
    $out = Join-Path $WorkDir 'whoami.txt'
    $p = Start-Process -FilePath "$env:SystemRoot\System32\cmd.exe" -ArgumentList "/c whoami /groups > `"$out`"" -Credential $cred -WorkingDirectory $env:SystemRoot -Wait -PassThru -WindowStyle Hidden
    & net.exe user $user /delete 2>&1 | Out-Null
    $groups = if (Test-Path $out) { Get-Content $out -Raw } else { '' }
    @{ ok = ($p.ExitCode -eq 0); detail = "exit=$($p.ExitCode) medium_il=$($groups -match 'Medium Mandatory Level')" }
}

# 8. Defender
Probe 'defender.status' {
    $mp = Get-MpComputerStatus
    $pref = Get-MpPreference
    @{ ok = $true; detail = "realtime=$($mp.RealTimeProtectionEnabled) tamper=$($mp.IsTamperProtected) exclusions=$(@($pref.ExclusionPath).Count)" }
}

# 10. ETW
Probe 'etw.wpr_cpu_cswitch' {
    $etl = Join-Path $WorkDir 'probe.etl'
    & wpr.exe -start CPU -filemode 2>&1 | Out-Null
    $startCode = $LASTEXITCODE
    Start-Sleep -Seconds 10
    & wpr.exe -stop $etl 2>&1 | Out-Null
    $size = if (Test-Path $etl) { (Get-Item $etl).Length } else { 0 }
    @{ ok = ($startCode -eq 0 -and $size -gt 0); detail = "start=$startCode etl_bytes=$size" }
}

# 11. Environment
Probe 'env.summary' {
    $os = Get-CimInstance -ClassName Win32_OperatingSystem
    $cpu = Get-CimInstance -ClassName Win32_Processor | Select-Object -First 1
    $free = [math]::Round((Get-PSDrive C).Free / 1GB, 1)
    @{ ok = $true; detail = "image=$env:ImageOS/$env:ImageVersion build=$($os.BuildNumber) cpu=$($cpu.Name.Trim()) logical=$([Environment]::ProcessorCount) ram_gib=$([math]::Round($os.TotalVisibleMemorySize / 1MB, 1)) free_c_gib=$free tz=$((Get-TimeZone).Id) culture=$((Get-Culture).Name)" }
}

# 13. hyperfine (installed by the workflow when available)
Probe 'noise.hyperfine_trivial_exe' {
    $hf = Get-Command hyperfine -ErrorAction SilentlyContinue
    if (-not $hf) { return @{ ok = $false; detail = 'hyperfine not installed' } }
    $json = Join-Path $WorkDir 'hyperfine.json'
    # -N splits the command with shell-word rules (backslashes are escapes),
    # so use the bare name; hyperfine 2 writes schema_version 2: the
    # statistics are under results[0].summary.<metric>.
    $out = & hyperfine -N --warmup 5 --runs 50 --export-json $json 'whoami.exe' 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path $json)) { return @{ ok = $false; detail = "hyperfine exit=$LASTEXITCODE $($out.Trim())" } }
    $r = (Get-Content $json -Raw | ConvertFrom-Json).results[0].summary.time_wall_clock
    $cv = if ($r.mean -gt 0) { [math]::Round($r.stddev / $r.mean * 100, 2) } else { 0 }
    @{ ok = $true; detail = "mean_ms=$([math]::Round($r.mean * 1000, 2)) cv_pct=$cv" }
}

# A probe reports; failed checks are data, not a failed step.
exit 0
