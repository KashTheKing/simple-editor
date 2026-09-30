<#
.SYNOPSIS
    Regenerate the docs site's screenshots (website/static/img/tutorial/<name>.png) from the real app.
.DESCRIPTION
    Builds the app (dev profile), copies the exe into a work folder, makes demo media (the --selftest
    clips plus a few ffmpeg test patterns and tones) and a demo project, then takes every shot in $Shots
    below. Everything runs in a throw-away profile (APPDATA / LOCALAPPDATA point into the work folder), so
    your own settings, recent files and autosaves are never touched.

    Each shot is its own launch: the demo project is opened with `--screenshot <ppm>`, the UI is driven over
    the app's MCP server (JSON-RPC POSTs to http://127.0.0.1:<Port>/mcp), and then the file named by
    SE_SCREENSHOT_WHEN is created, which makes the app capture that frame. The live `ui.screenshot` tool
    would be quicker, but it reads back black frames while the Windows session is locked, and this path
    doesn't. Every window opens at -WindowRect without taking focus (SE_BACKGROUND=1), so the 50-odd
    launches don't keep jumping in front of whatever you're doing.

    Re-run it after any UI change, then look at the images. To redo some of them:
        .\scripts\docs-shots.ps1 -Only timeline*,library
    Needs cargo and ffmpeg on PATH (Windows PowerShell 5.1 or later).
.PARAMETER Only
    Names (wildcards allowed) of the shots to take; every shot by default.
.PARAMETER FromExe
    Use this simple-editor.exe (built from this checkout) instead of running cargo build. Handy because the
    cargo target dir is shared by every worktree, so another checkout's build can replace the exe in it.
.PARAMETER Port
    The MCP port the app is started on; it must be free.
.PARAMETER WindowRect
    Where the app's window opens: x, y, width, height in screen pixels. The default is the maintainer's
    second monitor (left of the main one); put it anywhere you won't mind windows appearing. Keep the size
    1600x900, the size every shot is laid out for.
.PARAMETER Work
    Work folder (exe copy, profile, media, demo project). Wiped at the start of every run.
#>
param(
    [string[]]$Only = @('*'),
    [string]$FromExe = '',
    [int]$Port = 7463,
    [int[]]$WindowRect = @(-2520, 112, 1600, 900),
    [string]$Work = (Join-Path $env:TEMP 'se-docs-shots')
)

