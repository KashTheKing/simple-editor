# Launch-to-first-paint, timed from outside, the same way for every app:
#   start the process -> poll the screen under its top-level window's client area every few ms ->
#   stop when the middle of the window shows the picture (the test clip is colourful: high variance)
#   or, for an app launched without a file, when the window shows painted UI (any variance at all).
#
#   scripts/launch-bench.ps1 -Exe target\debug\simple-editor.exe -File clip.mp4 -Runs 7
#   scripts/launch-bench.ps1 -MediaPlayer -File clip.mp4 -Runs 7     (Windows Media Player, the app)
#   scripts/launch-bench.ps1 -Exe ... -Runs 7                        (no file: the editor's startup)
#
# Our exe runs with a scratch APPDATA / LOCALAPPDATA and SE_BACKGROUND=1 (never the user's profile).
# Media Player is refused while it is already running: the launch would hand the file to the user's
# own window. Every process started here is killed by PID afterwards.
param(
    [string]$Exe,
    [string]$File,
    [switch]$MediaPlayer,
    [int]$Runs = 7,
    [int]$TimeoutMs = 15000
)
$ErrorActionPreference = 'Stop'

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;

public static class LaunchBench {
    delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr l);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern bool GetClientRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] static extern bool ClientToScreen(IntPtr h, ref POINT p);
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }

    public static IntPtr Window(HashSet<uint> pids) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h, l) => {
            uint pid; GetWindowThreadProcessId(h, out pid);
            RECT r;
            if (pids.Contains(pid) && IsWindowVisible(h) && GetClientRect(h, out r) && r.R - r.L > 200 && r.B - r.T > 150) {
                found = h; return false;
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }

    // Luminance standard deviation over a grid in the middle of the window (whole = the whole window).
    public static double Variance(IntPtr h, bool whole) {
        RECT r; if (!GetClientRect(h, out r)) return 0;
        int w = r.R - r.L, ht = r.B - r.T; if (w < 16 || ht < 16) return 0;
        // what is on screen at the client area (PrintWindow returns nothing for a GL/D3D swap chain)
        POINT o = new POINT(); if (!ClientToScreen(h, ref o)) return 0;
        using (var bmp = new Bitmap(w, ht, PixelFormat.Format32bppArgb)) {
            using (var g = Graphics.FromImage(bmp)) {
                try { g.CopyFromScreen(o.X, o.Y, 0, 0, new Size(w, ht)); } catch { return 0; }
            }
            int x0 = whole ? 0 : w / 4, x1 = whole ? w : w * 3 / 4, y0 = whole ? 0 : ht / 4, y1 = whole ? ht : ht * 3 / 4;
            double s = 0, s2 = 0; int n = 0;
            for (int y = y0; y < y1; y += Math.Max(1, (y1 - y0) / 24))
                for (int x = x0; x < x1; x += Math.Max(1, (x1 - x0) / 32)) {
                    Color c = bmp.GetPixel(x, y);
                    double l = 0.2126 * c.R + 0.7152 * c.G + 0.0722 * c.B;
                    s += l; s2 += l * l; n++;
                }
            double m = s / n; return Math.Sqrt(Math.Max(0, s2 / n - m * m));
        }
    }
}
'@ -ReferencedAssemblies System.Drawing

function Median($xs) { $s = $xs | Sort-Object; $s[[int][math]::Floor($s.Count / 2)] }

$scratch = Join-Path $env:TEMP 'se-launch-bench'
New-Item -ItemType Directory -Force $scratch | Out-Null
if ($MediaPlayer -and (Get-Process -Name 'Microsoft.Media.Player' -ErrorAction SilentlyContinue)) {
    throw 'Media Player is already running: close it first (launching would take over its window).'
}
$whole = -not $File
# the picture is up when the middle of the window varies this much (testsrc2: ~60; a black/flat frame: ~0)
$threshold = if ($File) { 25 } else { 6 }
$results = @()
for ($i = 0; $i -le $Runs; $i++) {   # run 0 warms the file cache and is not counted
    $before = @(Get-Process -Name 'Microsoft.Media.Player' -ErrorAction SilentlyContinue | ForEach-Object Id)
    $sw = [Diagnostics.Stopwatch]::StartNew()
    if ($MediaPlayer) {
        $alias = Join-Path $env:LOCALAPPDATA 'Microsoft\WindowsApps\MediaPlayer.exe'
        $p = if ($File) { Start-Process $alias -ArgumentList "`"$File`"" -PassThru } else { Start-Process $alias -PassThru }
    } else {
        $psi = New-Object Diagnostics.ProcessStartInfo $Exe
        if ($File) { $psi.Arguments = "`"$File`"" }
        $psi.UseShellExecute = $false
        $psi.EnvironmentVariables['APPDATA'] = Join-Path $scratch 'appdata'
        $psi.EnvironmentVariables['LOCALAPPDATA'] = Join-Path $scratch 'localappdata'
        $psi.EnvironmentVariables['SE_BACKGROUND'] = '1'
        $p = [Diagnostics.Process]::Start($psi)
    }
    $hwnd = [IntPtr]::Zero; $visibleMs = $null; $paintMs = $null; $pids = New-Object 'System.Collections.Generic.HashSet[uint32]'
    while ($sw.ElapsedMilliseconds -lt $TimeoutMs) {
        if ($MediaPlayer) {
            Get-Process -Name 'Microsoft.Media.Player' -ErrorAction SilentlyContinue | Where-Object { $before -notcontains $_.Id } | ForEach-Object { [void]$pids.Add([uint32]$_.Id) }
        } else { [void]$pids.Add([uint32]$p.Id) }
        if ($hwnd -eq [IntPtr]::Zero -and $pids.Count -gt 0) {
            $hwnd = [LaunchBench]::Window($pids)
            if ($hwnd -ne [IntPtr]::Zero) { $visibleMs = $sw.ElapsedMilliseconds }
        }
        if ($hwnd -ne [IntPtr]::Zero -and [LaunchBench]::Variance($hwnd, $whole) -ge $threshold) {
            $paintMs = $sw.ElapsedMilliseconds; break
        }
        Start-Sleep -Milliseconds 2
    }
    foreach ($id in @($pids) + @($p.Id)) { Stop-Process -Id $id -Force -ErrorAction SilentlyContinue }
    Start-Sleep -Milliseconds 700
    if ($i -gt 0) {
        $results += [pscustomobject]@{ run = $i; window_ms = $visibleMs; first_paint_ms = $paintMs }
    }
}
$results | Format-Table -AutoSize | Out-String | Write-Host
$ok = @($results | Where-Object { $_.first_paint_ms } | ForEach-Object first_paint_ms)
$w = @($results | Where-Object { $_.window_ms } | ForEach-Object window_ms)
[pscustomobject]@{
    app = if ($MediaPlayer) { 'Media Player' } else { Split-Path $Exe -Leaf }
    file = if ($File) { Split-Path $File -Leaf } else { '(none)' }
    runs = $results.Count
    median_window_ms = if ($w.Count) { Median $w } else { $null }
    median_first_paint_ms = if ($ok.Count) { Median $ok } else { $null }
    timeouts = $results.Count - $ok.Count
} | Format-List
