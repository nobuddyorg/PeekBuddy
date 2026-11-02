# Turns the JSON lines of the nightly zero-loss tier (#277) into
# zero-loss-results.json and a table for the job summary. Fails when a run
# left an undetected loss or a difference between the replayed log and the
# disk, or when no run reported at all.
param(
    [string]$Runner = 'unknown',
    [string]$Lines = (Join-Path $env:RUNNER_TEMP 'zero-loss.jsonl'),
    [string]$Out = 'target/zero-loss/zero-loss-results.json'
)

if (-not (Test-Path $Lines)) { throw "the tier wrote no results to $Lines" }
$results = @(Get-Content $Lines | Where-Object { $_.Trim() } | ForEach-Object { $_ | ConvertFrom-Json })
if ($results.Count -eq 0) { throw 'the tier reported no run' }

$table = @('| run | ops | live | reconciled | gap rows | undetected | tree differences |', '| --- | --- | --- | --- | --- | --- | --- |')
foreach ($r in $results) {
    $table += ('| {0} | {1} | {2} | {3} | {4} | {5} | {6} |' -f $r.name, $r.ops, $r.live, $r.reconciled, $r.gapRows, $r.undetected, $r.treeDifferences)
}
$undetected = ($results | Measure-Object -Property undetected -Sum).Sum
$differences = ($results | Measure-Object -Property treeDifferences -Sum).Sum

New-Item -ItemType Directory -Force (Split-Path $Out) | Out-Null
@{ runner = $Runner; undetected = $undetected; treeDifferences = $differences; results = $results } |
    ConvertTo-Json -Depth 4 | Set-Content $Out
if ($env:GITHUB_STEP_SUMMARY) {
    "### Zero-loss tier ($Runner): undetected = $undetected" | Add-Content $env:GITHUB_STEP_SUMMARY
    $table | Add-Content $env:GITHUB_STEP_SUMMARY
}
$table
if ($undetected -ne 0 -or $differences -ne 0) {
    throw "undetected = $undetected, tree differences = $differences"
}
