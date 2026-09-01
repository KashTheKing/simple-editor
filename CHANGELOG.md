# Changelog

## unreleased

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
