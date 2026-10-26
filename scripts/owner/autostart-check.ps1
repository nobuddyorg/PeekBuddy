<#
.SYNOPSIS
    Checks PeekBuddy autostart on the owner's PC: the Run value, Task Manager's
    switch, what the autostarted engine costs, and its "Startup impact".
.DESCRIPTION
    Owner-run script for #63. Needs no Rust toolchain and no elevation. Three
    phases, because the check needs a real sign-in:

      -Phase prepare   writes HKCU Run value PeekBuddy = "<Exe>" (the only
                       persistent change; removed by -Phase cleanup). Then sign
                       out and in again.
      -Phase report    after sign-in: reports the Run value and StartupApproved
                       flag, whether the engine is running with no viewer child,
                       and its CPU time and bytes read since start. Pass
                       -TaskManagerImpact with what Task Manager > Startup apps
                       shows in the "Startup impact" column (it needs a few
                       minutes after sign-in; Low is the goal).
      -Phase cleanup   stops the engine and removes the Run value.

    Writes autostart-check-report.json next to the script unless -OutFile is
    given (only in the report phase).
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateSet('prepare', 'report', 'cleanup')][string]$Phase,
    [string]$Exe,
    [ValidateSet('Not measured', 'Not measured yet', 'Low', 'Medium', 'High')][string]$TaskManagerImpact = 'Not measured',
    [string]$OutFile = (Join-Path $PSScriptRoot 'autostart-check-report.json')
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$approvedKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run'
$name = 'PeekBuddy'

function Get-RegistryValue([string]$Path, [string]$Name) {
    try { (Get-ItemProperty -Path $Path -Name $Name -ErrorAction Stop).$Name } catch { $null }
}

switch ($Phase) {
    'prepare' {
        if (-not $Exe -or -not (Test-Path -LiteralPath $Exe)) {
            throw 'Pass -Exe with the full path of peekbuddy.exe.'
        }
        $full = (Resolve-Path -LiteralPath $Exe).Path
        New-ItemProperty -Path $runKey -Name $name -Value ('"{0}"' -f $full) -PropertyType String -Force | Out-Null
        Write-Output "Run value written. Sign out and in again, wait about 5 minutes, then run:"
        Write-Output "  autostart-check.ps1 -Phase report [-TaskManagerImpact Low]"
        Write-Output "and finally -Phase cleanup."
    }
    'report' {
        $value = Get-RegistryValue $runKey $name
        $approved = Get-RegistryValue $approvedKey $name
        $flag = if ($null -ne $approved) { $approved[0] } else { $null }
        $engines = @(Get-CimInstance Win32_Process -Filter "Name = 'peekbuddy.exe'")
        $engine = $engines | Select-Object -First 1
        $children = 0
        $cpuMs = $null
        $readKiB = $null
        if ($engine) {
            $children = @(Get-CimInstance Win32_Process -Filter "ParentProcessId = $($engine.ProcessId)").Count
            $proc = Get-Process -Id $engine.ProcessId
            $cpuMs = [math]::Round($proc.TotalProcessorTime.TotalMilliseconds)
            $readKiB = [math]::Round($engine.ReadTransferCount / 1KB)
        }
        $report = [ordered]@{
            script            = 'autostart-check.ps1'
            schemaVersion     = 1
            generatedUtc      = (Get-Date).ToUniversalTime().ToString('o')
            runValue          = $value
            startupApprovedFlag = $flag
            startupApprovedHex  = if ($null -ne $approved) { ($approved | ForEach-Object { '{0:X2}' -f $_ }) -join ' ' } else { $null }
            engineProcesses   = $engines.Count
            engineChildProcesses = $children
            engineCpuMs       = $cpuMs
            engineReadKiB     = $readKiB
            taskManagerImpact = $TaskManagerImpact
            budget            = [ordered]@{ cpuMs = 300; readKiB = 1024; note = 'F7: startup CPU <= 300 ms, disk read <= 1 MB; Task Manager rates High above 1 s CPU or 3 MB' }
        }
        $report | ConvertTo-Json -Depth 4 | Set-Content -Path $OutFile -Encoding utf8
        Write-Output "Report written to $OutFile"
    }
    'cleanup' {
        Get-Process -Name peekbuddy -ErrorAction SilentlyContinue | Stop-Process -Force
        Remove-ItemProperty -Path $runKey -Name $name -ErrorAction SilentlyContinue
        Write-Output 'Engine stopped and Run value removed.'
    }
}
