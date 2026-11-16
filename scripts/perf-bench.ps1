# perf-bench.ps1 - isolated playback benchmark. Never touches the user's profile, caches or live editor.
#   scripts/perf-bench.ps1 -Synthetic                        # the gate: ffmpeg testsrc stress project
#   scripts/perf-bench.ps1 -Project X.sedit -Label before    # a COPY of a real project
# Runs -Exe in a scratch profile (APPDATA/LOCALAPPDATA redirected, user caches only hard-linked,
# SE_BACKGROUND=1, own MCP port), plays -From..-To, writes per-window CSV + a summary line, appends a
# row to perf_log.csv (sha,label,dropped,published,worst_gap_ms,ui_p95_ms,open_ms,rss_mb), then kills
# that instance by PID. With -Synthetic it also GATES: exit 1 when dropped > 0, worst gap > 50 ms, or any
# metric is >10% worse than the previous perf_log.csv row with the same label.
param([string]$Exe, [string]$Project, [string]$Label, [switch]$Synthetic, [int]$Port = 7492, [int]$Window = 5,
  [double]$From = 0, [double]$To = 0, [int]$Settle = 10, [int]$CallTimeout = 300, [switch]$NoLog)
$ErrorActionPreference = 'Stop'
$repo = Split-Path (Split-Path $MyInvocation.MyCommand.Path)
if (-not $Exe) { $Exe = "$repo\target\release\simple-editor.exe" }
if (-not $Label) { $Label = if ($Synthetic) { 'synthetic' } else { [IO.Path]::GetFileNameWithoutExtension($Project) } }
if (-not $Synthetic -and -not (Test-Path $Project)) { throw "pass -Project <file.sedit> or -Synthetic" }
if ($Port -eq 7878) { throw "7878 is the live editor's port" }
$work = "$env:TEMP\se-perf"; $prof = "$work\prof-$Label"; $out = "$work\out"
Remove-Item "$prof\appdata" -Recurse -Force -ErrorAction SilentlyContinue  # no recovery state left by a killed run
New-Item -ItemType Directory -Force "$prof\appdata\SimpleEditor","$prof\local\SimpleEditor\cache", $out | Out-Null
if (-not $Synthetic) {  # hard links: zero disk, the user's files are never written
  foreach ($d in 'proxies', 'html') {
    $src = "$env:LOCALAPPDATA\SimpleEditor\cache\$d"; $dst = "$prof\local\SimpleEditor\cache\$d"
    if (-not (Test-Path $src)) { continue }
    New-Item -ItemType Directory -Force $dst | Out-Null
    Get-ChildItem $src -Recurse -File | ForEach-Object { $t = "$dst$($_.FullName.Substring($src.Length))"
      New-Item -ItemType Directory -Force (Split-Path $t) | Out-Null
      if (-not (Test-Path $t)) { New-Item -ItemType HardLink -Path $t -Target $_.FullName | Out-Null } }
  }
}
"{""mcp_enabled"":true,""mcp_port"":$Port,""assistant_guard"":false,""start_maximized"":false,""gpu"":true,""cache_mb"":0,""buffer_before_play"":true}" | Set-Content -Encoding ascii "$prof\appdata\SimpleEditor\settings.json"
$proj = "$prof\project.sedit"
if (-not $Synthetic) { Copy-Item $Project $proj -Force }

