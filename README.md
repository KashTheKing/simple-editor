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
* **GPU effects** (Settings ▸ Performance): the preview runs the effect chain as OpenGL shaders through the
  window's existing GL context — blur, motion blur, VHS, chroma key, curves/levels/hue, edge glow, threshold,
  3D plane, JPEG crush, blob tracking, REC dot and your own GLSL shader. No GL context, an old driver or a
  shader the driver rejects → it says so once and falls back to the CPU compositor. Preview render quality
  (25–100 %) and **movie mode** (play back pre-rendered full-quality frames) live in the same place.
* **Tools, shapes and masks**: a tool strip under the viewport (select, text, rectangle / ellipse / triangle /
  polygon / star / line / arrow, freehand draw, mask, zoom), vector shape clips, adjustment layers that process
  everything below them, and masks on a clip or on a single effect.
* **Node editor**: a clip's effect stack as a graph (blend, matte, mask and clip inputs) when a straight stack
  is not enough; the Curves pane keyframes node parameters too.
* **Mixer**: audio buses with EQ, high/low-pass, reverb, echo, distortion, compressor, noise gate, noise and
  gain; per-track and per-clip routing, meters, mute/solo/mono.
* **Markers** on the timeline and on clips (M), with colour labels, notes and a "copy as list" for AI tools.
* **Copy / Paste Attributes** (Ctrl+Alt+C / Ctrl+Alt+V) with a checkbox per attribute group.
* **Export Frame** (Ctrl+Shift+F) as PNG / JPG / WebP, at project size, 2×, 4× or a custom size.
* **Screen recording and voiceover** through ffmpeg — including "record while the editor is in the
  background", which starts when the window loses focus and stops (and imports) when it comes back.
* **Import timelines** from Premiere / DaVinci Resolve (Final Cut Pro 7 XML), EDL and `.prproj`, with an
  honest per-item report of what did not survive the trip.
* **Library** + **Recent** panels (recent media across projects), drag & drop from Explorer.
* **Theme**: follows Windows dark/light + accent colour. Bare Windows-forms style, DaVinci-like layout.
* **Hotkeys**: every action rebindable in Settings.
* **Explorer context menu**: "Edit with Simple Editor" on video files (classic menu / "Show more options").
* **Project format**: plain JSON (`.sedit`) + export to Final Cut Pro 7 XML for Premiere Pro / DaVinci Resolve.
* **MCP server** for AI co-editing (Settings ▸ General): the whole timeline, effects, masks, nodes, markers,
  buses, shapes, frame export and timeline import as tools.

### What the audio effects are *not*

The mixer's filters are built in. Simple Editor has **no VST / VST3 / AU / CLAP support**: it does not load
plugin DLLs, it has no plugin scanner, no plugin editor windows, no plugin presets (`.fxp`/`.vstpreset`), and
it cannot open projects that reference third-party plugins. Bounce such tracks to audio elsewhere and import
the result.

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
  · I / O mark in/out · Ctrl+Shift+I trim to in/out · T add text · Ctrl+S save (overwrite the opened video or
  save the project) · Ctrl+E export as… · Ctrl+Alt+E fast lossless cut · Ctrl+Shift+E Premiere/Resolve XML.
* Round 3: Ctrl+T repeat the last transition · M marker · Shift+S shape · Ctrl+Alt+L adjustment layer ·
  Ctrl+Shift+M mask · Ctrl+Alt+C / Ctrl+Alt+V copy / paste attributes · Ctrl+Shift+F export frame ·
  Ctrl+8 / Ctrl+9 / Ctrl+0 markers / nodes / mixer.
* Layout: the Timeline (with Mixer and Auto-cut) sits across the bottom, the Tools strip under the viewport;
  drag tabs to rearrange, "🗖" pops a pane into its own window, View ▸ Layout ▸ Reset layout restores it.
* All hotkeys: Settings ▸ Hotkeys.

## Verify

```
cargo test
cargo run -- --selftest          # headless media/engine/export check (needs ffmpeg)
cargo run -- video.mp4 --screenshot shot.ppm
```

See `ARCHITECTURE.md` for the module map and contracts.
