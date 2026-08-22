# Simple Editor

A tiny, fast video trimmer/editor for Windows. Open a video, watch it, cut it, add music/SFX, overlay
images/video/text, keyframe things — and save over the original or export to any format.

* **Rust + egui**, single ~10 MB exe, starts instantly, low CPU/GPU/memory.
* **Decoding**: Windows Media Foundation (native system codecs, no extra DLLs, instant seeks); ffmpeg.exe as a
  universal fallback (and for images).
* **Export**: ffmpeg.exe — any container/codec by extension (mp4, mov, mkv, webm, avi, gif, mp3, wav…),
  hardware encoders (NVENC/QSV/AMF) selectable in Settings; **Fast Lossless Cut** (`-c copy`) for plain
  cuts — optionally also for Ctrl+S (Settings ▸ General), instant but keyframe-accurate.
* **Timeline**: multiple video + audio tracks, per-track mute/solo, linked audio/video clips, split
  (Ctrl+B), ripple delete, trim by dragging edges, snapping, waveforms, keyframe diamonds, drag-to-resize
  track heights, Ctrl+Scroll zoom / Shift+Scroll pan / Alt+Scroll track height, draggable playhead, in/out points.
* **Inspector**: position / scale / rotation / opacity / volume with keyframes, 13 blend modes, text tool
  (system fonts, size, bold/italic, fill/outline/shadow/box, alignment, spacing).
* **Import URL…** (Library): paste a link and yt-dlp downloads it straight into the library — audio-only
  for music/SFX. The button only appears when yt-dlp is installed; set its folder in Settings.
* **Library** + **Recent** panels (recent media across projects), drag & drop from Explorer.
* **Theme**: follows Windows dark/light + accent colour. Bare Windows-forms style, DaVinci-like layout.
* **Hotkeys**: every action rebindable in Settings.
* **Explorer context menu**: "Edit with Simple Editor" on video files (classic menu / "Show more options").
* **Project format**: plain JSON (`.sedit`) + export to Final Cut Pro 7 XML for Premiere Pro / DaVinci Resolve.

## Requirements

* Windows 10/11 (x64).
* [FFmpeg](https://ffmpeg.org) on PATH (or set its folder in Settings) for export, images and fallback
  decoding: `winget install Gyan.FFmpeg` or `choco install ffmpeg`. Playback of common files works
  without it.
* HEVC/AV1 playback through Media Foundation needs the Microsoft Store codec extensions; otherwise the
  ffmpeg fallback decoder is used automatically.

## Build

```
cargo build --release
```

`target\release\simple-editor.exe` (release profile: LTO, 1 codegen unit, stripped).
Requires Rust ≥ 1.88 (pinned to egui 0.33 for that reason).

## Use

* `simple-editor.exe video.mp4` (or right-click a video → *Edit with Simple Editor*).
* Space play/pause · ←/→ frame step · ↑/↓ previous/next cut · Ctrl+B split · Del / Shift+Del (ripple)
  · I / O mark in/out · Ctrl+T trim to in/out · T add text · Ctrl+S save (overwrite the opened video or
  save the project) · Ctrl+E export as… · Ctrl+Alt+E fast lossless cut · Ctrl+Shift+E Premiere/Resolve XML.
* All hotkeys: Settings ▸ Hotkeys.

## Verify

```
cargo test
cargo run -- --selftest          # headless media/engine/export check (needs ffmpeg)
cargo run -- video.mp4 --screenshot shot.ppm
```

See `ARCHITECTURE.md` for the module map and contracts.
