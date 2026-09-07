# Changelog

## beta-0.3.0

The full UI/UX overhaul (`plans/ui-overhaul/`): 19 feature workstreams across waves 1-3 plus the
three wave-0 refactor PRs (22 total), closed out by this wave-4 docs pass (issue #38). This section folds what
used to be a separate `## unreleased` heading (PRs #12/#14, plus issues #16/#18/#21/#29/#32, kept
verbatim below) together with one new terse paragraph per remaining workstream area. Release exe at
wave-3-complete: **12,167,168 B (11.60 MB)**, sha `86c1793` — see `size_log.csv` and goals.md's
Core-goals line.

### Command palette & scripting (issue #20)
`Ctrl+K` opens a command palette over every Action, Pane, arg-free ToolDef (`:` opens a mini arg
form), script and workspace, with recency-ordered results; `F1` opens a keyboard-shortcuts cheat
sheet (a second palette entry point). Avid/Premiere/Resolve keymap presets are diff tables over the
defaults (no re-keying). Luau scripts gained `-- @name/@desc/@icon/@hotkey/@on <event>/@budget_ms`
header metadata so a script can appear as its own hotkey-bound, palette-visible command or subscribe
to an editor event.

### Trim & gesture primitives (issues #22, #23, #30)
A tiered `snap_target` (playhead > cursor > selected-clip edges > adjacent clips > markers) draws its
guide line before release and weakens as you zoom in. Ripple/roll/slip/slide/segment/multi-roller
trim primitives landed headless-first in `model/ops/trim.rs`, with `Track.locked`/`ripple`/`magnetic`
flags; `timeline-trim-gestures` then routed real mouse drags on the timeline through the frozen
`arm()` modifier table (Edge and Drop zones wired live; Body/Lane/Seam's non-Shift rows stay
tested-but-unwired, a known gap, not a silent claim of full coverage).

### Color engine (issue #24)
New `EffectKind::{Lut, Primaries, Qualifier, FrameBlend}` with GLSL bodies and CPU fallbacks, a
stdlib `.cube` 1D/3D LUT parser, builtin Looks, and master/source-clip effect chains via a new
`Asset.effects` field (applied ahead of any per-clip stack).

### Player rate/loop (issue #25)
`Clock.rate` and `Clock.loop_range` parameterise what used to be a fixed forward-1x, no-loop clock —
one mechanism now backs JKL shuttle (with a −1,−2,−4,−8 repeat ladder), Loop In→Out, Play In→Out,
Play Around Playhead, and the ±10-frame trim chords.

### Audio analysis & DSP (issues #19, #26)
Pure analysis functions over cached waveform peaks (onset/beat/BPM detection, peak/RMS/LUFS levels,
cross-correlation sync offset for multicam) back new AutoDuck/Normalize inspector buttons and
one-click repair chains; clips can be tagged with an `AudioRole` (Dialogue/Music/Sfx/Ambience).
`mixer_fx.rs` gained cascaded-notch DeHum, a lookahead-limiter Limiter, and a sidechain DeEsser, plus
a K-weighted LUFS meter fed to the UI over a ring buffer and per-bus gain automation keyframes.

### Canvas handles & alt-render monitor (issue #27)
On-canvas transform handles (corner/edge scale, rotate knob, a group box) replace numeric-field-only
transform editing, with canvas snap guides and viewer zoom/pan. Introduces the shared
`AltRenderState`/`AltRequest` async alt-render channel (GPU-accurate, never the CPU compositor,
paused whenever an export is running) that every later hover-preview/trim-view/Scopes/multicam
consumer builds on instead of its own render path.

### Export & delivery (issue #28)
One-click platform export-preset tiles (YouTube 1080p/4K, Shorts/Reels/TikTok 9:16, …), a render
queue with an ETA, Export In/Out range with letterbox, loudness normalisation, a render-in-place bake
pipeline (stabilize/denoise/slow-mo), and markers CSV / YouTube-chapters export and import.

### Layout modes & onboarding (issue #31)
`Settings.layout_mode` (Dynamic contextual vs. Granular explicit, chosen at first run and
toggleable later), named workspaces (Simple/Audio/Text/Deliver, `Alt+1`.. `6`), a per-panel pin/lock
so auto-surfacing can be opted out of per panel, a first-run welcome wizard and home screen, and an
adaptive tool strip.

### Inspector & Gallery (issue #33)
The inspector's clip-properties sections are now collapsible (`CollapsingState`, primary controls
first, fold state remembered in `Settings.inspector_folds`), with a new Color section for LUTs/
Primaries/Qualifier. `Pane::Presets` keeps its variant but now draws a tabbed Gallery (effects, node
graphs, adjustment layers, templates, LUTs, Looks, Titles, Captions) instead of the deleted
`presets_ui.rs`.

