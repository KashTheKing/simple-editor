# Build both release downloads into dist\:
#   simple-editor.exe        the portable app (cargo build --release, ~20 min clean: see agents.md)
#   simple-editor-setup.exe  the installer / updater / uninstaller (tools\se-setup, must stay under 1 MB)
# Then prints each file's size and SHA-256. Attach both to the GitHub Release; the updater looks for an
# asset named exactly simple-editor.exe and verifies it against the digest GitHub shows for it.
#
#   scripts\release.ps1             both
#   scripts\release.ps1 -SetupOnly  just the setup exe (seconds)
param([switch]$SetupOnly)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    if (-not $SetupOnly) {
        cargo build --release
        if ($LASTEXITCODE) { throw 'the app build failed' }
    }
    cargo build --release --manifest-path tools\se-setup\Cargo.toml
    if ($LASTEXITCODE) { throw 'the setup build failed' }

    $dist = Join-Path $root 'dist'
    New-Item -ItemType Directory -Force $dist | Out-Null
    if (-not $SetupOnly) {
        Copy-Item target\release\simple-editor.exe (Join-Path $dist 'simple-editor.exe') -Force
    }
    Copy-Item tools\se-setup\target\release\simple-editor-setup.exe (Join-Path $dist 'simple-editor-setup.exe') -Force

    foreach ($f in Get-ChildItem (Join-Path $dist '*.exe')) {
        $hash = (Get-FileHash $f.FullName -Algorithm SHA256).Hash.ToLower()
        '{0,-26} {1,12:N0} B  {2}' -f $f.Name, $f.Length, $hash
    }
    $setup = Get-Item (Join-Path $dist 'simple-editor-setup.exe')
    if ($setup.Length -ge 1MB) { throw "simple-editor-setup.exe is $($setup.Length) bytes: over the 1 MB budget" }
} finally {
    Pop-Location
}
