<#
.SYNOPSIS
    Regenerates .github/workflows/release.yml with dist and applies the one
    change that file carries (#73).
.DESCRIPTION
    dist writes release.yml, which starts on a pushed tag and on pull
    requests. A tag that release-plz creates with the built-in token starts no
    workflow, so release-plz starts the build with `gh workflow run`, and that
    needs the `workflow_dispatch` trigger. dist has no setting for it
    (`allow-dirty = ["ci"]` in dist-workspace.toml allows the difference), so
    this script adds it after every `dist generate`.
    -Mode Write regenerates and patches the file; -Mode Check does the same
    and fails when the committed file differs from the result.
#>
[CmdletBinding()]
param([ValidateSet('Write', 'Check')][string]$Mode = 'Check')

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$path = '.github/workflows/release.yml'

# dist refuses to generate a file its config marks as allowed to differ, so
# the setting is taken out for the run and put back afterwards.
$config = 'dist-workspace.toml'
$original = Get-Content -LiteralPath $config -Raw
try {
    $withoutAllow = ($original -split "`n" | Where-Object { $_ -notmatch '^allow-dirty\s*=' }) -join "`n"
    [IO.File]::WriteAllText((Join-Path (Get-Location) $config), $withoutAllow, (New-Object Text.UTF8Encoding($false)))
    dist generate --mode ci
    if ($LASTEXITCODE -ne 0) { throw 'dist generate failed' }
}
finally {
    [IO.File]::WriteAllText((Join-Path (Get-Location) $config), $original, (New-Object Text.UTF8Encoding($false)))
}
$text = Get-Content -LiteralPath $path -Raw
$anchor = "on:`n  pull_request:`n"
if (-not $text.Contains($anchor)) { throw "$path has no `on: pull_request:` block to extend; dist changed its layout" }
$patched = $text.Replace($anchor, "on:`n  workflow_dispatch:`n  pull_request:`n")
[IO.File]::WriteAllText((Join-Path (Get-Location) $path), $patched, (New-Object Text.UTF8Encoding($false)))

if ($Mode -eq 'Check') {
    git diff --exit-code -- $path
    if ($LASTEXITCODE -ne 0) { throw "$path is not what dist generate plus this script make of dist-workspace.toml; run scripts/ci/release-workflow.ps1 -Mode Write" }
    'release.yml is up to date'
}
