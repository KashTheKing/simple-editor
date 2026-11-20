<#
.SYNOPSIS
    Build the object-mask helper (tools/sam2-helper, se-sam2.exe) and point library/bin/sam2.json at it.
.DESCRIPTION
    The helper is NOT part of simple-editor.exe. It is a small Rust program (~1.2 MB) that drives SAM 2.1
    tiny through ONNX Runtime; the runtime DLLs and the models are separate parts of the same manifest,
    fetched from NuGet and Hugging Face by `engine::sam2::install`. Its own Cargo.lock; its dev profile is
    optimised (opt-level 3), so `cargo build` is what sessions and the release both use.
    Writes dist/sam2/se-sam2.exe (git-ignored) and rewrites the se-sam2.exe entry of library/bin/sam2.json
    (url, size, sha256). Upload dist/sam2/se-sam2.exe as the release asset named in -Url.
.PARAMETER Url
    Where the exe will be served from: a full address (a GitHub release asset, the default) or a path
    relative to the library ("bin/se-sam2.exe") for a folder library.
#>
param([string]$Url = "https://github.com/KashTheKing/simple-editor/releases/download/se-sam2-v0.1.0/se-sam2.exe")

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$crate = Join-Path $repoRoot "tools\sam2-helper"
Push-Location $crate
try {
    cargo build
    if ($LASTEXITCODE -ne 0) { Write-Error "cargo build failed ($LASTEXITCODE)"; exit $LASTEXITCODE }
    $targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { (cargo metadata --format-version 1 --no-deps | ConvertFrom-Json).target_directory }
    $exe = Join-Path $targetDir "debug\se-sam2.exe"
    $dist = Join-Path $repoRoot "dist\sam2"
    New-Item -ItemType Directory -Force $dist | Out-Null
    Copy-Item $exe (Join-Path $dist "se-sam2.exe") -Force
    $bytes = (Get-Item (Join-Path $dist "se-sam2.exe")).Length
    $sha256 = (Get-FileHash (Join-Path $dist "se-sam2.exe") -Algorithm SHA256).Hash.ToLower()

    $manifest = Join-Path $repoRoot "library\bin\sam2.json"
    $m = Get-Content $manifest -Raw | ConvertFrom-Json
    $entry = $m.files | Where-Object { $_.path -eq "se-sam2.exe" }
    $entry.url = $Url
    $entry.size = $bytes
    $entry.sha256 = $sha256
    # no BOM: the app's JSON reader (and any other) should not have to skip one
    [System.IO.File]::WriteAllText($manifest, ($m | ConvertTo-Json -Depth 6), (New-Object System.Text.UTF8Encoding($false)))
    Write-Host "se-sam2: $bytes bytes ($([math]::Round($bytes / 1MB, 2)) MB), sha256 $sha256"
    Write-Host "upload dist\sam2\se-sam2.exe to $Url (or copy it next to a folder library and pass -Url bin/se-sam2.exe)"
}
finally { Pop-Location }
