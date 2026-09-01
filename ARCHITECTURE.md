# Simple Editor — architecture & module contracts

A fast video editor for Windows. Rust + egui, single binary (~11 MB release), OpenGL render path.
**Read this before changing anything.** It is the contract between modules and the map for future agents.

Priorities, in order: **performance (startup, seek, playback, export), low memory/CPU/GPU, frictionless
open → cut → save, then everything else.** Bare Windows-Forms-style UI matching the OS theme; DaVinci-like
dockable layout without decoration. Popups never block the editor (plain `egui::Window`, never `Modal`).

Measured baselines (release, RTX 2060 SUPER): exe 15.06 MB (over the ~10 MB goal since
egui_commonmark; tracked in goals.md) · first frame ~330 ms · idle CPU 0.0 % · ~125 MB working set
with a clip open BEFORE playback caches fill — the playback cache may then grow to its budget
(`Settings::cache_mb`, auto = ¼ of RAM clamped 512 MB–4 GB, plus ¼ of that for decoded source
frames; deliberate, user-tunable in Settings ▸ Performance) · 655 unit tests + a headless
`--selftest`.

## Stack

| Concern | Choice | Why |
|---|---|---|
| UI | `eframe`/`egui` 0.33 (glow) + `egui_tiles` 0.14 | immediate mode; custom-painted timeline; repaints only on input/playback |
| Preview & export rendering | **OpenGL** through eframe's glow context — `engine/gpu.rs` | one shader set for preview *and* export |
| Decode | Windows **Media Foundation** (`windows` 0.62) — `media/mf.rs` | native codecs, no DLLs to ship, fast seeks |
| Decode fallback / images / probe / export / convert / capture | `ffmpeg.exe` + `ffprobe.exe` | universal codecs and containers |
| Audio out | `cpal` (WASAPI) | small, pure Rust |
| Text | `fontdb` + `ab_glyph` | system + user-imported fonts |
| Project / settings | `serde_json` | `.sedit` project, one settings file |
| Registry (theme, accent, context menu) | `winreg` | tiny |
| Dialogs | `rfd` | native Windows dialogs |
| MCP server | hand-rolled HTTP/1.1 + JSON-RPC on std `TcpListener` — `mcp/` | no deps; AI agents co-edit live |
| URL import (optional) | `yt-dlp.exe` if installed — `media/ytdlp.rs` | button only appears when found |

Frames are **top-down RGBA8, straight alpha** everywhere (`media::Frame`). Audio is **interleaved stereo
f32 @ 48 kHz**. No new dependencies without a strong reason.

## Module map (file = owner; keep files disjoint)

