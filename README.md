# Simple Editor

![Simple Editor](docs/images/screenshot.png)

A tiny, fast video trimmer/editor for Windows. Open a video, watch it, cut it, add music/SFX, overlay
images/video/text, keyframe things — and save over the original or export to any format.

Besides all of the general editing features it has: AI transcription, auto-subs, double-take
detection, dead-air removing, software is literally just 1 file and <15MB file size and launches in
<2ms, file-converter, YouTube/TikTok/X/URL video downloader and importer, Luau scripting/plugins,
tooling, and expressions, OpenGL + custom GLSL shader effects, motion-tracking, masking, shapes,
live-annotating and drawing, audio-mixing and filters with EQ, reverb, distortion, and more,
node-graphs, CapCut-style templates/containers, effect/transition presets, preset and custom themes
with preference for sharp and cozy style, full editor-layout customizability, planning and to-do
lists, auto-keyframing and easy velocity tools for extremely smooth animations and transitions,
style-reference sheet generator, MCP server for AI assistants to help you edit, importing/exporting
project files from/to DaVinci Resolve and Premiere Pro, and MORE for COMPLETELY FREE.

> **⚠️ Beta software.** Most features are experimental — nothing here is final or fully stable yet.
> It's usable for general-purpose editing today, but it is **not** intended for large-scale projects
> or anything that needs serious quality assurance. It's tested and updated daily on the way to being
> stable — expect rough edges, and keep backups of anything that matters.

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

