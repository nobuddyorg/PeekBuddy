# Per-crate size deltas from two `cargo bloat --crates --message-format json`
# outputs (#28): prints a Markdown table for the job summary.
param(
    [Parameter(Mandatory)][string]$Base,
    [Parameter(Mandatory)][string]$Head
)
$ErrorActionPreference = 'Stop'

function Read-Crates($path) {
    $json = Get-Content $path -Raw | ConvertFrom-Json
    $map = @{}
    foreach ($c in $json.crates) { $map[$c.name] = [int64]$c.size }
    return @{ map = $map; text = [int64]$json.'text-section-size'; file = [int64]$json.'file-size' }
}

$b = Read-Crates $Base
$h = Read-Crates $Head
$names = @($b.map.Keys) + @($h.map.Keys) | Sort-Object -Unique
$rows = foreach ($n in $names) {
    $bs = if ($b.map.ContainsKey($n)) { $b.map[$n] } else { 0 }
    $hs = if ($h.map.ContainsKey($n)) { $h.map[$n] } else { 0 }
    [pscustomobject]@{ crate = $n; base = $bs; head = $hs; delta = $hs - $bs }
}
$lines = @('### Size per crate (cargo bloat --crates)', '',
    "File size: base $($b.file) bytes, head $($h.file) bytes ($($h.file - $b.file) bytes).", '',
    '| crate | base | head | delta |', '| --- | --- | --- | --- |')
foreach ($r in ($rows | Sort-Object { [math]::Abs($_.delta) } -Descending | Select-Object -First 30)) {
    $lines += ('| {0} | {1} | {2} | {3:+#;-#;0} |' -f $r.crate, $r.base, $r.head, $r.delta)
}
$lines -join "`n"
