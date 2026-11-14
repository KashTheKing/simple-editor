<#
.SYNOPSIS
    Build the fast HTML renderer helper (tools/se-html), write its library manifest and log its size.
.DESCRIPTION
    The helper is NOT part of simple-editor.exe (it is ~11 MB: Blitz = Stylo + Taffy + Parley + vello_cpu).
    It is built on its own, with its own Cargo.lock and toolchain pin (tools/se-html/rust-toolchain.toml:
    Rust 1.90; beta.4 declares 1.91 but builds and renders the same, so `--ignore-rust-version`), and is
    published next to the library:  <library>/bin/se-html.exe  +  <library>/bin/se-html.json
    (version, protocol, url, size, sha256 - what `engine::blitz::install` checks before it runs the file).
    Output goes to dist/se-html/ (git-ignored). Appends "<sha>,<bytes>,<note>" to size_log.csv as the HELPER's
    own row (the note starts with "se-html:"), so the main exe's rows stay comparable.
.PARAMETER Url
    Where the exe will be served from: a full address (a GitHub release asset) or a path relative to the
    library (default "bin/se-html.exe"). A folder library (Settings > library folder) reads that path.
.PARAMETER Note
    Free text for the size_log.csv row.
#>
param([string]$Url = "bin/se-html.exe", [string]$Note = "")

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$crate = Join-Path $repoRoot "tools\se-html"
Push-Location $crate
try {
    cargo build --release --ignore-rust-version
    if ($LASTEXITCODE -ne 0) { Write-Error "cargo build failed ($LASTEXITCODE)"; exit $LASTEXITCODE }
    $targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { (cargo metadata --format-version 1 --no-deps | ConvertFrom-Json).target_directory }
    $exe = Join-Path $targetDir "release\se-html.exe"
    $dist = Join-Path $repoRoot "dist\se-html\bin"
    New-Item -ItemType Directory -Force $dist | Out-Null
    Copy-Item $exe (Join-Path $dist "se-html.exe") -Force
    $bytes = (Get-Item (Join-Path $dist "se-html.exe")).Length
    $sha256 = (Get-FileHash (Join-Path $dist "se-html.exe") -Algorithm SHA256).Hash.ToLower()
    $version = ((& $exe --version) -replace '^se-html (\S+).*$', '$1')
    $protocol = [int](((& $exe --version) -replace '^.*protocol (\d+).*$', '$1'))
    $manifest = [ordered]@{ version = $version; protocol = $protocol; url = $Url; size = $bytes; sha256 = $sha256 }
    # no BOM: the app's JSON reader (and any other) should not have to skip one
    [System.IO.File]::WriteAllText((Join-Path $dist "se-html.json"), ($manifest | ConvertTo-Json), (New-Object System.Text.UTF8Encoding($false)))
    Write-Host "se-html $version : $bytes bytes ($([math]::Round($bytes / 1MB, 2)) MB), sha256 $sha256"
    Write-Host "publish dist\se-html\bin\* next to the library (or upload se-html.exe as a release asset and pass -Url)"

    $csv = Join-Path $repoRoot "size_log.csv"
    $sha = (git rev-parse --short HEAD).Trim()
    Add-Content -Path $csv -Value "$sha,$bytes,`"se-html: the fast HTML renderer helper $version (separate exe, not linked into simple-editor.exe). $Note`"" -Encoding utf8
}
finally { Pop-Location }
