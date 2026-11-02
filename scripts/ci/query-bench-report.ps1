# Turns the criterion medians of the query benches (#267) into the series the
# benchmark action publishes, and warns when a median is above the limit of its
# row in perf/budgets.toml (wall-clock limits are reported, never a gate: ADR
# 0007; the trend alert is the gate).
param(
    [string]$Criterion = 'target/criterion',
    [string]$Budgets = 'perf/budgets.toml',
    [string]$Out = 'target/query-bench/output.json'
)

$limits = @{}
$id = $null
foreach ($line in Get-Content $Budgets) {
    if ($line -match '^\[\[budget\]\]') { $id = $null }
    elseif ($line -match '^id = "(bench\.query\.[a-z0-9_]+)"') { $id = $Matches[1] }
    elseif ($id -and $line -match '^limit = ([0-9.eE+]+)') { $limits[$id] = [double]$Matches[1] }
}

$rows = @()
$summary = @('| bench | median | limit |', '| --- | --- | --- |')
foreach ($file in Get-ChildItem "$Criterion/query/*/new/estimates.json") {
    $name = $file.Directory.Parent.Name
    $median = [double]((Get-Content $file -Raw | ConvertFrom-Json).median.point_estimate)
    $rows += @{ name = "query.$name"; unit = 'ns'; value = $median }
    $limit = $limits["bench.query.$name"]
    $shown = if ($limit) { '{0:N1} ms' -f ($limit / 1e6) } else { '-' }
    $summary += ('| `{0}` | {1:N2} ms | {2} |' -f $name, ($median / 1e6), $shown)
    if ($limit -and $median -gt $limit) {
        Write-Output ("::warning title=Query budget::query/{0} median {1:N2} ms is above {2:N1} ms" -f $name, ($median / 1e6), ($limit / 1e6))
    }
}
if ($rows.Count -eq 0) { throw 'no query bench estimates found' }

New-Item -ItemType Directory -Force (Split-Path $Out) | Out-Null
ConvertTo-Json -InputObject $rows | Set-Content $Out
if ($env:GITHUB_STEP_SUMMARY) { $summary | Add-Content $env:GITHUB_STEP_SUMMARY }
$summary