```
src/main.rs            args (--selftest, --screenshot, a file to open), eframe run
src/winpos.rs          opens the window on the monitor under the cursor (eframe's saved rect always won)
src/model.rs           THE data model + every edit operation + .sedit JSON (see "Data model" below)
src/settings.rs        %APPDATA%\SimpleEditor\settings.json: recents, layout profiles, curve/motion
                       presets, effect/node-graph presets, templates, user fonts,
                       GPU/quality/capture/frame/MCP options
src/hotkeys.rs         Action enum, default bindings, parse/format, per-frame polling
src/theme.rs           OS dark/light + accent -> egui visuals, Segoe UI; Palette for custom painting
src/contextmenu.rs     HKCU "Edit with Simple Editor" per video extension
src/media/mod.rs       Frame, VideoSource/AudioSource traits, Backend, probe/open dispatch, DecoderPool
src/media/mf.rs        Media Foundation decoders + probe
src/media/ffpipe.rs    ffmpeg/ffprobe child-process decoders + probe + exe lookup
src/media/waveform.rs  audio peaks cache (background compute, disk cache)
src/media/thumbs.rs    thumbnail cache (timeline filmstrips, library rows)
src/media/proxy.rs     background all-intra proxy transcodes (preview plays these, exports never do)
src/media/ytdlp.rs     optional URL download
src/scripting.rs       embedded Luau (mlua): sandboxed `editor.tool(...)` bridge into the MCP tool
                       catalogue; scripts folder listing; 5 s VM budget
src/engine/gpu.rs      GPU renderer: FBO pool, programs, node-graph eval, transitions, masks, readback
src/engine/shaders.rs  all GLSL: VERT/PRELUDE/BLEND/MASK/COMPOSITE + one body per EffectKind
src/engine/compose.rs  CPU compositor (fallback + "source frame only"): placement, transitions, masks
src/engine/effects.rs  CPU effect implementations, wobble()/track() helpers, gpu_only()
src/engine/shapes.rs   shape + recorded-drawing rasteriser
src/engine/text.rs     TextRasterizer (system + imported fonts)
src/engine/blend.rs    blend modes (CPU), mirrored in shaders.rs BLEND
src/engine/mixer.rs    audio mixer: speed/reverse/freeze, pan, fades, transitions, sequences, buses
src/engine/mixer_fx.rs filter DSP (EQ, reverb, echo, distortion, compressor, gate, noise, gain) + BusGraph
src/engine/export.rs   ffmpeg export (pipe), FrameSource::{Cpu,Gpu}, lossless cut, encoder detection
src/engine/convert.rs  "Convert To…" transcodes (gif<->video, containers, audio, rescale)
src/engine/capture.rs  screen recording + voiceover through ffmpeg (gdigrab / dshow)
src/engine/import.rs   FCP7 XML / EDL / .prproj import with a per-item report; async asset probing
src/engine/subtitles.rs SRT/VTT parse + write
src/engine/autocut.rs  silence/speech segmentation from waveform peaks
src/engine/presets.rs  curve/motion presets, effect-chain / node-graph presets, clip templates
src/engine/prerender.rs "movie mode" full-quality frame cache on disk, rendered on a worker thread
src/engine/style.rs    Markdown style summary of a project (for AI style guides)
src/engine/tracking.rs point / area tracking: NCC template match on a worker thread -> a reusable path
src/engine/transcribe.rs speech->text through a downloaded whisper.cpp model + exe, auto-subs, double takes
src/engine/xmeml.rs    FCP7 XML export for Premiere / Resolve
src/playback.rs        Player: render thread + audio thread + wall clock; decode_layers() for the GPU
src/mcp/mod.rs         MCP server (HTTP + JSON-RPC), ToolCall channel, png_encode
src/mcp/tools.rs       tool catalogue — the shared truth for server and app
src/ui/layout.rs       egui_tiles docking, pop-out viewports, layout profiles
src/ui/app.rs          App: panes, menus, actions, undo, file ops, GPU wiring, MCP execution, windows
src/ui/timeline.rs     the timeline widget (everything drawn and dragged on it)
src/ui/preview.rs      viewport + transport + tool interactions
src/ui/tools.rs        tool strip with painter-drawn icons
src/ui/inspector.rs    every property of the selected clip / asset / project
src/ui/effects_ui.rs   effect catalogue (thumbnail grid) + the clip's effect stack
src/ui/nodes.rs        node-graph editor
src/ui/curves.rs       keyframe graph editor (bezier velocity, presets, flow)
src/ui/library.rs      library + recent: folders, tags, labels, search, filters, linked folders
src/ui/mixer_ui.rs     bus strips, meters, filter chains
src/ui/markers_ui.rs   marker list (collapsible rows, icons, per-sequence scoping, bulk ops)
src/ui/planner.rs      Plan / Notes / Timer tabs: task tree, markdown notes, stopwatch dial
src/ui/moodboard_ui.rs standalone moodboard pane (gallery/list/slideshow, tags, add-at-playhead)
src/ui/history_ui.rs   History pane over the undo stack (day groups, search, filters, md export)
src/ui/guides.rs       social-platform guide overlays + aspect/format presets
src/ui/heartbeat.rs    repaint scheduling helper
src/ui/presets_ui.rs   Presets pane: effects / node graphs / adjustment layers / templates (machine-local)
src/ui/subtitles_ui.rs subtitle editor
src/ui/autocut_ui.rs   auto-cut pane
src/ui/tracking_ui.rs  Tracking pane: place a box, track a clip, save the result as a path
src/ui/{retime,export_ui,frame_ui,capture_ui,import_ui,paste_ui,transitions_ui,settings_ui,shader_ui}.rs windows
src/selftest.rs        headless end-to-end check (`--selftest`)
```

## Data model (`src/model.rs`)

