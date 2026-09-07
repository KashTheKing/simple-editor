# Changelog

## unreleased

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
- `[profile.release.package."*"] opt-level = "s"` for dependencies. <MAIN_CRATE_OPT_LEVEL_NOTE>
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
- Measured release exe: baseline `<BEFORE_SHA>` <BEFORE_BYTES> B (<BEFORE_MB> MB) -> `<AFTER_SHA>`
  <AFTER_BYTES> B (<AFTER_MB> MB), a delta of <DELTA_BYTES> B (<DELTA_MB> MB). See `size_log.csv`.

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
