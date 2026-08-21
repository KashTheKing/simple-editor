# Simple Editor — architecture & module contracts

A tiny, fast video trimmer/editor for Windows. Rust + egui. Single binary (< 50 MB, ~7 MB).

Priorities, in order: **performance (startup, seek, playback, export), low memory/CPU/GPU, frictionless
open → cut → save, then everything else.** Bare Windows-Forms-style UI matching the OS theme; DaVinci-like
layout (dockable panes) without styling. Popups never block the editor (plain `egui::Window`s, no modals).

## Stack

| Concern | Choice | Why |
|---|---|---|
| UI | `eframe`/`egui` 0.33 (glow backend) + `egui_tiles` 0.14 (docking) | immediate mode, custom-painted timeline trivial, repaints only on input/playback |
| Decode | Windows **Media Foundation** (`windows` crate) — `media/mf.rs` | native system codecs, no extra DLLs, instant seeks (software decode; D3D manager is the upgrade path) |
| Decode fallback / images / probe / export / convert | `ffmpeg.exe` + `ffprobe.exe` child processes — `media/ffpipe.rs` | universal codecs, any output container; already on most machines |
| Audio out | `cpal` (WASAPI) | small pure-Rust |
| Text | `fontdb` (system + imported fonts) + `ab_glyph` (raster) | small; outline = distance transform, shadow = offset+blur |
| Settings / project | `serde_json` | one settings file, `.sedit` project JSON |
| Registry (theme/accent/context menu) | `winreg` | tiny |
| Dialogs | `rfd` | native Windows dialogs |
| MCP server | hand-rolled HTTP/1.1 + JSON-RPC on std `TcpListener` — `mcp/` | no deps; lets AI agents co-edit live |

Frames are **top-down RGBA8** everywhere (`media::Frame`). Audio is **interleaved stereo f32 @ 48 kHz**.

## Module map (file = owner; keep files disjoint)

