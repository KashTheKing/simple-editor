<#
.SYNOPSIS
    Build the release exe and log its size against size_log.csv.
.DESCRIPTION
    size-diet (issue #18): the binary-size gate the plan asks for. Builds release, reads
    target/release/simple-editor.exe's byte length, appends "<sha>,<bytes>,<note>" to size_log.csv
    (repo root; git attributes it merge=union so concurrent worktrees never conflict on it), and prints
    the delta against the previous row. Warns (does not fail) when that delta exceeds +65536 bytes and
    no -Note was given; enforcing the gate (failing the check) is /se-verify's job, not this script's.
.PARAMETER Note
    Free-text reason for the size change, written into size_log.csv's note column (e.g. "size-diet",
    "+240 KB - new codec support"). Empty by default.
#>
param([string]$Note = "")

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
Push-Location $repoRoot
try {
    cargo build --release
    if ($LASTEXITCODE -ne 0) {
        Write-Error "cargo build --release failed (exit $LASTEXITCODE)"
        exit $LASTEXITCODE
    }

    $exe = Join-Path $repoRoot "target\release\simple-editor.exe"
    if (-not (Test-Path $exe)) {
        Write-Error "release exe not found at $exe"
        exit 1
    }
    $bytes = (Get-Item $exe).Length
    $mb = [math]::Round($bytes / 1MB, 2)
    $sha = (git rev-parse --short HEAD).Trim()

    $csv = Join-Path $repoRoot "size_log.csv"
    $prevBytes = $null
    if (Test-Path $csv) {
        $rows = Import-Csv $csv -Encoding UTF8
        if ($rows.Count -gt 0) {
            $prevBytes = [int64]($rows[-1].bytes)
        }
    }

    Add-Content -Path $csv -Value "$sha,$bytes,$Note" -Encoding utf8

    Write-Host "simple-editor.exe: $bytes bytes ($mb MB) [$sha]"
    if ($null -ne $prevBytes) {
        $delta = $bytes - $prevBytes
        $sign = if ($delta -ge 0) { "+" } else { "" }
        Write-Host "delta vs previous row: $sign$delta bytes"
        if ($delta -gt 65536 -and [string]::IsNullOrWhiteSpace($Note)) {
            Write-Warning "size grew by more than 64 KB with no -Note explaining why - se-verify/the PR body should name a reason."
        }
    } else {
        Write-Host "(no previous row in size_log.csv to diff against)"
    }
}
finally {
    Pop-Location
}