function Call($n, $a = @{}) {
  $b = @{jsonrpc = "2.0"; id = 1; method = "tools/call"; params = @{name = $n; arguments = $a } } | ConvertTo-Json -Depth 8 -Compress
  for ($try = 0; ; $try++) {  # the server drops idle keep-alive sockets: retry those (the request never reached it)
    try { $r = Invoke-WebRequest "http://127.0.0.1:$Port/mcp" -Method Post -ContentType application/json -Body $b -UseBasicParsing -TimeoutSec $CallTimeout; break }
    catch { if ($try -ge 2 -or "$_" -notmatch 'connection was closed') { if (-not $script:quiet) { Write-Host "call $n failed: $_" }; throw }; Start-Sleep -Milliseconds 200 }
  }
  ($r.Content | ConvertFrom-Json).result.content[0].text
}
function Media {  # cached ffmpeg testsrc media for the synthetic project
  $m = "$work\media"; New-Item -ItemType Directory -Force $m | Out-Null
  $v = [ordered]@{
    '1080p60.mp4' = '-f lavfi -i testsrc2=s=1920x1080:r=60:d=30 -c:v libx264 -preset veryfast -crf 20 -pix_fmt yuv420p'
    '1440p60-120M.mp4' = '-f lavfi -i testsrc2=s=2560x1440:r=60:d=15 -c:v libx264 -preset veryfast -b:v 120M -maxrate 120M -bufsize 60M -pix_fmt yuv420p'
    'overlay-png.mov' = '-f lavfi -i testsrc2=s=1280x720:r=30:d=10 -vf format=rgba,colorchannelmixer=aa=0.6 -c:v png'
    'longgop-9min.mp4' = '-f lavfi -i testsrc=s=1280x720:r=30:d=540 -c:v libx264 -preset ultrafast -crf 32 -g 900 -pix_fmt yuv420p'
  }
  foreach ($k in $v.Keys) {
    if (-not (Test-Path "$m\$k")) { & ffmpeg -y -v error ($v[$k] -split ' ') "$m\$k"; if ($LASTEXITCODE) { throw "ffmpeg $k" } }
  }
  $m
}
function Build($m) {  # stress project via MCP, saved to $proj
  Call project.new @{width = 1920; height = 1080; fps = 60} | Out-Null
  $a = '1080p60.mp4', '1440p60-120M.mp4', 'overlay-png.mov', 'longgop-9min.mp4' | ForEach-Object {  # one call each: a 4-file import can outlast the server
    (Call media.import @{paths = @("$m\$_")} | ConvertFrom-Json).asset_ids[0] }
  Call timeline.add_clip @{asset_id = $a[0]; at = 0; track = 0} | Out-Null
  Call timeline.add_clip @{asset_id = $a[1]; at = 30; track = 0} | Out-Null
  Call timeline.add_clip @{asset_id = $a[2]; at = 0; track = 1} | Out-Null
  $g = (Call timeline.add_clip @{asset_id = $a[3]; at = 45; track = 0} | ConvertFrom-Json).clip_ids[0]
  Call timeline.trim @{clip_id = $g; end = 65} | Out-Null
  Call timeline.slip @{clip_ids = @($g); dsrc = 300} | Out-Null          # the clip starts 300 s into a long-GOP file
  $n = (Call timeline.add_clip @{asset_id = $a[0]; at = 65; track = 0} | ConvertFrom-Json).clip_ids[0]
  Call timeline.trim @{clip_id = $n; end = 75} | Out-Null
  Call timeline.nest @{clip_ids = @($n); name = 'nested'} | Out-Null     # a nested sequence on the timeline
  Call project.save @{path = $proj} | Out-Null
}