```
src/main.rs            entry: args (--selftest, --screenshot), eframe run
src/model.rs           Project/Sequence/Track/Clip/Asset/Animated(+Ease incl. Bezier)/Effect/Transition/Cue/PlanItem
                       + all edit ops (split, ripple, move, trim, speed, nest, auto_cut, place_clips, flow) + JSON
src/settings.rs        Settings (JSON in %APPDATA%\SimpleEditor), recents (tags/labels/pins), layout profiles,
                       curve/motion presets, templates, user fonts, export scaler/res, MCP toggle/port
src/hotkeys.rs         Action enum, defaults, parse/format, poll(ctx)
src/theme.rs           system dark/light + accent → egui visuals, Segoe UI; Palette for custom painting
src/contextmenu.rs     HKCU "Edit with Simple Editor" per video/gif extension
src/media/mod.rs       Frame, VideoSource/AudioSource traits, Backend, probe/open dispatch, DecoderPool
src/media/mf.rs        Media Foundation decoders + probe
src/media/ffpipe.rs    ffmpeg/ffprobe process decoders + probe + exe lookup
src/media/waveform.rs  peaks cache (background compute, disk cache)
src/media/thumbs.rs    thumbnail cache (timeline filmstrips, library previews)       (agent: engine-misc)
src/engine/blend.rs    blend modes + composite
src/engine/compose.rs  Compositor: timeline → RGBA canvas; placement(); effects, transitions,
                       nested sequences, subtitles, scaler modes, camera-shake homography       (agent: engine-video)
src/engine/effects.rs  per-clip pixel effects + wobble()                                       (agent: engine-video)
src/engine/text.rs     TextRasterizer (system + user fonts)                                    (agent: engine-video)
src/engine/mixer.rs    Mixer: speed/reverse/freeze, pan, fades, transitions, sequences         (agent: engine-audio)
src/engine/export.rs   ffmpeg export (pipe), out-size + scaler, lossless cut, encoders        (agent: engine-audio)
src/engine/subtitles.rs SRT/VTT parse + write                                                 (agent: engine-audio)
src/engine/autocut.rs  silence/speech segmentation from waveform peaks                         (agent: engine-audio)
src/engine/convert.rs  "Convert To…" transcodes (gif↔video, containers, audio, rescale)        (agent: engine-audio)
src/engine/presets.rs  curve/motion presets + clip templates capture/apply                     (agent: engine-misc)
src/engine/style.rs    Markdown style summary of a project (for AI style guides)              (agent: engine-misc)
src/engine/xmeml.rs    FCP7 XML for Premiere/Resolve
src/playback.rs        Player: render thread + audio thread + clock (+ render_once for tools)  (agent: app-layout)
src/mcp/mod.rs         MCP server (HTTP + JSON-RPC), ToolCall channel, png_encode             (agent: mcp)
src/mcp/tools.rs       the tool catalogue (names/args) — shared truth for server + app
src/ui/layout.rs       egui_tiles layout, popouts (immediate viewports), profiles             (agent: app-layout)
src/ui/app.rs          App: panes, menus, actions, undo, file ops, export/convert flows, sequence
                       breadcrumb, fullscreen, MCP tool execution, non-blocking windows          (agent: app-layout)
src/ui/timeline.rs     timeline widget                                                         (agent: ui-timeline)
src/ui/preview.rs      preview + transport (+ fullscreen mode)
src/ui/inspector.rs    everything on the selected clip / asset                                 (agent: ui-panels-lib)
src/ui/effects_ui.rs   effects catalogue + stack editor                                        (agent: ui-panels-fx)
src/ui/transitions_ui.rs transitions catalogue + editor                                         (agent: ui-panels-fx)
src/ui/curves.rs       keyframe graph editor (bezier handles, presets, flow)                  (agent: ui-panels-fx)
src/ui/autocut_ui.rs   auto-cut pane                                                           (agent: ui-panels-fx)
src/ui/library.rs      library/recent: folders, tags, labels, descriptions, search, filters,
                       linked folders, sequences, templates, used/unused, convert                (agent: ui-panels-lib)
src/ui/subtitles_ui.rs subtitle editor                                                         (agent: ui-panels-lib)
src/ui/retime.rs       speed/reverse/freeze window                                             (agent: ui-panels-lib)
src/ui/planner.rs      planner (nested tasks, moodboards) + notes                              (agent: ui-panels-lib)
src/ui/export_ui.rs    export window (resolution, scaler, encoder, lossless)                  (agent: ui-panels-lib)
src/ui/settings_ui.rs  settings window incl. hotkey editor, MCP, fonts                         (agent: app-layout)
src/selftest.rs        headless end-to-end check
```

Every public signature in the stub files is the contract. **Do not change a signature in a file you
don't own**; if you truly need something, add a private helper in your own file or report it.

## Key behaviours

* **Open a video** → `Project::from_media(asset)`: project size/fps from the video, V1 with the clip,
  one audio track per audio stream (A1, A2, …), all linked. `source_video` set → **Save (Ctrl+S) =
  re-encode over the original** (temp file in the same folder, `Player::release_files()`, rename) — or the
  instant `-c copy` cut when `Settings.lossless_save` is on and `export::lossless_segments` applies.
* **Export** → Export window (non-blocking): resolution + scaler (ffmpeg `scale` flags) + encoder; any
  extension; Compositor frames piped to ffmpeg stdin, audio pre-mixed to a temp WAV. **Convert To…**
  (library) transcodes files directly (engine/convert.rs).
* **Preview == export**: both use `Compositor::render` and `Mixer::mix`, only the canvas size differs.
  `Project.scaler` selects the compositor's resampling (nearest / bilinear / bicubic).
* **Undo** = JSON snapshots (`push_undo()` before each mutation; panels receive an `undo` closure and call
  it with the pre-gesture project, once per gesture, only if something changed).
* **Keyframes**: `Animated { value, keys }`, clip-local time; `Keyframe.ease` (Linear/EaseIn/Out/InOut/Hold/
  Bezier handles). Inspector/Effects "◆" toggles a key at the playhead; editing an animated property writes
  a key at the playhead (`set_at`). Diamonds on clips are draggable; the Curves pane edits values, times,
  bezier velocity handles, applies curve/motion presets (scaled or exact) and "flows" two clips
  (`Project::flow_clips`).
