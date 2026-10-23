<#
.SYNOPSIS
    Writes an environment report for PeekBuddy owner runs.
.DESCRIPTION
    Example owner-run script (#86). Needs no Rust toolchain, no elevation and
    makes no persistent changes. Writes hello-report.json next to the script
    unless -OutFile is given.
#>
[CmdletBinding()]
param(
    [string]$OutFile = (Join-Path $PSScriptRoot 'hello-report.json')
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$started = Get-Date
$os = Get-CimInstance -ClassName Win32_OperatingSystem
$cpu = Get-CimInstance -ClassName Win32_Processor | Select-Object -First 1
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = [Security.Principal.WindowsPrincipal]::new($identity)

$sac = 'unknown'
try {
    $state = Get-ItemPropertyValue -Path 'HKLM:\SYSTEM\CurrentControlSet\Control\CI\Policy' -Name 'VerifiedAndReputablePolicyState'
    $sac = switch ($state) { 0 { 'off' } 1 { 'enforce' } 2 { 'evaluation' } default { "$state" } }
} catch {
    $sac = 'not-present'
}

$report = [ordered]@{
    script        = 'hello.ps1'
    schemaVersion = 1
    startedUtc    = $started.ToUniversalTime().ToString('o')
    os            = [ordered]@{
        caption = $os.Caption
        version = $os.Version
        build   = $os.BuildNumber
    }
    architecture  = $env:PROCESSOR_ARCHITECTURE
    cpu           = $cpu.Name
    logicalCpus   = [Environment]::ProcessorCount
    memoryGiB     = [math]::Round($os.TotalVisibleMemorySize / 1MB, 1)
    elevated      = $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    powershell    = $PSVersionTable.PSVersion.ToString()
    smartAppControl = $sac
    durationSec   = [math]::Round(((Get-Date) - $started).TotalSeconds, 2)
}

$report | ConvertTo-Json -Depth 4 | Set-Content -Path $OutFile -Encoding utf8
Write-Output "Report written to $OutFile"