if ($Synthetic) { $mediaDir = Media }
$env:APPDATA = "$prof\appdata"; $env:LOCALAPPDATA = "$prof\local"; $env:SE_BACKGROUND = "1"
$p = Start-Process $Exe -PassThru
try {
  $script:quiet = $true
  for ($i = 0; $i -lt 60; $i++) { try { Call playback.status | Out-Null; break } catch { Start-Sleep 1 } }
  $script:quiet = $false
  if ($Synthetic) { Build $mediaDir }
  $ot = [Diagnostics.Stopwatch]::StartNew()
  Call project.open @{path = $proj} | Out-Null
  $script:quiet = $true
  while ($true) { try { Call playback.status | Out-Null; break } catch { Start-Sleep -Milliseconds 50 } }
  $script:quiet = $false
  $openMs = [int]$ot.ElapsedMilliseconds     # open to interactive: the app answers again
  Start-Sleep $Settle                        # let stills/thumbs/sources open like a user would
  if ($To -le 0) { $To = if ($Synthetic) { 75 } else { (Call project.summary | ConvertFrom-Json).duration } }  # project.summary can be slow while media opens: pass -To to skip it
  $rows = @()
  Call playback.seek @{t = $From} | Out-Null; Start-Sleep 2
  $prev = Call playback.status | ConvertFrom-Json; $tPrev = $From
  $sw = [Diagnostics.Stopwatch]::StartNew(); Call playback.play | Out-Null
  while ($true) {
    Start-Sleep $Window
    $s = Call playback.status | ConvertFrom-Json
    $rows += [pscustomobject]@{ t0 = [math]::Round($tPrev, 1); t1 = [math]::Round($s.time, 1); published = $s.published_frames - $prev.published_frames
      dropped = $s.dropped_frames - $prev.dropped_frames; worst_gap_ms = [math]::Round($s.worst_gap_ms); decode_worst = [math]::Round($s.decode_worst_ms, 1)
      ui_p95 = [math]::Round($s.ui_frame_p95_ms, 1); state = $s.state }
    $prev = $s; $tPrev = $s.time
    if ($s.time -ge $To - 0.2 -or (-not $s.playing -and $s.state -eq 'paused') -or $sw.Elapsed.TotalSeconds -gt ($To - $From) * 3 + 60) { break }
  }
  Call playback.pause | Out-Null
  $wall = $sw.Elapsed.TotalSeconds
  $rss = [int]((Get-Process -Id $p.Id).WorkingSet64 / 1MB)
  $rows | Export-Csv "$out\bench-$Label.csv" -NoTypeInformation
  $tp = ($rows | Measure-Object published -Sum).Sum; $td = ($rows | Measure-Object dropped -Sum).Sum
  $wg = ($rows | Measure-Object worst_gap_ms -Maximum).Maximum; $u = ($rows | Measure-Object ui_p95 -Maximum).Maximum
  $sum = "[$Label] {0:N0}-{1:N0}s in {2:N0}s wall | published {3} dropped {4} | worst gap {5} ms | max ui p95 {6} ms | open {7} ms | RSS {8} MB" -f $From, $To, $wall, $tp, $td, $wg, $u, $openMs, $rss
  $sum; $sum | Add-Content "$out\bench-summary.txt"
} finally { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }

# perf_log.csv + gate
$log = "$repo\perf_log.csv"; $sha = (git -C $repo rev-parse --short HEAD).Trim()
$cur = [pscustomobject]@{ sha = $sha; label = $Label; dropped = $td; published = $tp; worst_gap_ms = $wg; ui_p95_ms = $u; open_ms = $openMs; rss_mb = $rss }
$prevRow = $null
if (Test-Path $log) { $prevRow = Import-Csv $log | Where-Object { $_.label -eq $Label } | Select-Object -Last 1 }
if (-not $NoLog) {
  if (-not (Test-Path $log)) { 'sha,label,dropped,published,worst_gap_ms,ui_p95_ms,open_ms,rss_mb' | Set-Content -Encoding ascii $log }
  ($cur.psobject.Properties.Value -join ',') | Add-Content -Encoding ascii $log
}
if ($Synthetic) {
  $bad = @()
  if ($td -gt 0) { $bad += "dropped frames $td > 0" }
  if ($wg -gt 50) { $bad += "worst gap $wg ms > 50" }
  if ($prevRow) {
    foreach ($m in 'dropped', 'worst_gap_ms', 'ui_p95_ms', 'open_ms', 'rss_mb') {
      if ([double]$cur.$m -gt [double]$prevRow.$m * 1.1 -and [double]$cur.$m -gt [double]$prevRow.$m) { $bad += "$m $($cur.$m) is >10% worse than $($prevRow.$m) ($($prevRow.sha))" }
    }
    if ([double]$cur.published -lt [double]$prevRow.published * 0.9) { $bad += "published $($cur.published) is >10% below $($prevRow.published)" }
  }
  if ($bad) { "PERF GATE FAILED:"; $bad | ForEach-Object { "  $_" }; exit 1 } else { "PERF GATE OK" }
}