### Media library (issue #34)
Offline-media detection with a badge plus Relink/Consolidate, subclips and Smart Bins, sortable list
columns, thumbnail-viewport culling so large bins stay smooth, keyboard navigation, an empty-state
hint, and image-sequence import (with its own `import` `-- @on` hook call site).

### Source monitor (issue #32)
See "Source monitor" below — `Pane::Source` landed as part of this wave.

### Pro monitor, pro timeline & titles (issues #35, #36, #37)
Pro-monitor: a trim view (outgoing/incoming frames via two alt-render requests), dynamic trim,
Scopes, wipe compare, stills, and a 4-angle-capped multicam angle grid. Pro-timeline: asymmetric
multi-roller trims from the edit-point set, inline track header rename/colour/reorder, timeline view
presets, an overview minimap strip, and a Find window across clips/markers/text. Text-titles: a
Gallery ▸ Titles tab with a handful of built-in templates whose params are exposed as keyframeable
text animation (reveal/wave).

### Binary size, corrected (issue #18)
`size_log.csv`'s `bc4c19e` row corrects an earlier PR-body claim that the main-crate
`opt-level = "s"` step was skipped: the merged commit actually kept it (retroactively validated —
`headless_1000_clips_stays_fast` 0.93 ms, `bench_4k_preview` 29.0 ms/frame @720p / 74.7 ms/frame
@native-4K, no regression). Real measured delta: **15,970,816 B (15.23 MB)** at `aaa3fdc` →
**10,121,728 B (9.65 MB)** at `bc4c19e`, **-5,849,088 B (-5.58 MB)**. See the entry below for what
shipped in the diet itself, and goals.md for the post-feature-wave number.