# ---------------------------------------------------------------------------------------------------
# THE SHOTS: one row per image, in website/static/img/tutorial/<name>.png.
#   crop   '' = the whole window; 'Timeline' = that panel with its tab bar; 'Library+Source' = both;
#          '0,0,400,300' = a rect in window points; parts are joined, so 'Timeline+0,300,700,548' works.
#   empty  $true = start without the demo project (the home screen).
#   steps  run in order once the demo project is open (Edit page, nothing selected, playhead at 0,
#          timeline zoomed to fit):
#     page Color                  switch page             seek 10.5              move the playhead
#     select pattern,title        select demo clips (names in New-DemoProject; 'xfade' is the transition)
#     open fractal [3]            open a Library item in the Source monitor (paused, at 3 s)
#     act zoom_fit                run an Action by id (see src/hotkeys.rs, or the MCP tool ui.actions)
#     show Mixer                  bring a panel to the front (adds it if it is hidden)
#     click|rclick|dclick|move <Panel> <x> <y> [ctrl+shift+alt]
#                                 points from the panel's top-left corner; a negative x / y counts from
#                                 its right / bottom edge; '@' instead of a panel = the whole window
#     key F1 [ctrl]   type hello  tool <mcp tool> <json args>   wait 0.5
#   Timeline points: with the demo zoomed to fit, x = 152 + 56.4 * seconds; rows: ruler 57, subtitles 83,
#   V2 124, V1 188, A1 248, A2 304.
# ---------------------------------------------------------------------------------------------------
$Shots = @(
    # getting started
    @{ name = 'home'; empty = $true; crop = '560,275,1040,545'; steps = @() }
    @{ name = 'welcome'; empty = $true; crop = '540,330,1060,510'; steps = @('tool onboarding.reset {}') }
    @{ name = 'edit-page'; crop = ''; steps = @('select pattern', 'seek 10.5') }
    # the interface
    @{ name = 'page-media'; crop = ''; steps = @('page Media', 'open fractal 3') }
    @{ name = 'page-cut'; crop = ''; steps = @('page Cut', 'select pattern', 'seek 10.5') }
    @{ name = 'page-color'; crop = ''; steps = @('page Color', 'select pattern', 'seek 10.5') }
    @{ name = 'page-audio'; crop = ''; steps = @('page Audio', 'select music', 'seek 10.5') }
    @{ name = 'page-export'; crop = ''; steps = @('page Export', 'seek 10.5') }
    @{ name = 'page-menu'; crop = '600,0,960,70'; steps = @('rclick @ 767 12') }
    @{ name = 'tab-menu'; crop = '8,30,360,200'; steps = @('rclick Library 39 13') }
    @{ name = 'add-panel-menu'; crop = '8,30,470,360'; steps = @('click Library -12 13') }
    @{ name = 'window-menu'; crop = '215,0,760,640'; steps = @('click @ 243 12', 'move @ 267 431') }
    @{ name = 'palette'; crop = '520,70,1080,265'; steps = @('key K ctrl', 'type split') }
    @{ name = 'cheatsheet'; crop = '0,30,560,700'; steps = @('key F1') }
    @{ name = 'scripts-menu'; crop = '215,0,760,560'; steps = @('click @ 243 12', 'move @ 267 475') }
    @{ name = 'settings-hotkeys'; crop = '550,150,1600,900'; steps = @('act settings', 'click @ 856 204') }
    # library and source
    @{ name = 'library'; crop = 'Library'; steps = @() }
    @{ name = 'library-item-menu'; crop = 'Library'; steps = @('rclick Library 90 166') }
    @{ name = 'library-empty-menu'; crop = '8,30,480,705'; steps = @('rclick Library 200 380') }
    @{ name = 'source'; crop = 'Source'; steps = @('open fractal 3', 'tool source.mark {"in": 2, "out": 6}') }
    @{ name = 'source-menu'; crop = 'Source'; steps = @('open fractal', 'rclick Source 395 200') }
    # viewer
    @{ name = 'viewer'; crop = 'Preview'; steps = @('select pattern', 'seek 10.5') }
    @{ name = 'viewer-menu'; crop = '357,30,1148,630'; steps = @('select pattern', 'seek 10.5', 'rclick Preview 395 150') }
    @{ name = 'viewer-shapes'; crop = '357,30,760,320'; steps = @('seek 10.5', 'click Preview 22 123') }
    # timeline
    @{ name = 'timeline'; crop = 'Timeline'; steps = @('select pattern', 'seek 10.5') }
    @{ name = 'timeline-clip-menu'; crop = '560,190,1200,892'; steps = @('seek 10.5', 'rclick Timeline 772 188') }
    @{ name = 'timeline-track-menu'; crop = '8,400,400,892'; steps = @('rclick Timeline 32 188') }
    @{ name = 'timeline-ruler-menu'; crop = '560,548,1100,800'; steps = @('seek 10.5', 'rclick Timeline 700 57') }
    @{ name = 'timeline-transition-menu'; crop = '450,640,900,860'; steps = @('rclick Timeline 603 188') }
    @{ name = 'timeline-seam-menu'; crop = '1100,640,1592,820'; steps = @('rclick Timeline 1280 200') }
    @{ name = 'timeline-gap-menu'; crop = '600,600,1000,820'; steps = @('rclick Timeline 716 124') }
    @{ name = 'timeline-cue-menu'; crop = '400,590,800,780'; steps = @('rclick Timeline 462 83') }
    @{ name = 'timeline-marker-menu'; crop = '250,560,650,720'; steps = @('rclick Timeline 321 50') }
    @{ name = 'timeline-nested'; crop = 'Timeline'; steps = @('dclick Timeline 1421 188') }
    @{ name = 'timeline-blade'; crop = 'Timeline'; steps = @('seek 10.5', 'key C', 'move Timeline 772 188') }
    # inspector, effects, titles
    @{ name = 'inspector-clip'; crop = 'Inspector'; steps = @('select pattern', 'seek 10.5') }
    @{ name = 'inspector-text'; crop = '357,30,1592,547'; steps = @('select title', 'seek 2.5') }
    @{ name = 'keyframes'; crop = '8,548,700,720'; steps = @('select title', 'seek 1.5') }
    @{ name = 'keyframe-menu'; crop = '1149,30,1592,330'; steps = @('select gradient', 'seek 17', 'rclick Inspector 135 165') }
    @{ name = 'inspector-effects'; crop = '1149,30,1592,560'; steps = @('tool inspector.folds {"section": "effects", "open": true}', 'select gradient', 'seek 17', 'rclick Inspector 101 303') }
    @{ name = 'effects'; crop = '0,30,1000,640'; steps = @('show Effects', 'select gradient', 'seek 17', 'tool layout.maximize {"pane": "Effects"}') }
    @{ name = 'transitions'; crop = 'Transitions'; steps = @('show Transitions') }
    @{ name = 'gallery'; crop = 'Gallery'; steps = @('show Gallery', 'select pattern') }
    # color, audio, captions, export
    @{ name = 'color-inspector'; crop = '8,30,700,892'; steps = @('page Color', 'select pattern', 'seek 10.5', 'tool layout.maximize {"pane": "Inspector"}') }
    @{ name = 'scopes'; crop = 'Scopes'; steps = @('page Color', 'select pattern', 'seek 10.5', 'wait 1', 'seek 10.6') }
    @{ name = 'curves'; crop = '0,30,1600,720'; steps = @('select title', 'seek 1.5', 'show Curves', 'tool layout.maximize {"pane": "Curves"}') }
    @{ name = 'mixer'; crop = 'Mixer'; steps = @('page Audio', 'select music', 'seek 10.5') }
    @{ name = 'subtitles'; crop = 'Subtitles'; steps = @('page Audio', 'show Subtitles', 'seek 10.5') }
    @{ name = 'voiceover'; crop = '0,30,320,260'; steps = @('act voiceover') }
    @{ name = 'export-panel'; crop = 'Export'; steps = @('page Export') }
    # advanced
    @{ name = 'autocut'; crop = 'Auto-cut'; steps = @('show Auto-cut') }
    @{ name = 'markers'; crop = 'Markers'; steps = @('show Markers') }
    @{ name = 'tracking'; crop = 'Tracking'; steps = @('show Tracking') }
    @{ name = 'planner'; crop = 'Planner'; steps = @('show Planner') }
)