* **Retime**: `Clip.speed/reverse/freeze`; `Clip::src_time` maps timeline→source; durations follow the
  source window (`Project::set_speed`); freeze = still frame, silent audio. Ctrl+R window.
* **Effects**: `Clip.effects` stack (`EffectKind` + keyframeable params), applied on the decoded layer
  before placement; `Wobble` moves the placement (x/y/roll/yaw/pitch — yaw/pitch via a homography).
* **Transitions**: `Track.transitions` centred on a cut (`Transition::window`); both clips extend virtually
  into the window; audio tracks get gain crossfades; pruned by `Project::tidy()` when cuts disappear.
* **Sequences** (nested timelines): `Project.sequences`; a `ClipKind::Sequence` clip renders/mixes its
  sequence recursively (`Project::sequence_tracks`); editing = `open_sequence` swaps its tracks into
  `Project.tracks` (main goes to `main_stash`), `close_sequence` swaps back; cycles are refused.
* **Subtitles**: `Project.subtitles` (+ style/margin/burn-in), SRT/VTT import/export, rendered bottom-centre.
* **Library**: folders/tags/labels/descriptions, search + filters (kind, Short SFX ≤ 10 s, Music,
  label, used/unused), linked folders browsed from disk, sequences and templates as drag sources, thumbnails.
* **Planner**: nested checkable tasks with notes, colour, moodboards (asset refs — never "unused");
  `Project.notes` free text. Both feed the **style summary** (`engine/style.rs`).
* **Auto-cut**: `engine::autocut` finds loud/quiet segments from waveform peaks; `Project::auto_cut`
  splits (+ linked video) and removes; the pane shades the kept ranges on the timeline while you keep editing.
* **MCP**: when enabled, `mcp::Server` serves `mcp/tools.rs` over HTTP; the App executes `ToolCall`s on the
  UI thread each frame (undo per call) and replies; `render.frame` uses `Player::render_once`.
* **Layout**: `ui/layout.rs` egui_tiles tree; panes pop out into OS windows; profiles saved in settings and
  exportable as `.sedit-layout` JSON.
* **Mute/Solo** per audio track; video tracks: V (visibility) / S. **Linked clips** select/move/split together.
* **Theme**: `theme::apply` at start and when settings change; custom painting uses `theme::palette(ctx)`.
* **Context menu** installed on first run (release builds) if `settings.context_menu` (HKCU, no admin).

## Threading

* UI thread: egui. Never blocks on media. MCP tool calls and convert/export jobs are polled per frame.
* `Player` render thread: owns a `DecoderPool` + `Compositor`; receives commands (mpsc); publishes
  `Arc<Frame>`; calls `ctx.request_repaint()`.
* `Player` audio thread: owns its own `DecoderPool` + `Mixer`; fills a ring buffer consumed by the cpal
  callback; wall clock is the master; ~120 ms lead.
* Export/convert threads: their own pool/compositor/mixer or ffmpeg child. Waveform + thumbnail workers:
  their own decoders. MCP server thread: sockets only; all project access happens on the UI thread.
* MF objects are created and used on one thread each (`unsafe impl Send` newtypes, COM/MF init per thread).

## Conventions

* Rust 2021, `cargo fmt` (rustfmt.toml, 120 cols), warning-free `cargo build`.
* No new dependencies without a strong reason — check the ladder: std → already-installed crate → code.
* Mark deliberate shortcuts with `// ponytail: <what> — <upgrade path>` comments.
* Any non-trivial logic leaves a test (`#[cfg(test)]`) or is covered by `--selftest`.
* Run `cargo test` + `cargo run -- --selftest` before declaring done.

## Verification

* `cargo test` — model/hotkey/engine/ui unit tests (headless `egui::Context::run` for widgets).
* `cargo run -- --selftest` — headless media + engine + export check against ffmpeg-generated media.
* `cargo run -- <video> --screenshot out.ppm` — renders the UI and saves a screenshot (convert with
  `ffmpeg -i out.ppm out.png`).