`Project` owns everything: `assets`, `folders`/`linked_folders`, `tracks` (video first, then audio),
`sequences` (nested timelines) with `editing`/`main_stash` for the one being edited, `subtitles`,
`markers`, `labels`, `buses`, `plan`, `notes`, `paths` (saved outlines), in/out points, `scaler`, and a
monotonic `next_id`.

* `Clip` — `kind` (Video/Image/Text/Audio/Sequence/Shape/Adjustment), timing (`start`, `duration`,
  `src_in`), retime (`speed` + the keyframable `speed_curve` ramp, `reverse`, `freeze`),
  transform/`opacity`/`blend`, `effects` (linear stack)
  or `graph` (node DAG), `mask`, audio (`volume`, `pan`, `fade_in/out`, `bus`), `text`, `shape`,
  `markers`, `label`, `link` (clips that move/split/delete together).
* `Animated { value, keys }` — every keyframeable property. Keys carry an `Ease`
  (Linear/EaseIn/Out/InOut/Hold/**Bezier** handles). Times are clip-local seconds.
* `Effect { kind, params: Vec<Animated>, mask, shader, start, len }` — `EffectKind::params()` is the
  parameter table (name/default/min/max); `is_bool_param()` marks checkbox params; `is_geometric()` marks
  effects folded into the placement; `needs_motion()` marks ones needing neighbouring frames. `start`/`len`
  are the clip-local window it runs in (`Effect::on_at`; `len <= 0` = to the end of the clip, i.e. every
  project written before the window existed).
* `NodeGraph { nodes, edges }` — `NodeKind::{Input, Color, Clip, Asset, Effect, Blend, Combine, Merge,
  Matte, Mask, String, Number, Bool, Random, Math, Compare, Logic, Select, Output}` (`String` was called
  `Text`; a serde alias keeps old projects loading), cycle-refusing `connect`, `eval_order()`,
  `eval_values()` — the scalar half of the graph, which drives effect parameter ports — and
  `to_effects()` (the inverse of `from_effects`, used by `Project::unlink_graph`). The node editor is
  **opt-in**: only a graph with real nodes in it (`Clip::uses_graph`) replaces the linear stack for
  rendering — a bare Input→Output pass-through never shadows `effects`, and the Nodes pane builds one
  only when asked.
* `Mask` — Rect/Ellipse/Polygon/Path with animated centre/radii/rotation/feather/expand/opacity/invert.
* `Transition` — lives on a `Track`, centred on a cut, **window clamped to its two clips**
  (`Transition::window(left, right)`); kinds CrossFade / FadeToColor / Push / Wipe.
* `Bus` / `AudioFilter` / `FilterKind` — the mixer graph; `Project::bus_of(track, clip)` resolves routing.
* `AttrSet` — which attributes Copy/Paste Attributes transfers.

Everything is `serde`-serialised; `Project::from_json` sanitises hand-edited files (fps, sizes, NaNs,
dangling ids) and `save()` writes a temp file then renames.

## Rendering

**One pipeline, two front ends.** Effects are GLSL (`engine/shaders.rs`); `engine/gpu.rs` compiles them
lazily, keeps a size-keyed FBO pool, and evaluates either the linear stack or the node graph per clip.

* **Preview** — the player decodes layers on its own thread (`playback::decode_layers`: one bitmap per
  visual clip, including rasterised text/shapes, CPU-composited nested sequences and the subtitle
  bitmap, plus both clips of a transition), the UI thread composites them and egui paints the renderer's
  own texture (**zero copy** — no readback, no re-upload).
* **Export** — the export thread decodes off-thread and posts `GpuFrameRequest`s; the UI thread renders
  them on the GL context (`FrameSource::Gpu`), so *exports run the same shaders as the preview*. If the
  GPU is off or stops answering within 5 s it finishes on the CPU compositor and the progress line names
  what was dropped (`export::cpu_gaps`).
* **CPU compositor** (`engine/compose.rs`) — fallback, "source frame only", and machines without GL. It
  implements placement, transitions, masks, shapes and adjustment layers, but not node graphs or the
  GPU-only effect kinds (`effects::gpu_only`).
* `Project.scaler` picks nearest / bilinear / bicubic sampling; `Settings.preview_quality` scales the
  preview render size; **movie mode** plays back `engine::prerender` frames.

GL work happens **only on the UI thread** (it owns the context). Everything GPU degrades to `Option`/
`Result` — a missing context or a rejected shader falls back and says so once.

### Playback smoothness (`src/playback.rs`)

* **Read-ahead**: while playing, the render thread pre-renders ~1.5 s of frames past the playhead into
  a byte-budgeted LRU cache and eviction-protects a ~0.5 s trail behind it (instant scrub-backs). The
  budget is `Settings::cache_mb` (0 = automatic: a quarter of installed RAM, clamped 512 MB-4 GB —
  `playback::cache_budget_bytes`); the horizon is sized from the cache's measured average entry cost,
  so multi-layer GPU frames can't oversubscribe it. Prefetch through the compositor also pre-opens the
  next clip's decoder before a cut.
* **Buffering**: if the clock runs >150 ms past the newest published frame and the due frame is not
  cached, the thread sets `Shared.buffering`, stops live-rendering and free-runs the prefetcher. The
  app polls `Player::is_buffering()`, pauses the clock (flushing audio with it — audio never plays
  over frozen video) and shows a spinner; hysteresis (⅓ of the horizon cached) resumes it.
* **Selective invalidation**: `video_dirty_spans(old, new)` diffs projects on `SetProject` and evicts
  only the frame ranges an edit can change (clips by id, JSON equality). A finished proxy
  (`Cmd::Proxies`) likewise evicts only `spans_using_source` for the remapped files instead of the
  whole cache. Audio/marker/planner edits evict nothing; canvas/fps/asset/sequence changes still
  clear everything.
* **DecoderPool LRU**: at most 16 live video / 32 audio decoders per pool; least-recently-used are
  dropped so long timelines don't hoard MF readers / ffmpeg children. The preview pool also keeps a
  decoded-source-frame LRU (`DecoderPool::frame_at`, a quarter of the cache budget; 0 = off for every
  other pool) so a scrub-back past the composited cache is a memcpy, not an ffmpeg respawn.
* **Proxies** (`src/media/proxy.rs`): the app builds all-intra 720p proxies (ffmpeg, one background
  job at a time, hash-of-(path, mtime, height) filenames in the cache dir) and pushes a
  source→proxy map to the player (`Cmd::Proxies` → `DecoderPool::set_proxies`). Only preview pools
  carry the map; export and one-shot renders always read originals. Per-asset status
  (`proxy::status`: queued / building N% / ready) shows on library rows, the inspector's Asset block
  and the preview badge.

## Threading

* UI thread: egui, all GL, MCP tool execution, export frame service, pre-render slices.
* `Player` render thread: decoders + (CPU) compositor or layer decoding; publishes `Arc<Frame>` / `LayerSet`.
* `Player` audio thread: decoders + `Mixer` + `BusGraph` → ring buffer → cpal. Wall clock is the master.
* Export / convert / capture threads: their own decoders or an ffmpeg child.
* Waveform + thumbnail workers: one background thread each, LIFO, memoised failures.
* Every decode on a long-lived worker runs under `catch_unwind`; MF objects are per-thread
  (`unsafe impl Send` newtypes, COM/MF init per thread).

## Feature inventory (implemented and tested)

**Editing** — open any video/gif/image/audio; trim, split (Ctrl+B), ripple delete, drag/move, snapping,
multi-track video+audio, linked clips, rubber-band multi-select, drag above/below the tracks to create
one, in/out points, nudge, undo/redo (JSON snapshots, 200 deep).
**Retime** — speed, reverse, freeze frame (Ctrl+R).
**Effects** — 24 kinds: Blur, Motion Blur, Pixelate, JPEG Compression, VHS, Chroma Key, Threshold, Edge
Glow, Color Tint, Color Correction, Curves, Levels, Hue/Saturation, B&W, Invert, Vignette, Sharpen, Flip,
Crop, 3D Plane, Camera Shake (motion waveform + smoothness), Blob Tracking, Security-camera REC, and a
user GLSL Shader (edited in a non-blocking window that compiles the source and prints the driver's log).
Each has keyframeable params and an optional mask.
**Transitions** — cross fade, fade to colour, push, wipe; audio crossfades mirror them.
**Masks** — per clip and per effect; rect/ellipse/polygon/path with feather, expand, invert.
**Node editor** — chain/combine effects with blend/combine/merge, matte, mask, colour, clip, asset and
text (`{frame}`/`{time}`/`{n}` counters and clocks) inputs; "Unlink" turns a simple chain back into a
plain effect list.
**Keyframes** — value-positioned diamonds on clips, a graph editor with bezier velocity handles, easing
menus, curve/motion presets (scaled or exact), and "flow" between two clips.
**Text** — system + imported fonts, size, bold/italic, fill/outline/shadow/box, alignment, spacing.
**Shapes & drawing** — rect/ellipse/triangle/polygon/star/line/arrow plus recorded freehand drawings;
the Polygon tool places real points (click to add, Enter / click a point to close, drag them when the
shape is selected) into `ShapeStyle.points` — empty keeps the regular n-gon. The Draw tool's play button
records while the video plays (every stroke of the take joins one drawing clip, until the video stops or
the tool is put away), and a drawing or polygon outline can be saved to `Project.paths` and reused as a
motion path: `Project::apply_path` keyframes a clip's X/Y along it, and a mask node's centre can follow it.
**Tracking** — the Tracking pane follows a point or a box through a clip (normalised cross-correlation
template matching, template refreshed every N frames, on a worker thread with a progress bar); the result
saves into `Project.paths` and can be applied straight to the clip's X/Y keyframes.
**Adjustment layers** — effects that apply to everything below them.
**Sequences** — nested timelines usable as footage.
**Audio** — waveforms, per-clip volume line + fades, pan, mute/solo, buses with EQ/reverb/echo/
distortion/compressor/gate/noise/gain, per-track and per-clip routing, meters, mono fold, auto-cut by
silence, voiceover recording.
**Subtitles** — editor, SRT/VTT import/export, burnt-in rendering.
**Markers** — timeline and clip markers with labels and notes (also exposed over MCP).
**Library** — folders, tags, custom labels, descriptions, search, filters (kind, Short SFX ≤ 10 s, Music,
label, unused), linked folders browsed from disk, thumbnails, sequences, templates, Convert To…,
Remove unused, optional URL import.
**Planner** — nested tasks with moodboards, plus free-form project notes.
**Capture** — screen recording (region, bitrate, mic, desktop audio, record-while-unfocused) and voiceover.
**Export** — any container/codec by extension, resolution + scaler, hardware encoders, fast lossless cut
(`-c copy`), export frame (PNG/JPG/WebP), Premiere/Resolve XML, style summary Markdown.
**Import** — Premiere/Resolve FCP7 XML, EDL, `.prproj` with a per-item report.
**Layout** — dockable panes, pop-out windows, saved/shared profiles.
**AI** — MCP server (toggle in Settings) exposing ~50 tools over `http://127.0.0.1:<port>/mcp`.
Luau scripts (`src/scripting.rs`, Scripts menu) call the identical tool catalogue in-process — one
API surface for scripts and MCP clients; each script run is a single undo step.

**Icons** — all UI icons are runtime-painted `Glyph` variants (`ui/tools.rs`; fonts lack the symbol
chars). `tools::action_glyph` / `Pane::glyph` give defaults; `Settings.icon_overrides`
("action.<id>" / "pane.<title>" → glyph name or "none", edited in Settings ▸ Appearance ▸ Icons)
overrides them.

## Conventions

* Rust 2021, `cargo fmt` (rustfmt.toml, 120 cols), warning-free `cargo build`.
* No unwrap() on external data; reuse buffers in hot paths; no busy loops.
* Mark deliberate shortcuts `// ponytail: <what> — <upgrade path>`.
* Non-trivial logic leaves a `#[cfg(test)]` test or is covered by `--selftest`; widgets are tested with
  headless `egui::Context::run` harnesses (see `ui/timeline.rs`, `ui/library.rs`).
* Panels take an `undo` closure and call it **once per gesture, before mutating**, only if something
  changed. Panels that cannot reach `Settings` hand values back through documented thread-locals.

## Verification (run all three before declaring done)

```
cargo test                                  # unit + headless widget tests
cargo run -- --selftest                     # real media through decoders, engine, export
cargo run -- <video> --screenshot out.ppm   # renders the UI; SE_SCREENSHOT_DELAY=<s> to warm caches
```
Convert the screenshot with `ffmpeg -i out.ppm out.png` and actually look at it.
