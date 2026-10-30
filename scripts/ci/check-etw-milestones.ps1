<#
.SYNOPSIS
    Checks the PeekBuddy milestones in a tracerpt XML dump (#162).
.DESCRIPTION
    Reads the PeekBuddy provider's `Milestone` events from the XML that
    `tracerpt -of XML` wrote, in the order they were logged, and fails unless
    every name in -Expect appears in that order (other milestones may sit in
    between), the `SinceProcessCreateUs` values never go backwards, and each
    name in -Once appears exactly once.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Xml,
    [Parameter(Mandatory)][string[]]$Expect,
    [string[]]$Once = @()
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$provider = '69eb5350-8420-5c03-5c05-1a6dafc0df73'
$text = Get-Content -LiteralPath $Xml -Raw
$rows = New-Object System.Collections.Generic.List[object]
foreach ($event in [regex]::Matches($text, '<Event[ >].*?</Event>', 'Singleline')) {
    if ($event.Value -notmatch $provider) { continue }
    $name = [regex]::Match($event.Value, "Name=[""']Name[""']\s*>([^<]*)<").Groups[1].Value
    $us = [regex]::Match($event.Value, "Name=[""']SinceProcessCreateUs[""']\s*>([^<]*)<").Groups[1].Value
    if ($name) {
        $rows.Add([pscustomobject]@{ Name = $name; Us = [uint64]$us })
    }
}
"PeekBuddy milestones in the dump: " + (($rows | ForEach-Object { "$($_.Name)@$($_.Us)" }) -join ' ')
if ($rows.Count -eq 0) { throw 'no PeekBuddy milestone events found in the dump' }

$at = 0
foreach ($want in $Expect) {
    $found = $false
    while ($at -lt $rows.Count) {
        if ($rows[$at].Name -eq $want) { $found = $true; $at++; break }
        $at++
    }
    if (-not $found) { throw "milestone $want is missing or out of order" }
}
for ($i = 1; $i -lt $rows.Count; $i++) {
    if ($rows[$i].Us -lt $rows[$i - 1].Us) {
        throw "SinceProcessCreateUs went backwards: $($rows[$i - 1].Name)@$($rows[$i - 1].Us) then $($rows[$i].Name)@$($rows[$i].Us)"
    }
}
foreach ($name in $Once) {
    $count = @($rows | Where-Object { $_.Name -eq $name }).Count
    if ($count -ne 1) { throw "milestone $name was logged $count times, expected once" }
}
'milestones ok'
