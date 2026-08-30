# Changelog

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