The following entries were carried forward from this release's working `## unreleased` section
(PRs #12/#14, issues #16/#18/#21/#29/#32) rather than rewritten — they already match the density
this section aims for.

### Transcript & captions (issue #29)
- A transcribed clip's word timings now persist in the project (survive save/reopen) instead of
  dying when the Subtitles pane closed. A new collapsible **Transcript** section there: click a
  word to seek, drag-select a range and Delete to ripple-cut it (cues, markers and the transcript
  shift together), search across every transcribed clip, and an editable filler-word list
  ("um", "uh", "you know"…) with Mark-instead-first before it cuts.
- Karaoke-style captions: subtitles can now animate word-by-word against the transcript
  (Highlight / Pop / Typewriter), set from the new `subtitles.animation` script/MCP tool.
- "Get captions" is now a single button that names the exact download size before it fetches
  anything — never runs at startup.
- Any imported video/audio clip's right-click menu gained a **Transcript** submenu:
  Transcribe… (runs in the background, offers the model download first if needed), View
  transcript (a live, non-blocking word list with click-to-seek and a filter box), and Export
  transcript… (.txt / .srt / .json).
- Basic text-to-speech: a Speech panel in the Transcript section using Windows' own voices, which
  drops the result on the timeline as a linked audio clip.
- Scripts/agents can now drive all of this: `transcript.get/set/cut_words/remove_fillers/search/
  export`, `transcribe.run/install`, `tracking.run`, `subtitles.animation`, `tts.speak`, and
  `media.transcribe`/`media.transcript`.
### Source monitor (issue #32, source-monitor)
- `Pane::Source` is real: the old library preview (`lib_preview.rs`, deleted — it used to hijack the
  Preview pane) is now a dockable Source monitor (`ui/source_ui.rs` + `ui/app/source_pane.rs`) with its
  own `Player`, I/O marks (ticks + band on the shared `preview::scrub_bar`), a Source/Record focus
  button, a three-point / smart-edit button row (Splice, Overwrite, Append at End, Ripple Overwrite,
  Close Up, Place on Top + a nearest-cut readout), Subclip-from-marks and Source Tape (the bin laid end
  to end in a transient project — never touches the real project or its undo stack).
- Transport focus: Space/J/K/L/I/O/Home/End drive whichever of Source or Timeline was clicked last
  (the timeline is the fallback; a press on the Preview or Timeline pane hands focus back). The two
  players never run at once — starting the timeline pauses the source.
- `App::insert_at` is gone: every placement (drops, library "Add", voiceover/recording import,
  three-point edits) routes through `edit_ops::place`/`place_many` with an explicit `DropMode` —
  drop modifiers are live (none = Place, Ctrl = Splice, Alt = Overwrite, Shift = Place on Top), and
  an Alt-drop landing on a clip body is a Replace edit (`Project::replace_clip`: duration/effects/
  transform kept, linked audio swapped too), elsewhere a plain overwrite.
- New actions: Match Frame (F), Reveal in Library (Ctrl+Shift+R); Append at End / Ripple Overwrite /
  Close Up / Place on Top / Source Tape / Show-Hide Source Monitor (palette + Source pane buttons).
  `Project::subclip_from_marks`. MCP: `source.open/mark/get/focus/tape/insert/subclip`,
  `timeline.place`, `timeline.match_frame`, `timeline.smart_edit` (splice/overwrite/lift/extract/replace
  stay on trim-model's existing `timeline.*` tools — nothing re-registered).
- Tripwire count update: `pre_existing_repaint_sites_unchanged_and_named` drops from 16 to 15 —
  `lib_preview.rs`'s one raw `request_repaint_after` (the buffering spinner's 50 ms poll) is deleted with
  the file; `source_pane.rs` routes the same poll through `App::animate_until`, so this is a genuine
  migration of one of the tracked sites, not a relocation.
- Verification aid: `--screenshot` with `SE_SCREENSHOT_SOURCE=1` opens the project's first asset in the
  Source monitor with marks set before the shot (like `SE_SCREENSHOT_DELAY`).
- Deferred to same-day follow-ups per the plan's ownership notes: the `library.rs` edit (Source Tape
  filter toggle; Enter/double-click loads into Source) lands after media-library (#34); the timeline
  clip context-menu rows (Match Frame / Reveal in Library / Replace with Library Selection) after the
  wave-2 timeline owner merges — both verbs are already reachable by hotkey and palette.

### Forgiveness (issue #21)
- Toasts gained a kind (info/success/warn/error), an optional Undo button and an optional progress
  bar; `App::toast`/`toast_with_folder` (122 existing call sites) compile unchanged. Delete/Ripple
  Delete and Remove-unused-assets now toast an Undo.
- Every blocking Yes/No/Cancel dialog (6 sites: `confirm_discard`, both `act_overwrite` prompts,
  library's Clear recent/Remove unused, subtitles' Clear all/Import-replace) is now a non-blocking
  `egui::Window` (`src/ui/confirm.rs`) — nothing in the editor blocks the frame waiting on a click
  anymore. `confirm_discard() -> bool` is gone; `confirm_discard_then(on_yes)` is its non-blocking,
  continuation-based replacement.
- Debounced off-thread autosave (`Settings.autosave_secs`, 20 rolling backups per project under
  `%LOCALAPPDATA%\SimpleEditor\autosave`), a panic hook (`crash.log` + the latest autosave snapshot),
  and a non-blocking "Recover unsaved project?" offer on startup when a newer autosave exists.
  `App::fire_hook("project_open"/"project_save", ...)` gives `-- @on project_open`/`-- @on
  project_save` scripts a real call site (the other four `@on` events named in the plan are each a
  different workstream's own addition).
- Settings ▸ Performance gained a "Clear Caches" button + live on-disk cache size; History panel rows
  gained a Restore button (labeled "Restore history entry" in the undo stack).
- `Settings::load()` keeps its signature but now quarantines a corrupt `settings.json` to
  `settings.json.bad` instead of silently falling back to defaults over it; `App::new` toasts the
  reason via the new `Settings::load_reporting()`.
- Tripwire count update: `pre_existing_repaint_sites_unchanged_and_named` drops from 17 to 16 — the
  toast area's own `request_repaint_after` call moved out of `mod.rs` into the new `feedback.rs`
  (outside that test's scanned file list) as part of extracting the Toast type; the call itself still
  exists, just relocated, so the crate-wide site count is unchanged.

### Binary size (issue #18, size-diet)
- Dropped `egui_commonmark` (+ `egui_commonmark_backend`, `egui_extras`, `pulldown-cmark`) for a small
  in-house `ui::markdown` (~250 lines: headings, bold/italic/code spans, bullet lists, fenced code,
  links, `---` rules) — used by the Planner's Notes tab and the new "What's New" window.
- Dropped eframe's `default_fonts` feature (the embedded Hack/NotoEmoji/Ubuntu-Light/emoji-icon TTFs);
  `theme::fonts` now loads Segoe UI + Consolas straight from `%WINDIR%\Fonts`, falling back through
  Tahoma/Arial and Courier New/Lucida Console, and finally to `fontdb`'s system-font scan if none of
  those exist (never expected on Windows 10/11). Every headless UI test that builds a bare
  `egui::Context` now seeds it with `theme::test_fonts()` (`#[cfg(test)]`, zero release-binary cost) —
  without the Cargo feature, `egui::FontDefinitions::default()` is empty crate-wide, and several
  existing tests measure real text/button metrics.
- Dropped eframe's `persistence` feature (`ron`/`home`/egui's own memory serde); the window rect now
  lives in `Settings::window_rect`, written by a 500ms-debounced `winpos::tick` and seeded once at
  launch (`winpos::apply_rect`) from `main.rs`. Tradeoff: egui's own CollapsingHeader/scroll-position
  memory (e.g. the Planner's fold state) no longer survives a restart — accepted for wave 0, not
  rebuilt here (inspector fold state moves to `Settings` explicitly in wave-2 inspector-gallery).
- `[profile.release.package."*"] opt-level = "s"` for dependencies, plus the main-crate `opt-level =
  "s"` gate (kept, not reverted — see the "Binary size, corrected" entry above for the retroactive
  bench validation that confirms it shipped).
- Deleted verified-dead/duplicate code: `Tool::Zoom` (an unbound, always-no-op tool, including its
  `tool_drag` guard reference in `preview.rs`), `src/ui/presets_ui.rs` (270 lines — `Pane::Presets` now
  draws `library::reuse_ui`'s Effects/Node-graph/Adjustment-layer rows, the same ones `Pane::Library`'s
  Recent tab already used; Save-from-selection/rename/delete are gone from the UI until wave-2
  inspector-gallery's Gallery pane, restored meanwhile via the new `templates.save` MCP tool),
  `library.rs`'s `sequences_section` (zero call sites) and `templates_section` (test-only call site;
  its regression coverage moved to call `row()` directly), and the lib-preview mini-player's duplicated
  scrub-bar painting (now calls the one `preview::scrub_bar`, which returns a seek target instead of
  writing into `PreviewResponse` directly).
- New MCP tools: `help.changelog` (current version + this file's text, backing the What's New window)
  and `templates.save` (saves the given, or currently selected, clips as a reusable effect chain / node
  graph / clip template — keeps presets_ui.rs's deleted "Save from selection" capability scriptable).
- `scripts/size.ps1` (+ `size_log.csv`, `merge=union`) builds release and logs `<sha>,<bytes>,<note>`;
  `/se-verify` now runs it instead of a manual build-and-eyeball, and fails the pass on an unexplained
  >64 KB size growth. `--selftest` gained an `idle_repaint` step (a bare, idle egui frame requests no
  repaint) — smoke-level only, it cannot exercise the 17 pre-existing raw `ctx.request_repaint_after`
  call sites inside real panes' own live UI code (app.rs-descended files, `planner.rs`, `preview.rs`,
  `subtitles_ui.rs`) — those are a known, separately-tracked gap (`chore/migrate-repaint-timers`,
  unscheduled), not migrated by this PR. `App::animate_until` (added by #17/registries-schema-hooks) is
  therefore accurately the sanctioned path for **new** timed-repaint code from wave 0 onward, not yet a
  codebase-wide invariant — `winpos.rs`'s window-rect debounce and `whatsnew.rs`'s version-gate tick are
  its first two callers.
- Measured release exe: baseline `aaa3fdc` 15,970,816 B (15.23 MB) -> `bc4c19e` 10,121,728 B
  (9.65 MB), a delta of -5,849,088 B (-5.58 MB). See `size_log.csv`.

### Refactor (issue #16)
- Split the five largest files by responsibility, zero behaviour change: `model.rs` (6.1K lines) into
  `src/model/*.rs` (data types) + `src/model/ops/*.rs` (edit operations), `ui/app.rs` (6.8K lines)
  into `src/ui/app/*.rs`, `ui/timeline.rs` (4.3K lines) into `src/ui/timeline/*.rs`,
  `ui/settings_ui.rs` into `src/ui/settings_ui/*.rs`, and `ui/inspector.rs`'s clip-properties/text
  sections into `ui/inspector_audio.rs`/`ui/inspector_text.rs`. 657 tests unchanged, `--selftest`
  passes, release binary +46 KB (more per-file codegen boundaries after 5 files became 76 — not a
  behaviour change).

### Playback & performance (PR #14)
- **RAM-scaled playback cache**: `Settings::cache_mb` (Performance tab; 0 = automatic — ¼ of
  installed RAM, clamped 512 MB–4 GB) replaces the fixed 512 MB budget; lowering it frees RAM
  immediately. The prefetch horizon is sized from measured entry cost, so multi-layer GPU frames
  no longer oversubscribe it.
- **A finished proxy no longer restarts the read-ahead**: only the remapped source's spans (and
  decoders) are evicted — other clips keep their warm caches.
- **Decoded-source-frame cache** on the preview pool: scrubbing back past the composited cache is
  a memcpy instead of an ffmpeg re-seek/respawn.
- **Per-asset proxy status**: queued / building N % / ready — hourglass badge on library rows and
  tiles, a line in the inspector's Asset block, and the preview badge now names the file.

### Fixes (PR #12)
- Tool hotkeys match modifiers exactly (Shift+T/D/R work again); bare **M adds a marker at the
  playhead** (on the selected clip when under it), the Marker tool moved to Shift+M.
- "Use project background" on export actually works (CPU compositor honours the background;
  Overwrite-Original/MCP exports stay black unless opted in); background editable in project
  settings.
- History panel: layout rows can't desync the panel-undo stack, labels describe their own edit
  (derived lazily — no more per-gesture JSON parses), local-time day grouping, label cache
  invalidated on delete.
- Bulk edit clamps propagated keyframes into each sibling and never severs expression/path links;
  scale X/Y fully integrated (trim/split-safe keys, curve editor rows, ranges).
- Marker ruler scoped per sequence; snap-to-clip uses nearest edge; single-marker snap/link;
  row click seeks.
- Timer ticks (and notifies) with its tab hidden; moodboard tags typeable with one undo per
  gesture, in-app drags import, Import… button; text spans follow edits, style-paste clamps
  ranges, Clear Style on Selection; per-encoder quality scales (VP9/AV1 0–63, QSV floor 1) with
  honest warning bands; live shape preview uses the real tool style; Settings opens 900×700.

## beta-0.2.0

Nine PRs since beta-0.1.0.

### Playback & performance
- Deep read-ahead (~1.5 s prefetch horizon, ~0.5 s eviction-protected trail behind the playhead) and
  a real **buffering** state — the clock and audio pause together, a spinner shows, and playback
  resumes once the read-ahead refills, instead of audio racing ahead of frozen video.
- Background **proxy media**: an all-intra 720p proxy per imported clip, so scrubbing and reverse
  playback are as cheap as forward; exports always use the originals.
- Selective cache invalidation (an edit only evicts the frame ranges it can actually change), a
  DecoderPool LRU cap, and pre-opening the next clip's decoder before a cut.
- Fixed the library asset preview being permanently black (its canvas was never sized before the
  first frame arrived).
- Fixed space and timeline clicks fighting over playback focus: Space now controls whichever player
  owns the Preview pane, and clicking the timeline reclaims it from an open library preview.
- Fixed a bug where a focused numeric field could silently record/alter keyframes at the moving
  playhead when playback started.

### Library preview
- A hover-only click/drag scrub bar on the docked and fullscreen preview (previously only the
  library preview had one).
- A heartbeat-monitor style audio visualizer for audio-only files with no video and no cover art — a
  fixed EKG-blip silhouette that pulses with the current amplitude, reusing the same waveform peak
  data the timeline already computes. An embedded cover art image takes precedence when present, and
  it can be toggled from the preview's right-click menu.
- The library preview now works for files never imported into the project (Global/Recent), and gained
  the main preview's quality selector (100/75/50/25%) and buffering spinner. A still image previewed
  from the library shows just the picture — no transport controls.

### Editing
- **Edge transitions**: a transition can start/end a clip with nothing on the other side, blending
  from/to empty on both the CPU and GPU render paths.
- A transition's Start/End/Last/Next position selector, video opacity fades (not just audio), and
  transitions that select/multi-edit like clips on the timeline.
- **Live keyframe links** (After Effects–style): any keyframable property can follow a saved
  path or a Luau expression instead of being hand-keyed, re-evaluating from its source every frame.
  The expression field gets Luau syntax highlighting.
- One-click **aspect ratio / project-format presets** (YouTube, TikTok, Reels, Shorts, Square,
  Portrait, 4K…), quality/FPS quick buttons, user-saved project templates, and a toggleable
  **social-guide overlay** (platform safe zones and UI silhouettes) over the preview.

### Subtitles
- Regenerate cues from cached word timings without re-transcribing; a subtitle lane on the timeline
  (select, trim, split, resize like any clip); convert cues to editable Text clips; bulk panel actions;
  a whisper `--prompt` field; bold/italic/align/spacing/shadow and continuation-mark styling.

### UI & theming
- Cozy/Sharp look toggle (rounded corners and softened borders vs. the old flat look).
- 25 built-in theme presets, exportable/importable `.sedit-theme` files, per-clip-kind colour
  overrides, tab context menus (pop out/hide/set icon), layout presets, and an editor background
  image with tint/blur/opacity.
- No more white flash on startup (the window is created hidden and shown after its first frame).

## beta-0.1.0

Initial public beta — see the [release notes](https://github.com/KashTheKing/simple-editor/releases/tag/beta-0.1.0).