* Windows 10/11 (x64). No installer — one `.exe`, run it from anywhere.
* [FFmpeg](https://ffmpeg.org) on PATH (or set its folder in Settings ▸ General) for export, images
  and fallback decoding: `winget install Gyan.FFmpeg` or `choco install ffmpeg`. Playback of common
  files works without it, but you'll want it for export.
* HEVC/AV1 playback through Media Foundation needs the Microsoft Store codec extensions; otherwise the
  ffmpeg fallback decoder is used automatically.

### Optional bonus features

These are off/hidden until you install their tool — the app never bundles or downloads them for you:

* **YouTube/TikTok/X/URL import** (Library ▸ Import URL…) needs [yt-dlp](https://github.com/yt-dlp/yt-dlp):
  `winget install yt-dlp.yt-dlp` or `choco install yt-dlp`, or drop `yt-dlp.exe` in the app's own folder.
  The *Import URL…* button only appears once it's found on PATH or in Settings ▸ General (yt-dlp folder).
* **AI transcription / auto-subs / double-take detection** need a [whisper.cpp](https://github.com/ggml-org/whisper.cpp)
  build (`whisper-cli.exe`) and a model file. Point Settings ▸ General (Whisper folder) at the binary, or
  drop it in `%APPDATA%\SimpleEditor\whisper`; the Subtitles pane's Transcribe section downloads a model
  for you the first time you use it.
* **Hardware export encoders** (NVENC/QSV/AMF) need the matching GPU driver installed; they show up in
  the encoder dropdown (Settings ▸ Export) automatically when ffmpeg reports them available.

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

## Playback

Playback is the specialty: the render thread keeps ~1.5 s of frames decoded ahead of the playhead
(and protects ~0.5 s behind it, so scrub-backs replay from RAM), pre-warming the next clip's decoder
before a cut. If decode still falls behind, playback **buffers** — the clock (and audio) pauses and a
spinner shows until the read-ahead refills — instead of playing audio over frozen video. Edits only
evict the frames they can actually change; everything else stays cached.

* **Proxy media** (Settings ▸ Performance): imported video gets a background low-res all-intra proxy
  (every frame a keyframe), so scrubbing and reverse playback are as cheap as forward. Exports always
  use the originals. Library ▸ right-click ▸ *Regenerate proxy* rebuilds one.
* **Fullscreen (F11)**: move the mouse for the transport bar (it hides ~2 s after the mouse stops;
  spacebar play/pause never summons it), double-click or Esc to leave.

## Icons

Menu actions, panes and panel buttons carry painted vector glyphs (film reel = import footage,
lightning = effects, …). Pick your own per action/pane in Settings ▸ Appearance ▸ Icons — any glyph,
"None" to remove, "Default" to restore.

## Scripting (Luau)

`Scripts` in the menu bar lists `.luau` files from `%APPDATA%\SimpleEditor\scripts` (an
`example.luau` is created on first open). Scripts run sandboxed inside the editor and drive the same
tool catalogue the MCP server exposes:

```lua
local s = editor.tool("project.summary", {})
editor.tool("timeline.add_clip", { asset = 1, at = 0 })
editor.log("done")          -- shows a toast
-- editor.tools() lists every tool with its description
```

One script run = one undo step; a failed tool call stops the script and rolls its edit back. A
runaway script is aborted after 5 s. MCP clients (e.g. Claude) and scripts share one API — an AI
co-editor can write a script into the folder and you run it from the menu.

## Path & expression links

Any keyframable property (Position, Scale, Rotation, Opacity, Volume, Pan, effect/mask params) can
be driven live instead of hand-keyed, After Effects–style: **follow a saved path**, or **drive it
with a Luau expression**. Both are *links* — they re-evaluate every frame from their source, so
editing the source (the path's points, or the expression text) updates every clip using it
immediately, with nothing to re-bake or re-apply.

### The link menu

Every property row in the Inspector has a small **∿** button next to its keyframe diamond:

| State | Look | Menu offers |
|---|---|---|
| Unlinked | ∿ (dim) | *Path: \<name\>* for each saved path (Position X/Y only), *Expression…* |
| Linked | **∿** (bold) | *Unlink*, plus the same path/expression choices to switch |

While a property is linked its slider/drag field is greyed out (still shows the live value) — drag
it or type into it and the link is **dropped first**, then the edit lands as a normal
keyframe/constant, same as breaking an expression's pickwhip in After Effects. There is no "linked
but also keyframed" state.

### Path links

A path is a saved drawing/polygon outline or motion-tracked point list (`Inspector ▸ Path ▸ Save`,
or the Tracking pane's *Save as path*) kept on the project so any clip can reuse it. Picking a path
from a property's ∿ menu on **Position X** or **Position Y** links that axis to the path's X or Y
over the clip's own duration, using the path's own recorded rhythm when it has one (a motion track)
and spreading points evenly when it doesn't (a polygon has no clock) — both ends are clamped, so the
clip holds the path's first point before its start and its last point after its end.

Because it's a link, not a bake: redraw or re-track the saved path afterwards and every clip
following it moves too. (Contrast with the Tracking pane's own *Apply to clip* button, which still
bakes a one-shot copy of a track directly onto a clip's X/Y keys — useful when you want to hand-tweak
the resulting keyframes afterward, which a live-linked property doesn't allow.)

### Expression links

Picking *Expression…* on any property opens a one-line Luau field syntax-highlighted for Luau
(keywords, strings, numbers, comments). It must be a full script body ending in `return <number>` —
no implicit "last expression" return, so you can use `local` variables or multi-line logic if you
need them:

```lua
return value + math.sin(t * 4) * 20
```

```lua
-- offset that eases out over the first second, then holds
local ramp = math.min(t, 1)
return value + 50 * (1 - (1 - ramp) ^ 3)
```

Two variables are in scope, filled in fresh for every sample:

| Variable | Meaning |
|---|---|
| `t` | Clip-local time in seconds (0 at the clip's start) |
| `value` | The property's own keyframed/constant value at `t` — the "no link" answer, so an expression can add to or override it |

The expression has the full Luau standard library available (`math`, `string`, etc. — same sandbox
as scripts: no `io`/`os`/`ffi`). It is **not** given `editor.tool(...)` — expressions only read `t`
and `value` and return one number; they cannot reach the rest of the project or the timeline. There
is no time budget on a single call (unlike the 5 s script budget) since it's expected to be cheap
per-sample math, but a script that hangs will hang the evaluation pass, so keep it to arithmetic
rather than loops over large data.

A broken expression (syntax error, runtime error, or a result that isn't a finite number) never
crashes the editor: the property falls back to its own value and a small `!` appears next to the
field — hover it to see the Luau error text.

### How it's evaluated (for scripting/MCP authors)

Internally a link is baked once per frame into a dense sample table (30 Hz for expressions; the
path's own point density for path links), and only re-baked when its inputs actually change — the
path's points, the expression text, or the property's own keys/value that an expression reads as
`value`. This keeps per-frame cost to a hash comparison for every linked property, decoupling the
render/mixer/UI code that reads `Animated::at(t)` from the link machinery entirely: nothing outside
of the bake step needs to know a property is linked. Links round-trip through the project JSON like
any other clip data, so a saved project reopens with its links intact and live.

## Verify

```
cargo test
cargo run -- --selftest          # headless media/engine/export check (needs ffmpeg)
cargo run -- video.mp4 --screenshot shot.ppm
```

See `ARCHITECTURE.md` for the module map and contracts, and `CHANGELOG.md` for release history.
