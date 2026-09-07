# Simple Editor — architecture & module contracts

A fast video editor for Windows. Rust + egui, single binary (~11.6 MB release), OpenGL render path.
**Read this before changing anything.** It is the contract between modules and the map for future agents.

Priorities, in order: **performance (startup, seek, playback, export), low memory/CPU/GPU, frictionless
open → cut → save, then everything else.** Bare Windows-Forms-style UI matching the OS theme; DaVinci-like
dockable layout without decoration. Popups never block the editor (plain `egui::Window`, never `Modal`).

Measured baselines (release, RTX 2060 SUPER): exe **12,167,168 B (11.60 MB)** at `86c1793`
(wave-3-complete — see `size_log.csv`'s last row; still over the ~10 MB core goal, tracked in
goals.md) · first frame ~330 ms · idle CPU 0.0 % · ~125 MB working set with a clip open BEFORE
playback caches fill — the playback cache may then grow to its budget (`Settings::cache_mb`, auto =
¼ of RAM clamped 512 MB–4 GB, plus ¼ of that for decoded source frames; deliberate, user-tunable in
Settings ▸ Performance) · **1126 unit/integration tests + a headless `--selftest`** (`cargo test`,
this PR's own rerun: 1126 passed, 2 ignored, 0 failed).

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
| Scripting | `mlua` (Luau) — `src/scripting.rs` | sandboxed, embeddable, the one deliberately-approved extra dependency (goals.md) |
| MCP server | hand-rolled HTTP/1.1 + JSON-RPC on std `TcpListener` — `mcp/` | no deps; AI agents co-edit live |
| URL import (optional) | `yt-dlp.exe` if installed — `media/ytdlp.rs` | button only appears when found |

Frames are **top-down RGBA8, straight alpha** everywhere (`media::Frame`). Audio is **interleaved stereo
f32 @ 48 kHz**. No new dependencies without a strong reason (see goals.md's dependency budget — this
wave added zero).

## Module map (file = owner; keep files disjoint)

Rebuilt from `ls -R src/ui/app src/model src/ui/timeline` at merge time (wave-3-complete, `86c1793`);
every other directory checked too. Grouped by the god-file split (issue #16) that created them.

```
src/main.rs            args (--selftest, --screenshot, a file to open), eframe run
src/winpos.rs          opens the window on the monitor under the cursor (eframe's saved rect always won)
src/keymaps.rs          named chord presets (Avid/Premiere/Resolve) as diff tables over the defaults
src/settings.rs        %APPDATA%\SimpleEditor\settings.json: recents, layout profiles, curve/motion
                        presets, effect/node-graph presets, templates, user fonts, layout_mode,
                        inspector_folds, export_presets, autosave_secs, beat_thr, GPU/quality/
                        capture/frame/MCP options
src/hotkeys.rs          Action enum, default bindings (actions! macro, one marker section per
                        workstream), parse/format, per-frame polling
src/theme.rs            OS dark/light + accent -> egui visuals, Segoe UI/Consolas from %WINDIR%\Fonts;
                        Palette for custom painting; test_fonts() for headless widget tests
src/contextmenu.rs      HKCU "Edit with Simple Editor" per video extension
src/selftest.rs         headless end-to-end check (`--selftest`), incl. an idle_repaint step

src/model/mod.rs        THE data model + .sedit JSON — Project + core types (see "Data model" below)
src/model/{asset,audio,clip,effect,graph,io,marker,path,project,shape,
           subtitle,text,track,transition}.rs   one data type each, re-exported from model/mod.rs
src/model/ops/mod.rs    re-exports every ops/*.rs file below as one `impl Project` surface
src/model/ops/{assets,attrs,autocut,buses,editing,effects,graph,markers,multicam,paths,
           planner,queries,sequences,shapes,subtitles,templates,tracks,transitions,trim}.rs
                        every edit operation on Project, grouped by concern;
                        `trim.rs`/`multicam.rs` are wave-1/3 additions (ripple/roll/slip/slide primitives,
                        multicam angle-switch) alongside the original wave-0a split
src/model/tests.rs      model-level tests

src/media/mod.rs        Frame, VideoSource/AudioSource traits, Backend, probe/open dispatch, DecoderPool
src/media/mf.rs         Media Foundation decoders + probe
src/media/ffpipe.rs     ffmpeg/ffprobe child-process decoders + probe + exe lookup
src/media/waveform.rs   audio peaks cache (background compute, disk cache) — feeds engine/analysis.rs
src/media/thumbs.rs     thumbnail cache (timeline filmstrips, library rows)
src/media/proxy.rs      background all-intra proxy transcodes (preview plays these, exports never do)
src/media/ytdlp.rs      optional URL download

src/scripting.rs        embedded Luau (mlua): sandboxed `editor.tool/tools/log` bridge into the MCP
                        tool catalogue; `-- @name/@desc/@icon/@hotkey/@on <event>/@budget_ms` header
                        parsing (`meta()`); 5 s run budget, 250 ms default `@on` hook budget
src/engine/gpu.rs       GPU renderer: FBO pool, programs, node-graph eval, transitions, masks, readback
src/engine/shaders.rs   all GLSL: VERT/PRELUDE/BLEND/MASK/COMPOSITE + one body per EffectKind
src/engine/compose.rs   CPU compositor (fallback + "source frame only"): placement, transitions, masks
src/engine/effects.rs   CPU effect implementations, wobble()/track() helpers, gpu_only()
src/engine/shapes.rs    shape + recorded-drawing rasteriser
src/engine/text.rs      TextRasterizer (system + imported fonts)
src/engine/blend.rs     blend modes (CPU), mirrored in shaders.rs BLEND
src/engine/mixer.rs     audio mixer: speed/reverse/freeze, pan, fades, transitions, sequences, buses
src/engine/mixer_fx.rs  filter DSP (EQ, reverb, echo, distortion, compressor, gate, noise, gain,
                        DeHum/Limiter/DeEsser) + BusGraph + K-weighted LUFS metering
src/engine/analysis.rs  onset/beat/BPM detection, peak/RMS/LUFS levels, loud-segment speech ranges,
                        cross-correlation sync offset — all pure functions over `waveform::Peaks`
src/engine/lut.rs       stdlib `.cube` 1D/3D LUT parser -> `Lut3D`/`Lut1D` for `EffectKind::Lut`
src/engine/export.rs    ffmpeg export (pipe), FrameSource::{Cpu,Gpu}, lossless cut, encoder detection,
                        ExportPreset table (platform presets), render queue + ETA
src/engine/convert.rs   "Convert To…" transcodes (gif<->video, containers, audio, rescale, batch)
src/engine/capture.rs   screen recording + voiceover through ffmpeg (gdigrab / dshow)
src/engine/import.rs    FCP7 XML / EDL / .prproj import with a per-item report; async asset probing
src/engine/subtitles.rs SRT/VTT parse + write; `cue_layer_at` (shared cue+style lookup, dedup'd in 0b)
src/engine/autocut.rs   silence/speech segmentation from waveform peaks
src/engine/presets.rs   curve/motion presets, effect-chain / node-graph presets, clip templates
src/engine/prerender.rs "movie mode" full-quality frame cache on disk, rendered on a worker thread
src/engine/style.rs     Markdown style summary of a project (for AI style guides)
src/engine/tracking.rs  point / area tracking: NCC template match on a worker thread -> a reusable path
src/engine/transcribe.rs speech->text through a downloaded whisper.cpp model + exe, auto-subs, double takes
src/engine/tts.rs       Windows-voices text-to-speech, dropped on the timeline as a linked audio clip
src/engine/xmeml.rs     FCP7 XML export for Premiere / Resolve

src/playback.rs         Player: render thread + audio thread + wall clock; decode_layers()/
                        request_layers() for the GPU (see "Alt-render channel" below); Clock.rate/
                        loop_range (player-rate-loop)
src/mcp/mod.rs          MCP server (HTTP + JSON-RPC), ToolCall channel, png_encode
src/mcp/tools.rs        ToolDef/ToolKind/ToolOutcome + `all()`/`find()`/`list_json()` — flattens
                        `ui::app::TOOL_TABLES`, the shared truth for `tools/list`, `editor.tools()`
                        and the palette

src/ui/mod.rs           shared UI types/helpers (DragPayload, label_name, duration_text, …)
src/ui/layout.rs        egui_tiles docking, pop-out viewports (`on_viewport` hook), layout profiles,
                        `Pane` enum (`ALL`/`ROUND3`/`glyph`/`title`), `stack_unplaced`
src/ui/palette.rs       Ctrl+K command palette widget: `Command{Action,Pane,Tool,Script,Workspace}`
src/ui/cheatsheet.rs    F1 keyboard-shortcuts overlay (second palette entry point)
src/ui/home.rs          welcome wizard + home screen (first-run layout-mode choice)
src/ui/onboarding.rs    guided-tour overlay
src/ui/confirm.rs       non-blocking confirm windows (`confirm::ask`) replacing blocking rfd Yes/No
src/ui/markdown.rs      in-house Markdown renderer (headings/bold/italic/code/lists/links/rules) —
                        replaces egui_commonmark (size-diet)
src/ui/preview.rs       viewport + transport + tool interactions; transform handles, crop ring, canvas
                        snap guides (canvas-handles-monitor)
src/ui/tools.rs         tool strip with painter-drawn `Glyph` icons (enum/ALL/name/from_name/draw_glyph
                        carry one marker section per workstream)
src/ui/inspector.rs     clip/asset/project properties: dispatch + project panel + per-clip zone-1/
                        zone-2 layout, label/mask/shape/path/markers sections
src/ui/inspector_audio.rs clip audio properties (props grid, fades, blend, bus override, Duck/Normalize
                        buttons dispatching Action::AutoDuck/Normalize)
src/ui/inspector_text.rs text/typography editing + per-selection style overrides
src/ui/color_ui.rs      Color inspector section: LUT/Primaries/Qualifier params, Gallery Looks tabs
src/ui/effects_ui.rs    effect catalogue (thumbnail grid) + the clip's effect stack (drag-reorder, bulk edit)
src/ui/gallery.rs       Gallery pane: re-purposed `Pane::Presets` drawer (effects/node-graphs/adjustment
                        layers/templates/LUTs/Looks/Titles/Captions tabs) (inspector-gallery)
src/ui/nodes.rs         node-graph editor
src/ui/curves.rs        keyframe graph editor (bezier velocity, presets, flow)
src/ui/library.rs       library + recent: folders, tags, labels, search, filters, linked folders,
                        offline detection/badge, Relink/Consolidate, subclips, Smart Bins, sortable
                        columns, thumbnail-viewport culling, image-sequence import
src/ui/source_ui.rs     Source monitor content (`Pane::Source`): own Player, in/out marks, three-point/
                        smart-edit buttons, Subclip-from-marks, Source Tape
src/ui/mixer_ui.rs      bus strips, meters (incl. LUFS), filter chains
src/ui/markers_ui.rs    marker list (collapsible rows, icons, per-sequence scoping, bulk ops)
src/ui/planner.rs       Plan / Notes / Timer tabs: task tree, markdown notes, stopwatch dial
src/ui/moodboard_ui.rs  standalone moodboard pane (gallery/list/slideshow, tags, add-at-playhead)
src/ui/history_ui.rs    History pane over the undo stack (day groups, search, filters, md export)
src/ui/guides.rs        social-platform guide overlays + aspect/format presets
src/ui/heartbeat.rs     repaint scheduling helper
src/ui/subtitles_ui.rs  subtitle editor
src/ui/transcript_ui.rs collapsible Transcript section (word click-to-seek, drag-select+delete cut,
                        filler-word list, search, TTS panel) inside Subtitles
src/ui/autocut_ui.rs    auto-cut pane (silence/speech detection, Mark-instead)
src/ui/tracking_ui.rs   Tracking pane: place a box, track a clip, save the result as a path
src/ui/scopes_ui.rs     Scopes window (waveform/histogram/vectorscope-style monitor overlay)
src/ui/multicam_ui.rs   multicam angle grid (`egui::Window`, capped at 4 angles at ¼ size)
src/ui/find_ui.rs       Find window (search clips/markers/text across the timeline) (pro-timeline)
src/ui/{retime,export_ui,frame_ui,capture_ui,import_ui,paste_ui,transitions_ui,shader_ui}.rs windows
src/ui/settings_ui/mod.rs Settings window: tabs + shared plumbing (settings_ui/*.rs re-exports)
src/ui/settings_ui/{appearance,capture,general,hotkeys,performance}.rs one settings tab each

src/ui/app/mod.rs       App: top-level state + frame loop; the five dispatch registries
                        (TOOL_TABLES/ACT_HANDLERS/FRAME_HOOKS/WINDOW_DRAWERS/PANE_DRAWERS — see
                        "Registries" below); fire_hook/fire_markers_added
src/ui/app/actions.rs   act() — the Action match, falls through to ACT_HANDLERS first
src/ui/app/audio_actions.rs  audio-analysis's ACT_HANDLERS entry (AutoDuck/Normalize dispatch)
src/ui/app/autosave.rs  debounced off-thread autosave, crash recovery snapshot
src/ui/app/boot.rs      App::new's startup sequence (extracted for readability)
src/ui/app/caches.rs    cache-size/clear plumbing behind Settings ▸ Performance's button
src/ui/app/drops.rs     drag-and-drop handling (files, library, timeline)
src/ui/app/edit_ops.rs  shared edit-op dispatch helpers used by several tools_*.rs files
src/ui/app/feedback.rs  Toast{kind, action, progress, dedupe, dismiss} builder + draw
src/ui/app/files.rs     open/import/save/export/overwrite/finish_export; fire_hook("project_open"/
                        "project_save"/"export_done", …) call sites (see "Registries" -> Luau hooks)
src/ui/app/frame.rs     SelSig (selection-signature helper used by fire_hook("selection_changed", …))
src/ui/app/gallery_ctl.rs PANE_DRAWERS entry for Pane::Presets -> the Gallery UI
src/ui/app/gpu.rs       sync_gpu … source_frame: GPU wiring shared by preview/export/alt-render
src/ui/app/jobs.rs      screen capture, voiceover, import_recording, audio_inputs, probe polling
src/ui/app/layout_ctl.rs layout_mode/pin/glow/reveal_auto, WORKSPACES builders, maximize
src/ui/app/library_pane.rs Pane::Library draw
src/ui/app/mcp_exec.rs  run_script, sync_mcp/poll_mcp/handle_tool/start_tool_job, run_tool_undoable
                        (the single Mutate/Read/Job/Ui wrapper for MCP + scripts + palette)
src/ui/app/media_sync.rs media-library's FRAME_HOOKS/ACT_HANDLERS/WINDOW_DRAWERS entries; fire_hook
                        ("import", …) call site (image-sequence bake completion)
src/ui/app/menus.rs     glyph_for, menu_item, pane_glyph, view_menu, menu_bar
src/ui/app/monitor.rs   AltRequest/AltRenderState — the async alt-render pipeline (see "Alt-render
                        channel" below); canvas-handles-monitor + pro-monitor's ACT_HANDLERS/tick
src/ui/app/palette_ctl.rs command-palette's tick/act/windows; fire_hook + fire_hook("selection_changed",
                        …) call site; script metadata cache
src/ui/app/panes.rs     draw_pane/draw_pane_inner — falls through to PANE_DRAWERS first
src/ui/app/playback_ctl.rs player-rate-loop's ACT_HANDLERS/FRAME_HOOKS entries (JKL shuttle, loop, rate)
src/ui/app/preview_pane.rs Pane::Preview draw
src/ui/app/recovery.rs  "Recover unsaved project?" startup offer
src/ui/app/source_ctl.rs source-monitor's ACT_HANDLERS entry
src/ui/app/source_pane.rs Pane::Source draw + FRAME_HOOKS tick (PANE_DRAWERS entry)
src/ui/app/thumbs.rs    build_effect_thumbnails + effect_thumb_* + STOCK + box_blur
src/ui/app/timeline_pane.rs Pane::Timeline draw
src/ui/app/tools_args.rs Args<'a>(&'a Value) typed-getter helper (ids_or_selection, t_or_playhead, …)
src/ui/app/tools_{audio,clip,color,commands,export,gallery,layout,media,mixer,monitor,playback,
           preview,project,source,subtitles,timeline,timeline_pro,titles,transcript,trim,ui}.rs
                        one `pub const TOOLS: &[ToolDef]` per tool group, registered in TOOL_TABLES;
                        several use a local `row!(name, kind, desc, args)` macro over the same
                        `ToolDef` shape for density
src/ui/app/tools_helpers.rs shared free-fn helpers for the tools_*.rs handlers
src/ui/app/tools_registry_tests.rs every_edit_op_has_a_tool / OP_TOOLS / OP_INTERNAL structural test
src/ui/app/transcript_ctl.rs transcript-captions' ACT_HANDLERS/FRAME_HOOKS/WINDOW_DRAWERS entries
src/ui/app/trim_actions.rs trim-model's ACT_HANDLERS entry (ripple/roll/slip/slide dispatch)
src/ui/app/whatsnew.rs  What's New window (registries-schema-hooks); help.changelog/templates.save tools
src/ui/app/windows.rs   name_window, windows() — falls through to WINDOW_DRAWERS first
src/ui/app/tests.rs     app-level tests

src/ui/timeline/mod.rs   the timeline widget: types + top-level draw hub, Act enum, click/select handling
src/ui/timeline/arm.rs   the frozen modifier table: `arm(mods, zone, flags, tool) -> Option<GestureKind>`
                         (see "Gesture model" below) — pure lookup, one test per row
src/ui/timeline/snap.rs  tiered `snap_target` (playhead > cursor > selected-clip edges > adjacent clips >
                         markers), weakens with zoom, draws the guide line before release
src/ui/timeline/gestures.rs drag-state machine consuming `arm()`'s `GestureKind` (ripple/roll/slip/
                         slide/segment/multi-roller trims, ghost-paint, apply-on-release)
src/ui/timeline/header.rs ruler/toolbar, track header rename (inline TextEdit) / colour / reorder
src/ui/timeline/cue_lane.rs subtitle-cue lane (select/trim/split/resize like a clip)
src/ui/timeline/menus.rs context menus (clip/label/transition/transform)
src/ui/timeline/paint.rs draw helpers (clip bodies, waveforms, thumbnails, view presets, overview strip)
src/ui/timeline/tests.rs timeline-level tests
```

Everything under `src/` not listed above (there is none at the time of writing — the tree above is
exhaustive for `src/ui/app`, `src/model`, `src/ui/timeline`, and covers every other file one level
deep) is either a leaf data/engine module already itemised or a top-level file listed at the top of
this map.

## Data model (`src/model/`)

`Project` owns everything: `assets`, `folders`/`linked_folders`, `tracks` (video first, then audio;
each with `locked: bool`, `ripple: Option<bool>` — `#[serde(default)]`-resolved in `from_json` per the
schema-first rule — and `magnetic: bool`), `sequences` (nested timelines) with `editing`/`main_stash`
for the one being edited, `subtitles`, `transcripts` (per-clip word timings in timeline seconds),
`markers`, `labels`, `buses`, `plan`, `notes`, `paths` (saved outlines), `moodboard`, in/out points,
`scaler`, and a monotonic `next_id`. `Project::VERSION = 2` warns on a newer file than this binary
understands.

* `Clip` — `kind` (Video/Image/Text/Audio/Sequence/Shape/Adjustment), timing (`start`, `duration`,
  `src_in`), retime (`speed` + the keyframable `speed_curve` ramp, `reverse`, `freeze`),
  transform/`opacity`/`blend`, `effects` (linear stack) or `graph` (node DAG), `mask`, audio
  (`volume`, `pan`, `fade_in/out`, `bus`, `audio_role: AudioRole` — Unset/Dialogue/Music/Sfx/Ambience,
  drives the inspector's audio defaults and ducking target selection), `text`, `shape`, `markers`,
  `label`, `link` (clips that move/split/delete together).
* `Asset` — gained its own `effects: Vec<Effect>` (color-engine's master/source-clip effect chains,
  applied ahead of any per-clip stack) and an `AssetStatus` (`Ready`/`Decoding`/`Offline`/
  `ProxyBuilding`, media-library) surfaced on library rows/inspector/preview badge.
* `Animated { value, keys }` — every keyframeable property. Keys carry an `Ease`
  (Linear/EaseIn/Out/InOut/Hold/**Bezier** handles). Times are clip-local seconds.
* `Effect { kind, params: Vec<Animated>, mask, shader, start, len }` — `EffectKind::params()` is the
  parameter table (name/default/min/max); `is_bool_param()` marks checkbox params; `is_geometric()` marks
  effects folded into the placement; `needs_motion()` marks ones needing neighbouring frames. `start`/`len`
  are the clip-local window it runs in (`Effect::on_at`; `len <= 0` = to the end of the clip). Gained
  `Lut`/`Primaries`/`Qualifier`/`FrameBlend` kinds (color-engine) with GLSL bodies + CPU fallbacks or
  `gpu_only()`.
* `NodeGraph { nodes, edges }` — `NodeKind::{Input, Color, Clip, Asset, Effect, Blend, Combine, Merge,
  Matte, Mask, String, Number, Bool, Random, Math, Compare, Logic, Select, Output}`, cycle-refusing
  `connect`, `eval_order()`, `eval_values()`, `to_effects()`. Opt-in: only a graph with real nodes
  (`Clip::uses_graph`) replaces the linear stack for rendering.
* `Mask` — Rect/Ellipse/Polygon/Path with animated centre/radii/rotation/feather/expand/opacity/invert.
* `Transition` — lives on a `Track`, centred on a cut, **window clamped to its two clips**
  (`Transition::window(left, right)`); kinds CrossFade / FadeToColor / Push / Wipe.
* `Bus` / `AudioFilter` / `FilterKind` — the mixer graph (now incl. `DeHum`/`Limiter`/`DeEsser`,
  audio-dsp-automation); `Project::bus_of(track, clip)` resolves routing; per-bus gain automation
  keyframes (`bus.volume_key`).
* `AttrSet` — which attributes Copy/Paste Attributes transfers.
* `Transcript` (per-clip, `transcript-captions`) — words in timeline seconds, `Cue.words` for
  karaoke-style caption animation, `cut_word_ranges` ripples cues/markers/transcript together.

Everything is `serde`-serialised; `Project::from_json` sanitises hand-edited files (fps, sizes, NaNs,
dangling ids, missing-field defaults) and `save()` writes a temp file then renames.

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
  only the frame ranges an edit can change (clips by id, JSON equality); returns `None` (full clear) if
  a `Track.id` differs at any index (reorder). A finished proxy (`Cmd::Proxies`) likewise evicts only
  `spans_using_source` for the remapped files instead of the whole cache. Track flags/colour/volume
  never enter the full-clear list (pinned by `flags_do_not_dirty_video`); audio/marker/planner edits
  evict nothing; canvas/fps/asset/sequence changes still clear everything.
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
* **Rate/loop** (`player-rate-loop`): `Clock.rate` and `Clock.loop_range` parameterise the previously
  fixed forward-1x/no-loop clock (`now = base_t + elapsed·rate`, stop at 0/duration, wrap inside a loop
  range) — JKL shuttle, Loop In→Out and Play In→Out all build on this, not a second clock.

## Threading

* UI thread: egui, all GL, MCP tool execution, export frame service, pre-render slices, alt-render service.
* `Player` render thread: decoders + (CPU) compositor or layer decoding; publishes `Arc<Frame>` / `LayerSet`.
* `Player` audio thread: decoders + `Mixer` + `BusGraph` → ring buffer → cpal. Wall clock is the master.
* Export / convert / capture threads: their own decoders or an ffmpeg child.
* Waveform + thumbnail workers: one background thread each, LIFO, memoised failures.
* Every decode on a long-lived worker runs under `catch_unwind`; MF objects are per-thread
  (`unsafe impl Send` newtypes, COM/MF init per thread).

## Registries

The UI/UX overhaul (`plans/ui-overhaul/`) added five append-only dispatch registries in
`src/ui/app/mod.rs` so 23 concurrent workstreams could add code without ever sharing a merge hunk.
Each is a `const` slice pre-seeded with one `// ---- ws:<name> ----` marker-comment line per
workstream (wave-then-name order); a workstream's PR replaces **only its own line** with a real
entry — never a neighbour's:

| Registry | Type | Tried from | Wins when |
|---|---|---|---|
| `TOOL_TABLES` | `&[&[mcp::tools::ToolDef]]` | `mcp::tools::all()` (flattens for `tools/list`, `editor.tools()`, the palette) | always included — not a fallthrough dispatcher |
| `ACT_HANDLERS` | `&[fn(&mut App, Action) -> bool]` | `act()`'s prelude, before its own `match` | first handler returning `true` |
| `FRAME_HOOKS` | `&[fn(&mut App, &egui::Context)]` | `update()`, once per frame | every hook runs (not a fallthrough — polling, not dispatch) |
| `WINDOW_DRAWERS` | `&[fn(&mut App, &egui::Context)]` | `windows()`, before its own body | every drawer runs |
| `PANE_DRAWERS` | `&[fn(&mut App, &mut egui::Ui, Pane) -> bool]` | `draw_pane_inner()`, before its own `match` | first drawer returning `true` |

Same protocol on four more shared tables: `src/hotkeys.rs`'s `actions!` macro body (one section per
workstream; `reserved_chords_are_free`/`no_duplicate_defaults` tests guard bare `S`, `Shift+S`,
`Ctrl+Y`, `Backspace`, `Esc`, `Tab`/`Shift+Tab`, `Alt+Space`), `src/ui/tools.rs`'s `Glyph`
enum/`ALL`/`name`/`from_name`/`draw_glyph` (`every_glyph_paints_a_picture` guards), `Pane::ALL`
(`src/ui/layout.rs`, now a slice — a new pane needs one line, not a signature change at every call
site; `stack_unplaced` places any pane a preset builder forgot), and `Settings`/`Project`/`App`
struct literals (schema-first: every planned serde field landed in wave 0b with `#[serde(default)]` —
feature branches only ever add fields, never touch an existing type definition's shape mid-wave).

**Tool registry** (`src/mcp/tools.rs` + `src/ui/app/tools_*.rs`): every MCP tool is a
`ToolDef { name, desc, args, kind: ToolKind, run }` beside its handler in a `tools_<group>.rs` file
(a `pub const TOOLS: &[ToolDef]`, registered once in `TOOL_TABLES`; several files use a local
`row!(name, kind, desc, args)` macro over the same shape). `ToolKind::{Read, Mutate, Job, Ui}` drives
undo policy in `App::run_tool_undoable` — the single wrapper for MCP calls, Luau's `editor.tool`, and
the palette's arg-free/`:` rows: `Mutate` snapshots JSON, runs, pushes one undo step iff the JSON
changed, rolls back on `Err`; `Read`/`Ui` just run; `Job` polls the existing `McpJob`/`Progress`
machinery. `tools_registry_tests.rs`'s `every_edit_op_has_a_tool` scans every `src/model/ops/*.rs` for
`pub fn NAME(&mut self` and fails the build unless `NAME` has a `ToolDef` row or a recorded
`OP_INTERNAL` reason — **live count at merge time: 236 tools** (13 `Job`, 109 `Mutate`, 69 `Read`, 45
`Ui`; see `docs/customizing.md`'s full table, transcribed from a live `tools/list` call, not this
count alone).

## Gesture model

The timeline's modifier table is frozen in `src/ui/timeline/arm.rs`: a pure lookup
`arm(mods: Modifiers, zone: Zone, flags: TrackFlags, tool: Tool) -> Option<GestureKind>`, one
`#[cfg(test)]` per row. Only the `Tool::Select` tool consults it — every other tool (Cut, Marker,
Stretch, Spacer, Draw, Text, Shape, Mask) keeps its own pre-existing gesture regardless of zone or
modifier (`GestureKind::LegacyToolGesture`). `flags.magnetic` (a per-track opt-in, `Track.magnetic`)
flips a plain `Body` drag from `MoveNoOverlap` to `MagneticMove` and a plain `Edge` drag from `Trim`
to `RippleTrim` — nothing else about the table depends on track state.

| Zone | Ctrl | Alt | Shift | Ctrl+Alt | Ctrl+Shift | Plain |
|---|---|---|---|---|---|---|
| `Body` | MoveNoOverlap (toggle-select) | Slip | MoveNoOverlap | Slide | Segment | MoveNoOverlap (MagneticMove if `flags.magnetic`) |
| `BodyBottom` | — (undefined) | — | — | — | — | SplitAt |
| `EdgeStart`/`EdgeEnd` | RippleTrim | Roll | RateStretch | MultiRippleTrim | — | Trim (RippleTrim if `flags.magnetic`) |
| `Seam` | SeamLeft | SeamRight | SeamAddToSet | — | — | SeamBoth |
| `Fade`/`VolumeLine`/`Key`/`Marker`/`TransitionEdge` | *(arm() abstains on every modifier — caller keeps its pre-existing gesture unchanged)* |||||
| `Lane` | — | — | RubberBandAdd | — | — | GapSelect |
| `RulerInOut` | — | — | — | — | — | RulerInOutDrag |
| `Drop` | DropSplice | DropOverwrite | DropPlaceOnTop | — | — | DropDefault |

`GestureKind::Pan` (middle-mouse) and `LegacyTool` (any zone, any modifier, a non-`Select` tool) exist
in the enum for documentation completeness but are button-/tool-driven, not modifier-driven — never a
real `arm()` output for those inputs. Wiring status at merge time: `timeline-trim-gestures` (wave 2)
wired every `Edge` row and all four `Drop` rows into a live drag; `pro-timeline` (wave 3) wired
`Seam`'s Shift row (asymmetric multi-roller trim); `docs-refresh` wires nothing (docs-only) — the
`Body`/`Lane`/`RulerInOut` rows and the rest of `Seam` are tested and compiled but not yet consumed by
a live drag, a known, tracked gap (not silently claimed as shipped here).

`Zone`/`TrackFlags`/`GestureKind` have no `Project` dependency of their own — callers read the track
and build the `TrackFlags` themselves, keeping `arm()` a flat, side-effect-free lookup a test can hit
directly.

## Keymap

Defaults live in `src/hotkeys.rs`'s `actions!` macro (one marker section per workstream); named
presets (`Avid`/`Premiere`/`Resolve`) are diff tables in `src/keymaps.rs` applied over the defaults —
`keymap_presets_resolve_and_have_no_duplicate_chords` pins every preset to a real Action id with no
duplicate chord. Full reference in `docs/customizing.md`; the headline additions from the overhaul:

| Area | Chords |
|---|---|
| Discovery | `Ctrl+K` Command Palette, `F1` Cheat Sheet, `Ctrl+Shift+G` Layout Mode toggle |
| Shuttle/transport | `J`/`L` shuttle reverse/forward (ladder −1,−2,−4,−8 on repeat), `Ctrl+Shift+L` Loop In→Out, `Ctrl+Shift+Space` Play In→Out, `Ctrl+Space` Play to Out, `/` Play Around Playhead, `Shift+←`/`Shift+→` step 10 frames |
| Edit points | `U` select nearest, `Shift+U` cycle side, `Shift+I`/`Shift+O` go to In/Out |
| Trim | `[`/`]` ±1 frame, `Ctrl+[`/`Ctrl+]` ±10 frames (not `Shift+[` — see below), `E` extend to playhead, `Q`/`W` trim start/end (Top/Tail) to playhead |
| Slip | `Alt+,`/`Alt+.` ±1 frame (sorted before the bare Nudge chords) |
| Marking | `X` mark clip (in/out from clip under playhead) |

`Shift+[` reaches egui as the logical key `{` (egui-winit has no physical-key fallback in 0.33's
`consume_key`), so the ±10-frame trims are on `Ctrl+[`/`Ctrl+]` instead — a keyboard-layout
workaround, not a design preference. `Alt+Space` is never bound (reserved by the Windows system
menu). Every existing default keeps its old chord; presets are additive diffs, never a re-key.

## Alt-render channel

Hover previews, the trim view, Scopes, wipe compare and the multicam angle grid all need a second,
GPU-accurate render of a *different* moment/effect than what's currently playing, without touching the
live preview or falling back to the slower CPU compositor. `src/ui/app/monitor.rs`'s `AltRenderState` +
`AltRequest` (`Effect(kind)`, `Transition(kind)`, `Gallery(namespace, name)`, and pro-monitor's
`TrimOut`/`TrimIn`/`Compare`/`Angle(u8)`) is the shared plumbing:

* One `AltKey = (AltRequest, Option<Id>, i64)` coalesces requests — the bare `AltRequest` isn't enough
  because the same request can resolve to a different clip/time as selection or the playhead moves, and
  the third field is a frame-quantized playhead so a `shown` request whose context moved doesn't keep
  painting a stale frame.
* `App::request_layers` (via `Player`) renders through the same GPU shader set as the live preview —
  never the CPU compositor — newest request per consumer wins, at most one in flight.
* **Contention with export**: every alt-render call site checks `self.export.is_some()` first and
  skips the request while an export is running, since the UI thread also services `GpuFrameRequest`s
  for the export thread and the two must never fight over the GL context in the same frame.
* Pro-monitor's `TrimSlot` (outgoing/incoming trim-view frames) is a deliberately separate small state
  machine from the shared `AltRenderState`, not a retrofit onto it — see `TrimView`'s doc comment in
  `monitor.rs` for why.

## New-pane checklist

Exactly **one** new `Pane` variant landed in the whole overhaul: `Pane::Source` (wave 0b,
pre-declared; filled with real content by `source-monitor` in wave 2). Everything else that looked
pane-shaped in the original design became something cheaper: Color is an inspector section +
Gallery tabs, Scopes/the multicam angle grid are `egui::Window`s, Transcript is a collapsible section
inside Subtitles, Titles/Captions are Gallery tabs, the timeline overview is an inline minimap strip,
trim view/wipe are Preview overlays, `Pane::Presets` kept its variant but now draws the Gallery. A new
pane costs all of the following — do every step, in this order, before calling it done:

1. Add the variant to `Pane` and to the `Pane::ALL` slice (own marker section) in `src/ui/layout.rs`.
2. Give it a `glyph()` (`Glyph` enum + `ALL` + `draw_glyph` in `src/ui/tools.rs`) and a `title()`.
3. Every preset builder must place it or explicitly leave it for `stack_unplaced` to tab-stack behind
   an existing pane (never silently missing from a saved layout).
4. A toggle `Action` (View menu + palette) or a `ToolDef` — `every_pane_has_a_toggle_action_or_tool`
   fails the build otherwise.
5. A `PANE_DRAWERS` entry (own marker section) in `src/ui/app/mod.rs`, or a direct arm in
   `draw_pane_inner`'s match if you're the pane's original 0a owner.
6. An `assert_no_idle_repaint`-style test (see "Size & idle-CPU gates" below) — every existing pane's
   test file has one to copy the shape from (`src/ui/palette.rs`, `src/ui/scopes_ui.rs`,
   `src/ui/multicam_ui.rs`, `src/ui/source_ui.rs`, `src/ui/timeline/tests.rs`, …).
7. Decide `Pane::ROUND3` membership (panes added in that older round; a stored layout without them is
   treated as pre-ROUND3 in `from_json`) — `Pane::Source` is deliberately **not** in `ROUND3`.

## Size & idle-CPU gates

`scripts/size.ps1 [-Note]`: builds `--release`, appends `<sha>,<bytes>,<note>` to `size_log.csv`
(`.gitattributes` marks it `merge=union` — and *only* it plus `CHANGELOG.md`; every `.rs` file relies
on disjoint hunks, never a union merge), and prints the delta against the previous row. `/se-verify`
fails a pass when the release exe grows more than 64 KB since the last row with no `size:` line in the
PR body explaining why; a growth over 300 KB needs a named offset, not just a number. Baseline history
(`size_log.csv`): 15,970,816 B pre-size-diet → 10,121,728 B post-size-diet (issue #18, corrected —
the main-crate `opt-level = "s"` gate *passed* `bench_4k_preview`/`headless_1000_clips_stays_fast` and
is kept, not reverted) → **12,167,168 B (11.60 MB)** at wave-3-complete, the number this file's header
reports. Feature waves added the size back; getting under the ~10 MB core goal stays open (goals.md).

Idle CPU stays 0 % by convention, not by a single shared test helper: every new pane/overlay/window's
own test file adds a test named `assert_no_idle_repaint_<context>` that runs ~30 headless frames with
no input through a bare `egui::Context` and asserts `!ctx.has_requested_repaint()` afterward (grep
`fn assert_no_idle_repaint` across `src/` — it is a **naming convention repeated per test**, not a
shared harness fn; copy the pattern from an existing one rather than looking for a helper to call).
`App::animate_until(ctx, at)` is the one sanctioned funnel for a *new* timed-repaint request
(`winpos.rs`'s window-rect debounce and `whatsnew.rs`'s version-gate tick are its first two callers);
17 pre-existing raw `ctx.request_repaint_after(...)` call sites predate this and are an accepted,
un-migrated ceiling, not a codebase-wide invariant yet. `--selftest` has its own `idle_repaint` step
(a bare idle frame after opening a clip requests no repaint) as a smoke-level, not exhaustive, check.

## Feature inventory (implemented and tested)

**Editing** — open any video/gif/image/audio; trim, split (Ctrl+B), ripple delete, drag/move, tiered
snapping with a pre-release guide line, multi-track video+audio, linked clips, rubber-band
multi-select, drag above/below the tracks to create one, in/out points, nudge, undo/redo (JSON
snapshots, 200 deep). Ripple/roll/slip/slide/segment/multi-roller trims and per-track
locked/ripple/magnetic flags (trim-model, timeline-trim-gestures, pro-timeline) — see "Gesture model".
Track header rename/colour/reorder, view presets, an overview minimap strip, and a Find window across
clips/markers/text (pro-timeline).
**Retime** — speed, reverse, freeze frame (Ctrl+R); parameterised playback rate/loop range powers JKL
shuttle, Loop In→Out and Play In→Out (player-rate-loop).
**Effects** — 24+ kinds incl. Blur, Motion Blur, Pixelate, JPEG Compression, VHS, Chroma Key,
Threshold, Edge Glow, Color Tint, Color Correction, Curves, Levels, Hue/Saturation, B&W, Invert,
Vignette, Sharpen, Flip, Crop, 3D Plane, Camera Shake, Blob Tracking, Security-camera REC, a user GLSL
Shader, plus color-engine's `Lut`/`Primaries`/`Qualifier`/`FrameBlend` (stdlib `.cube` parser, builtin
Looks, master/source-clip effect chains via `Asset.effects`). Each has keyframeable params and an
optional mask.
**Transitions** — cross fade, fade to colour, push, wipe; audio crossfades mirror them.
**Masks** — per clip and per effect; rect/ellipse/polygon/path with feather, expand, invert.
**Node editor** — chain/combine effects with blend/combine/merge, matte, mask, colour, clip, asset and
text inputs; "Unlink" turns a simple chain back into a plain effect list.
**Keyframes** — value-positioned diamonds on clips, a graph editor with bezier velocity handles, easing
menus, curve/motion presets, "flow" between two clips.
**Text & titles** — system + imported fonts, size, bold/italic, fill/outline/shadow/box, alignment,
spacing; Gallery ▸ Titles template thumbnails with exposed, keyframeable animation params
(reveal/wave, text-titles).
**Shapes & drawing** — rect/ellipse/triangle/polygon/star/line/arrow plus recorded freehand drawings;
outlines/drawings save to `Project.paths` and reuse as motion paths or tracking targets.
**Tracking** — the Tracking pane follows a point or box through a clip (NCC template matching, worker
thread, progress bar); saves to `Project.paths`, applies to X/Y keyframes.
**Adjustment layers** — effects that apply to everything below them.
**Sequences** — nested timelines usable as footage.
**Multicam** — create from cross-correlation sync offsets (`audio.sync_offset`) into a nested sequence
with one track per angle; switch = split + enabled toggles; angle grid capped at 4 angles, ¼-size
alt-renders (pro-monitor).
**Audio** — waveforms, per-clip volume line + fades, pan, mute/solo, buses with EQ/reverb/echo/
distortion/compressor/gate/noise/gain/DeHum/Limiter/DeEsser, K-weighted LUFS metering, per-bus volume
automation, per-track and per-clip routing, meters, mono fold, silence auto-cut, voiceover recording.
Beat/onset/BPM detection, peak/RMS/LUFS analysis, loud-segment speech ranges, cross-correlation sync
offset, AutoDuck/Normalize/one-click repair chains, `AudioRole` tagging (audio-analysis,
audio-dsp-automation).
**Subtitles & transcript** — editor, SRT/VTT import/export, burnt-in rendering; a persisted
per-clip transcript (word timings survive save/reopen), click-to-seek/drag-select-and-cut, filler-word
removal, search, karaoke-style caption animation (Highlight/Pop/Typewriter), Windows-voices
text-to-speech dropped as a linked clip (transcript-captions).
**Markers** — timeline and clip markers with labels and notes; `marker_added` fires once per marker
from every creation path (also exposed over MCP).
**Library** — folders, tags, custom labels, descriptions, search, filters, linked folders, offline
detection/badge, Relink/Consolidate, subclips, Smart Bins, sortable columns, thumbnail-viewport
culling, keyboard nav, image-sequence import, Convert To… (incl. batch), Remove unused, optional URL
import (media-library).
**Planner** — nested tasks with moodboards, plus free-form project notes.
**Capture** — screen recording (region, bitrate, mic, desktop audio, record-while-unfocused) and voiceover.
**Export & delivery** — any container/codec by extension, resolution + scaler, hardware encoders, fast
lossless cut (`-c copy`), export frame (PNG/JPG/WebP), Premiere/Resolve XML, style summary Markdown,
platform preset tiles, a render queue with ETA, Export In/Out range + letterbox, loudness
normalisation, render-in-place bake pipeline (stabilize/denoise/slow-mo), markers CSV/YouTube-chapters
export+import (export-deliver).
**Import** — Premiere/Resolve FCP7 XML, EDL, `.prproj` with a per-item report.
**Layout & onboarding** — dockable panes, pop-out windows (with hotkeys polled on the child viewport),
saved/shared profiles, `Settings.layout_mode` (Dynamic contextual vs. Granular explicit), named
workspaces (Simple/Audio/Text/Deliver, Alt+1..6), pin/lock per panel, a first-run welcome wizard and
home screen, an adaptive tool strip (layout-modes-onboarding).
**Command palette & scripting** — `Ctrl+K` palette over every Action/Pane/arg-free ToolDef/script/
workspace, `F1` cheat sheet, named keymap presets (Avid/Premiere/Resolve), Luau `-- @on <event>` script
hooks (command-palette).
**Forgiveness** — non-blocking toasts with Undo actions, non-blocking confirm windows (no more
blocking Yes/No dialogs), debounced autosave with crash recovery, cache-clear/size in Settings, History
panel restore (forgiveness).
**AI** — MCP server (toggle in Settings) exposing **236 tools** over `http://127.0.0.1:<port>/mcp`
(default port 7337; see `docs/customizing.md` for the full, namespace-grouped table). Luau scripts
(`src/scripting.rs`, Scripts menu) call the identical tool catalogue in-process via `editor.tool`/
`editor.tools`/`editor.log` — one API surface for scripts and MCP clients; each script run is a single
undo step; `-- @on <event>` headers fire on real `fire_hook` call sites (see `docs/customizing.md`'s
event table — not every originally-planned event has one yet, and that doc says which).

**Icons** — all UI icons are runtime-painted `Glyph` variants (`ui/tools.rs`; fonts lack the symbol
chars). `tools::action_glyph` / `Pane::glyph` give defaults; `Settings.icon_overrides`
("action.<id>" / "pane.<title>" → glyph name or "none", edited in Settings ▸ Appearance ▸ Icons)
overrides them.

## Conventions

* Rust 2021, `cargo fmt` (rustfmt.toml, 120 cols), warning-free intent (a handful of pre-existing
  `dead_code`/`deprecated` warnings are tracked, not a blocker — don't add new ones).
* No unwrap() on external data; reuse buffers in hot paths; no busy loops.
* Mark deliberate shortcuts `// ponytail: <what> — <upgrade path>`.
* Non-trivial logic leaves a `#[cfg(test)]` test or is covered by `--selftest`; widgets are tested with
  headless `egui::Context::run` harnesses (see `ui/timeline/`, `ui/library.rs`).
* Panels take an `undo` closure and call it **once per gesture, before mutating**, only if something
  changed. Panels that cannot reach `Settings` hand values back through documented thread-locals.
* Every new `pub fn ...(&mut self)` on `Project` needs a `ToolDef` row (or a recorded `OP_INTERNAL`
  reason) — `every_edit_op_has_a_tool` fails the build otherwise. Every new `Action`/`Gesture`/`Pane`
  needs the matching registry entries described in "Registries" and "New-pane checklist" above; see
  agents.md's "Concurrent worktree protocol" for the full process rule.

## Verification (run all three before declaring done)

```
cargo test                                  # unit + headless widget tests
cargo run -- --selftest                     # real media through decoders, engine, export
cargo run -- <video> --screenshot out.ppm   # renders the UI; SE_SCREENSHOT_DELAY=<s> to warm caches
```
Convert the screenshot with `ffmpeg -i out.ppm out.png` and actually look at it. Run
`scripts/size.ps1 [-Note <name>]` whenever the change adds code or a dependency (see "Size & idle-CPU
gates" above).