# ---------------------------------------------------------------------------------------------------
$ErrorActionPreference = 'Stop'
$Repo = Split-Path -Parent $PSScriptRoot
$OutDir = Join-Path $Repo 'website\static\img\tutorial'
$W = 1600
$H = 900
$Settle = 1.5 # seconds after the last step, for thumbnails and the preview frame to arrive
$Exe = Join-Path $Work 'se-docs-shots.exe'
$Prof = Join-Path $Work 'profile'
$Media = Join-Path $Work 'media'
$Demo = Join-Path $Work 'demo.sedit'
$Ppm = Join-Path $Work 'shot.ppm'
$Go = Join-Path $Work 'go'
$Version = (Select-String -Path (Join-Path $Repo 'Cargo.toml') -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
$Clips = @{}
$Transitions = @{}
$Assets = @{}

function Invoke-Mcp([string]$Tool, $ToolArgs = @{}, [int]$TimeoutSec = 120) {
    $body = @{ jsonrpc = '2.0'; id = 1; method = 'tools/call'; params = @{ name = $Tool; arguments = $ToolArgs } } |
        ConvertTo-Json -Depth 20 -Compress
    $r = Invoke-RestMethod -Uri "http://127.0.0.1:$Port/mcp" -Method Post -ContentType 'application/json' `
        -Body ([Text.Encoding]::UTF8.GetBytes($body)) -TimeoutSec $TimeoutSec
    if ($r.error) { throw "${Tool}: $($r.error.message)" }
    $text = $r.result.content[0].text
    if ($r.result.isError) { throw "${Tool}: $text" }
    try { return ($text | ConvertFrom-Json) } catch { return $text }
}

function Test-PortBusy {
    $c = New-Object Net.Sockets.TcpClient
    try { $c.Connect('127.0.0.1', $Port); return $true } catch { return $false } finally { $c.Close() }
}

# A clean profile before every launch: fresh settings (MCP on, onboarded, this version's What's New
# already seen, dark theme with a fixed teal accent so shots don't depend on this PC's Windows accent, the
# window at -WindowRect), no autosave (a killed run leaves one, and the next launch would offer to recover
# it) and no project lock.
function Reset-Profile {
    $dir = Join-Path $Prof 'SimpleEditor'
    New-Item -ItemType Directory -Force $dir | Out-Null
    $json = '{"mcp_enabled": true, "mcp_port": ' + $Port + ', "onboarded": true, "last_seen_version": "' + $Version +
        '", "theme": "dark", "palette": {"accent": [0, 183, 195]}, "bg_image": "", "window_rect": [' +
        ($WindowRect -join ', ') + ']}'
    [IO.File]::WriteAllText((Join-Path $dir 'settings.json'), $json, (New-Object Text.UTF8Encoding $false))
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue (Join-Path $Prof 'SimpleEditor\autosave')
    Remove-Item -Force -ErrorAction SilentlyContinue "$Demo.lock"
}

function Start-Editor([string[]]$ArgList) {
    if (Test-PortBusy) { throw "port $Port is already in use - pass -Port <free port>" }
    $saved = @{}
    foreach ($k in 'APPDATA', 'LOCALAPPDATA', 'SE_SCREENSHOT_WHEN', 'SE_BACKGROUND') {
        $saved[$k] = [Environment]::GetEnvironmentVariable($k)
    }
    try {
        $env:APPDATA = $Prof
        $env:LOCALAPPDATA = $Prof
        $env:SE_SCREENSHOT_WHEN = $Go
        $env:SE_BACKGROUND = '1' # open at window_rect, never take focus
        $quoted = $ArgList | ForEach-Object { '"' + $_ + '"' }
        $p = Start-Process -FilePath $Exe -ArgumentList $quoted -PassThru
    } finally {
        foreach ($k in $saved.Keys) { [Environment]::SetEnvironmentVariable($k, $saved[$k]) }
    }
    $t = [Diagnostics.Stopwatch]::StartNew()
    while ($true) {
        if ($p.HasExited) { throw "the app exited during startup (code $($p.ExitCode))" }
        try { $null = Invoke-Mcp 'project.summary' @{} 5; $script:ReadyAt = Get-Date; break } catch { }
        if ($t.Elapsed.TotalSeconds -gt 60) { Stop-Process -Id $p.Id -Force; throw "no MCP answer on port $Port" }
        Start-Sleep -Milliseconds 250
    }
    return $p
}

# Background jobs (waveform peaks, probes) finished, so the menu bar's jobs counter isn't in the shot.
function Wait-Idle {
    $t = [Diagnostics.Stopwatch]::StartNew()
    while ((Invoke-Mcp 'jobs.list').running -gt 0 -and $t.Elapsed.TotalSeconds -lt 60) { Start-Sleep -Milliseconds 250 }
}

function Get-PaneRect([string]$Pane) {
    $p = (Invoke-Mcp 'layout.list').panes | Where-Object { $_.name -eq $Pane }
    if (-not $p -or -not $p.rect) { throw "panel '$Pane' is not on screen" }
    return $p.rect
}

function Resolve-Point([string]$Pane, [double]$X, [double]$Y) {
    if ($Pane -eq '@') { return @($X, $Y) }
    $r = Get-PaneRect $Pane
    if ($X -lt 0) { $X = $r[2] + $X } else { $X = $r[0] + $X }
    if ($Y -lt 0) { $Y = $r[3] + $Y } else { $Y = $r[1] + $Y }
    return @($X, $Y)
}

function Invoke-Step([string]$Step) {
    $verb, $rest = $Step -split ' ', 2
    switch ($verb) {
        'page' { $null = Invoke-Mcp 'layout.page' @{ name = $rest } }
        'seek' { $null = Invoke-Mcp 'playback.seek' @{ t = [double]$rest } }
        'act' { $null = Invoke-Mcp 'ui.action' @{ id = $rest } }
        'show' { $null = Invoke-Mcp 'layout.surface' @{ pane = $rest; force = $true } }
        'open' {
            $name, $at = $rest -split ' '
            $null = Invoke-Mcp 'source.open' @{ asset_id = $Assets[$name]; seek = [double]$at }
        }
        'select' {
            $c = @(); $tr = @()
            foreach ($n in $rest -split ',') {
                if ($Transitions.ContainsKey($n)) { $tr += $Transitions[$n] }
                elseif ($Clips.ContainsKey($n)) { $c += $Clips[$n] }
                else { throw "unknown demo clip '$n'" }
            }
            $null = Invoke-Mcp 'selection.set' @{ clip_ids = $c; transition_ids = $tr }
        }
        { $_ -in 'click', 'rclick', 'dclick', 'move' } {
            $pane, $x, $y, $mods = $rest -split ' '
            $pt = Resolve-Point $pane ([double]$x) ([double]$y)
            $e = @{ type = $verb; x = $pt[0]; y = $pt[1] }
            if ($mods) { $e.mods = $mods }
            $null = Invoke-Mcp 'ui.input' @{ events = @($e) }
        }
        'key' {
            $key, $mods = $rest -split ' '
            $e = @{ type = 'key'; key = $key }
            if ($mods) { $e.mods = $mods }
            $null = Invoke-Mcp 'ui.input' @{ events = @($e) }
        }
        'type' { $null = Invoke-Mcp 'ui.input' @{ events = @(@{ type = 'text'; text = $rest }) } }
        'tool' {
            $name, $json = $rest -split ' ', 2
            $a = @{}
            if ($json) { ($json | ConvertFrom-Json).PSObject.Properties | ForEach-Object { $a[$_.Name] = $_.Value } }
            $null = Invoke-Mcp $name $a
        }
        'wait' { Start-Sleep -Milliseconds ([int]([double]$rest * 1000)) }
        default { throw "unknown step '$Step'" }
    }
}

# '' | 'Timeline' | 'Library+Source' | '0,0,400,300' | mixes -> [x0, y0, x1, y1] in window points
function Resolve-Crop([string]$Crop) {
    if (-not $Crop) { return @(0, 0, $W, $H) }
    $x0 = [double]::MaxValue; $y0 = [double]::MaxValue; $x1 = 0; $y1 = 0
    foreach ($part in $Crop -split '\+') {
        if ($part -match ',') { $r = $part -split ',' | ForEach-Object { [double]$_ } } else { $r = Get-PaneRect $part }
        $x0 = [math]::Min($x0, $r[0]); $y0 = [math]::Min($y0, $r[1]); $x1 = [math]::Max($x1, $r[2]); $y1 = [math]::Max($y1, $r[3])
    }
    return @($x0, $y0, $x1, $y1)
}

function New-DemoMedia {
    New-Item -ItemType Directory -Force $Media | Out-Null
    & $Exe --selftest (Join-Path $Work 'selftest') | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "--selftest failed (exit $LASTEXITCODE)" }
    $v = @('-c:v', 'libx264', '-pix_fmt', 'yuv420p', '-preset', 'veryfast', '-crf', '23', '-c:a', 'aac', '-shortest')
    $clips = @(
        @('fractal.mp4', 'mandelbrot=s=1280x720:r=30:start_scale=3:end_scale=0.3', 'aevalsrc=0.4*sin(2*PI*196*t)*(0.5+0.5*sin(2*PI*0.5*t)):s=48000', 8),
        @('pattern.mp4', 'testsrc2=s=1280x720:r=30', 'sine=f=440:sample_rate=48000', 6),
        @('gradient.mp4', 'gradients=s=1280x720:r=30:speed=0.03:c0=0x00b7c3:c1=0x1b1b1b:c2=0x5a2a82', 'sine=f=330:sample_rate=48000', 6),
        @('bars.mp4', 'smptehdbars=s=1280x720:r=30', 'sine=f=1000:sample_rate=48000', 5)
    )
    foreach ($c in $clips) {
        & ffmpeg -hide_banner -loglevel error -y -f lavfi -i $c[1] -f lavfi -i $c[2] -t $c[3] @v (Join-Path $Media $c[0])
        if ($LASTEXITCODE -ne 0) { throw "ffmpeg failed on $($c[0])" }
    }
    # a 120 BPM chord for music (beats detect cleanly) and a "voice" that talks for 2.2 s out of every 3
    & ffmpeg -hide_banner -loglevel error -y -f lavfi -i 'aevalsrc=0.25*(sin(2*PI*261.6*t)+sin(2*PI*329.6*t)+sin(2*PI*392*t))*exp(-4*mod(t\,0.5)):s=48000:d=24' (Join-Path $Media 'music.wav')
    & ffmpeg -hide_banner -loglevel error -y -f lavfi -i 'aevalsrc=0.6*sin(2*PI*180*t)*sin(2*PI*3*t)*gte(mod(t\,3)\,0.8):s=48000:d=12' (Join-Path $Media 'voice.wav')
    if ($LASTEXITCODE -ne 0) { throw 'ffmpeg failed on the audio clips' }
    Copy-Item (Join-Path $Work 'selftest\logo.png'), (Join-Path $Work 'selftest\test.mp4') $Media
}

# The demo project every shot opens. Clip ids go into $Clips / $Transitions under the names the steps use.
function New-DemoProject {
    Reset-Profile
    $p = Start-Editor @('--size', "${W}x${H}")
    try {
        $names = 'fractal.mp4', 'pattern.mp4', 'gradient.mp4', 'bars.mp4', 'music.wav', 'voice.wav', 'logo.png', 'test.mp4'
        $ids = (Invoke-Mcp 'media.import' @{ paths = @($names | ForEach-Object { Join-Path $Media $_ }) }).asset_ids
        $a = @{}
        for ($i = 0; $i -lt $names.Count; $i++) { $a[$names[$i]] = $ids[$i]; $Assets[($names[$i] -split '\.')[0]] = $ids[$i] }
        # imports are probed in the background; a clip placed before its asset has a duration gets none
        $t = [Diagnostics.Stopwatch]::StartNew()
        while (@(Invoke-Mcp 'media.list' | Where-Object { $_.kind -ne 'Image' -and $_.duration -le 0 }).Count -gt 0) {
            if ($t.Elapsed.TotalSeconds -gt 60) { throw 'the demo media never finished probing' }
            Start-Sleep -Milliseconds 250
        }
        $place = { param($asset, $at, $name) $r = (Invoke-Mcp 'timeline.add_clip' @{ asset_id = $a[$asset]; at = $at }).clip_ids
            $script:Clips[$name] = $r[0]; if ($r.Count -gt 1) { $script:Clips["$name-a"] = $r[1] } }
        & $place 'fractal.mp4' 0 'fractal'
        & $place 'pattern.mp4' 8 'pattern'
        & $place 'gradient.mp4' 14 'gradient'
        & $place 'bars.mp4' 20 'bars'
        & $place 'music.wav' 0 'music'
        $Clips['title'] = (Invoke-Mcp 'timeline.add_clip' @{ text = 'Simple Editor'; at = 1; duration = 4 }).clip_ids[0]
        $null = Invoke-Mcp 'clip.set' @{ clip_id = $Clips['title']; fields = @{ name = 'Title'; size = 110; bold = $true; y = -190 } }
        $Clips['logo'] = (Invoke-Mcp 'timeline.add_clip' @{ asset_id = $a['logo.png']; at = 15; track = 1 }).clip_ids[0]
        $Transitions['xfade'] = (Invoke-Mcp 'timeline.add_transition' @{ right_clip_id = $Clips['pattern']; kind = 'CrossFade'; duration = 1 }).transition_id
        $seq = (Invoke-Mcp 'timeline.nest' @{ clip_ids = @($Clips['bars'], $Clips['bars-a']); name = 'Bars sequence' }).sequence_id
        $nested = (Invoke-Mcp 'timeline.list').tracks | ForEach-Object { $_.clips } | Where-Object { $_.sequence -eq $seq }
        $Clips['nest'] = @($nested)[0].id # video tracks are listed first, then the audio twin
        $Clips['nest-a'] = @($nested)[1].id
        $Clips.Remove('bars'); $Clips.Remove('bars-a')
        foreach ($m in @(@(3, 'Intro'), @(8, 'B-roll'), @(14, 'Outro'))) { $null = Invoke-Mcp 'markers.add' @{ t = $m[0]; name = $m[1] } }
        $null = Invoke-Mcp 'subtitles.set' @{ cues = @(
                @{ start = 1; end = 3.5; text = 'Welcome to Simple Editor' },
                @{ start = 4; end = 7; text = 'Cut, trim and grade in one small app' },
                @{ start = 9; end = 12; text = 'Everything else is one right-click away' }) }
        # the title zooms and fades in and the gradient zooms slowly (keyframes to show), the gradient has an
        # effect, the music a bus
        foreach ($k in @(@('Scale', 0, 0.6), @('Scale', 1.5, 1.0), @('Opacity', 0, 0), @('Opacity', 0.8, 1))) {
            $null = Invoke-Mcp 'clip.keyframe' @{ clip_id = $Clips['title']; property = $k[0]; t = $k[1]; value = $k[2] }
        }
        foreach ($k in @(@(0, 1.0), @(6, 1.15))) {
            $null = Invoke-Mcp 'clip.keyframe' @{ clip_id = $Clips['gradient']; property = 'Scale'; t = $k[0]; value = $k[1] }
        }
        $null = Invoke-Mcp 'clip.add_effect' @{ clip_id = $Clips['gradient']; kind = 'Vignette' }
        $bus = (Invoke-Mcp 'audio.add_bus' @{ name = 'Music' }).bus_id
        $null = Invoke-Mcp 'audio.route' @{ clip_id = $Clips['music']; bus = $bus }
        $null = Invoke-Mcp 'audio.filter_add' @{ bus_id = $bus; kind = 'Eq' }
        $plan = foreach ($t in 'Rough cut', 'Titles and captions', 'Colour pass', 'Export for YouTube and Shorts') {
            (Invoke-Mcp 'plan.add' @{ title = $t }).id
        }
        $null = Invoke-Mcp 'plan.set' @{ id = $plan[0]; done = $true }
        $null = Invoke-Mcp 'project.set' @{ name = 'Demo' }
        $null = Invoke-Mcp 'project.save' @{ path = $Demo }
        Wait-Idle # waveform peaks and proxies land in the profile's cache, so every shot's launch finds them
    } finally {
        Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
        $p.WaitForExit()
    }
}

function Invoke-Shot($Shot) {
    Reset-Profile
    Remove-Item -Force -ErrorAction SilentlyContinue $Ppm, $Go
    $argList = @('--size', "${W}x${H}")
    if (-not $Shot.empty) { $argList += $Demo }
    $argList += @('--screenshot', $Ppm)
    $p = Start-Editor $argList
    try {
        Wait-Idle
        if (-not $Shot.empty) { Invoke-Step 'act zoom_fit'; Invoke-Step 'wait 0.3' }
        foreach ($s in $Shot.steps) { Invoke-Step $s }
        Start-Sleep -Milliseconds ([int]($Settle * 1000))
        # the startup toasts ("GPU preview: ...", "MCP server at ...", both from before MCP answered) live 5 s
        $age = ((Get-Date) - $ReadyAt).TotalSeconds
        if ($age -lt 5.3) { Start-Sleep -Milliseconds ([int]((5.3 - $age) * 1000)) }
        $crop = Resolve-Crop $Shot.crop
        New-Item -ItemType File $Go | Out-Null
        $t = [Diagnostics.Stopwatch]::StartNew()
        $last = -1
        while ($true) {
            $len = if (Test-Path $Ppm) { (Get-Item $Ppm).Length } else { 0 }
            if ($len -gt 1000 -and $len -eq $last) { break }
            if ($t.Elapsed.TotalSeconds -gt 30) { throw 'no screenshot within 30 s' }
            $last = $len
            Start-Sleep -Milliseconds 300
        }
    } finally {
        # the shot is written before the app tries to close (and an edited project would ask to save)
        Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
        $p.WaitForExit()
    }
    # the PPM is in pixels, the crop in points: scale by the window's pixels-per-point
    $fs = [IO.File]::OpenRead($Ppm)
    try { $hdr = New-Object byte[] 32; $null = $fs.Read($hdr, 0, 32) } finally { $fs.Close() }
    $size = ([Text.Encoding]::ASCII.GetString($hdr) -split '\s+')[1..2] | ForEach-Object { [double]$_ }
    $ppp = $size[0] / $W
    # someone maximised or resized the window while it was up: the crop no longer fits it
    if ([math]::Abs($size[1] / $H - $ppp) -gt 0.01) { Write-Warning "$($Shot.name): the window was resized, retrying"; return $false }
    $c = $crop | ForEach-Object { [math]::Round($_ * $ppp) }
    $out = Join-Path $OutDir "$($Shot.name).png"
    # 256 colours: UI shots stay crisp and come out about a third of the size of a full-colour PNG
    $vf = "crop=$($c[2] - $c[0]):$($c[3] - $c[1]):$($c[0]):$($c[1]),split[a][b];[a]palettegen=stats_mode=full[p];[b][p]paletteuse"
    & ffmpeg -hide_banner -loglevel error -y -i $Ppm -vf $vf $out
    if ($LASTEXITCODE -ne 0) { throw "ffmpeg could not write $out" }
    Write-Host ("  {0,-28} {1,5} KB" -f $Shot.name, [math]::Round((Get-Item $out).Length / 1KB))
    return $true
}

# ---------------------------------------------------------------------------------------------------
Push-Location $Repo
try {
    if (-not $FromExe) {
        cargo build
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed (exit $LASTEXITCODE)" }
        $FromExe = Join-Path (cargo metadata --format-version 1 --no-deps | ConvertFrom-Json).target_directory 'debug\simple-editor.exe'
    }
    $FromExe = (Resolve-Path $FromExe).Path
    if (Test-Path $Work) { Remove-Item -Recurse -Force $Work }
    New-Item -ItemType Directory -Force $Work, $OutDir | Out-Null
    # a private copy: other worktrees share the target dir and may rebuild the exe while we run
    Copy-Item $FromExe $Exe
    if (-not (Select-String -Path $Exe -Pattern 'SE_SCREENSHOT_WHEN' -SimpleMatch -Quiet)) {
        throw "$FromExe predates SE_SCREENSHOT_WHEN (another checkout's build?) - rebuild, or pass -FromExe"
    }
    $clock = [Diagnostics.Stopwatch]::StartNew()
    Write-Host 'Making the demo media and project...'
    New-DemoMedia
    New-DemoProject
    Write-Host ("  done in {0:n0} s" -f $clock.Elapsed.TotalSeconds)
    $todo = $Shots | Where-Object { $n = $_.name; @($Only | Where-Object { $n -like $_ }).Count -gt 0 }
    Write-Host "Taking $(@($todo).Count) screenshots into $OutDir"
    foreach ($s in $todo) {
        if (-not (Invoke-Shot $s) -and -not (Invoke-Shot $s)) { throw "$($s.name): the window keeps being resized" }
    }
    # the README's screenshot is the Edit page shot
    if (@($todo | Where-Object { $_.name -eq 'edit-page' }).Count -gt 0) {
        Copy-Item (Join-Path $OutDir 'edit-page.png') (Join-Path $Repo 'docs\images\screenshot.png')
    }
    Write-Host ("All done in {0:n0} s - now look at the images." -f $clock.Elapsed.TotalSeconds)
} finally {
    Pop-Location
}
