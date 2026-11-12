<#
.SYNOPSIS
    Rebuild library/index.json and library/thumbs/*.jpg from the files under library/.
.DESCRIPTION
    A thin wrapper (ws:online-library): the work is in Rust tests, because thumbnails need the app's
    own renderers (the GL driver for shaders, Edge for HTML / icons, the LUT sampler for .cube files).
      1. sets SE_LIBRARY_WRITE=1 (library_index_matches_files writes index.json instead of checking it)
         and SE_THUMBS / SE_SHOTS = a temp folder (the thumbnail tests write <kind dir>-<id>.png there),
      2. runs `cargo test library_ -- --include-ignored --test-threads=1 --nocapture`,
      3. converts every PNG into library/thumbs/<kind dir>-<id>.jpg, 240 px wide, transparent parts on
         dark grey, with ffmpeg (on PATH).
    Stops with a message on the first failure. Run it again after adding or changing a library file;
    `cargo test library_index_matches_files` fails until you do.
#>
$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$shots = Join-Path ([IO.Path]::GetTempPath()) "se-library-thumbs-$PID"
New-Item -ItemType Directory -Force $shots | Out-Null

Push-Location $repoRoot
try {
    $env:SE_LIBRARY_WRITE = "1"
    # luts-<id>.png (library_lut_thumbs); html-<id>.png, icons-<id>.png (library_html_thumbs, through Edge);
    # effects-<id>.png, transitions-<id>.png (library_shaders_compile_and_draw_on_the_real_driver)
    $env:SE_THUMBS = $shots

    cargo test library_ -- --include-ignored --test-threads=1 --nocapture
    if ($LASTEXITCODE -ne 0) {
        Write-Error "library tests failed (exit $LASTEXITCODE): fix the files they name, then run this again."
        exit 1
    }

    if (-not (Get-Command ffmpeg -ErrorAction SilentlyContinue)) {
        Write-Error "ffmpeg not found on PATH: index.json was written, thumbnails were not."
        exit 1
    }
    $out = Join-Path $repoRoot "library\thumbs"
    New-Item -ItemType Directory -Force $out | Out-Null
    $pngs = Get-ChildItem $shots -Filter *.png | Where-Object { $_.BaseName -match '^(effects|transitions|luts|html|icons)-[a-z0-9-]+$' }
    $bad = @()
    foreach ($png in $pngs) {
        $jpg = Join-Path $out ($png.BaseName + ".jpg")
        # flatten any transparency onto dark grey, 240 px wide, even height
        $filter = "[0]scale=240:-2,split[a][b];[a]drawbox=c=0x303030:t=fill[bg];[bg][b]overlay=format=auto"
        & ffmpeg -v error -y -i $png.FullName -filter_complex $filter -frames:v 1 -q:v 4 $jpg
        if ($LASTEXITCODE -ne 0) { $bad += $png.Name }
    }
    if ($bad.Count -gt 0) {
        Write-Error "ffmpeg could not convert: $($bad -join ', ')"
        exit 1
    }
    Write-Host "index.json written; $($pngs.Count) thumbnails in library\thumbs"
}
finally {
    Remove-Item Env:SE_LIBRARY_WRITE, Env:SE_THUMBS -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force $shots -ErrorAction SilentlyContinue
    Pop-Location
}
