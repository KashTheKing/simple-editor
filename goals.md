# Goals — Simple Editor

Root priorities for the project, a running log of what's already achieved, and UX principles to
design against. Agents append here via `/se-goal` (record only) or `/goal` (record + implement).
Users can also just edit this file or tell an agent to. Read this before proposing anything that
trades size/deps/speed for a feature, or before designing a UI panel.

## Core goals (the non-negotiables)

- **Small binary.** Target ~10 MB release exe. Measured pre-size-diet baseline: **15.23 MB**
  (15,970,816 B, sha `aaa3fdc`, measured fresh on main — supersedes any older, smaller reading from
  before `egui_commonmark` landed). Post-size-diet (issue #18, corrected once the main-crate
  `opt-level="s"` gate was confirmed kept, not reverted): **9.65 MB** (10,121,728 B, sha `bc4c19e`),
  a measured delta of **-5.58 MB**. Wave 1-3 feature work (23 workstreams) then added size back: the
  final measured exe at wave-3-complete is **11.60 MB** (12,167,168 B, sha `86c1793` — see
  `size_log.csv`'s last row and ARCHITECTURE.md's header). Any change that grows the binary should
  say by how much.
- **Few dependencies.** Every new crate is a liability (compile time, binary size, supply chain,
  MSRV drift). Prefer stdlib, the `windows` crate, or shelling out to `ffmpeg.exe` over a new
  dependency. mlua (Luau scripting) is the one deliberately-approved exception so far.
- **Instant open.** No splash screens, no blocking network calls on startup, no visible white
  flash (already fixed — window created hidden, shown after first frame). Idle CPU should be
  ~0%.
- **Smooth, intuitive UI.** See "UX principles" below — this is a first-class goal, not
  secondary to features.
- **Deep user customizability.** Themes, icon overrides, layout presets, keybindings, Luau
  scripting hooks — customization should be a first-class extension point, not bolted on.

## Achieved (don't re-litigate these)

Pulled forward from CHANGELOG.md — mark new completions here as they land.

- [x] MF hardware/software decode with ffmpeg.exe fallback for playback/export/images
- [x] Deep read-ahead + buffering state (clock/audio pause together, spinner, no desync)
- [x] Background proxy media (all-intra 720p) so scrub/reverse playback is cheap
- [x] Selective cache invalidation (edits only evict the frame ranges they can change)
- [x] Edge transitions, transition position selector, video opacity fades
- [x] Live keyframe links (After Effects–style, Luau-expression driven)
- [x] One-click aspect ratio/format presets, project templates, social-guide overlay
- [x] Subtitle lane on timeline (trim/split/resize like a clip), cue regen without re-transcribe
- [x] Cozy/Sharp theme toggle, 25 built-in themes, exportable `.sedit-theme`, per-clip-kind colors
- [x] No white flash on startup
- [x] Live co-editing MCP server (Streamable HTTP, toggled in Settings)
- [x] RAM-scaled playback cache (`cache_mb`, auto = ¼ RAM) + read-ahead costed from real entry sizes
- [x] Proxy swaps evict only their source's spans (decoders + frames); decoded-source-frame cache
- [x] Per-asset proxy status (queued / building N% / ready) in library, inspector and preview badge
- [x] Luau scripting embedded (`editor.tool()` bridges into the MCP tool catalogue)
- [x] **God-file split + registry protocol** (issues #16, #17): `app.rs`/`model.rs`/`timeline.rs`
  split into `src/ui/app/*.rs` (58 files) / `src/model/*.rs` + `src/model/ops/*.rs` / `src/ui/timeline/*.rs`
  (9 files), zero behaviour change; five dispatch registries (`TOOL_TABLES`/`ACT_HANDLERS`/
  `FRAME_HOOKS`/`WINDOW_DRAWERS`/`PANE_DRAWERS`) + marker-section protocol let 23 workstreams add
  files without ever sharing a merge hunk. See ARCHITECTURE.md's "Registries" section.
- [x] **Command palette & scripting** (issue #20, command-palette): `Ctrl+K` palette over every
  Action/Pane/arg-free ToolDef/script/workspace, `F1` cheat sheet, Avid/Premiere/Resolve keymap
  presets, Luau `-- @on <event>` script-hook headers.
- [x] **Forgiveness** (issue #21, forgiveness): non-blocking toasts with Undo actions replace every
  blocking Yes/No dialog, debounced off-thread autosave + crash recovery offer, cache-clear/size in
  Settings, History panel restore.
- [x] **Trim & gesture primitives** (issues #22, #23, #30, snap-engine/trim-model/timeline-trim-gestures):
  tiered `snap_target` with a pre-release guide line; ripple/roll/slip/slide/segment/multi-roller
  trim primitives and per-track locked/ripple/magnetic flags; the frozen `arm()` modifier table
  wired live for Edge and Drop gestures. See ARCHITECTURE.md's "Gesture model" section.
- [x] **Color engine** (issue #24, color-engine): `Lut`/`Primaries`/`Qualifier`/`FrameBlend` effect
  kinds, a stdlib `.cube` parser, builtin Looks, master/source-clip effect chains via `Asset.effects`.
- [x] **Player rate/loop** (issue #25, player-rate-loop): `Clock.rate`/`loop_range` parameterise the
  playback clock — JKL shuttle, Loop In→Out, Play In→Out, ±10-frame trims.
- [x] **Audio analysis & DSP** (issues #19, #26, audio-analysis/audio-dsp-automation): beat/onset/BPM
  detection, peak/RMS/LUFS analysis, cross-correlation sync offset, AutoDuck/Normalize/one-click
  repair chains, `AudioRole` tagging, DeHum/Limiter/DeEsser filters, K-weighted LUFS metering,
  per-bus volume automation.
- [x] **Canvas handles & alt-render monitor** (issue #27, canvas-handles-monitor): on-canvas
  transform/crop/rotate handles, canvas snap guides, the shared async `AltRenderState`/`AltRequest`
  alt-render channel every hover-preview/trim-view/Scopes/multicam consumer now uses.
- [x] **Export & delivery** (issue #28, export-deliver): platform export-preset tiles, a render
  queue with ETA, Export In/Out range + letterbox, loudness normalisation, a render-in-place bake
  pipeline (stabilize/denoise/slow-mo), markers CSV/YouTube-chapters export+import.
- [x] **Layout modes & onboarding** (issue #31, layout-modes-onboarding): `Settings.layout_mode`
  (Dynamic contextual vs. Granular explicit), named workspaces (Alt+1..6), per-panel pin/lock, a
  first-run welcome wizard/home screen, an adaptive tool strip.
- [x] **Source monitor** (issue #32, source-monitor): `Pane::Source` is real — a dockable two-up
  monitor with its own Player, in/out marks, three-point/smart-edit editing, Subclip-from-marks,
  Source Tape.
- [x] **Inspector & Gallery** (issue #33, inspector-gallery): collapsible, primary-first inspector
  sections with fold state in `Settings.inspector_folds`; a Color section; the re-purposed
  `Pane::Presets` now draws a tabbed Gallery (effects/node-graphs/LUTs/Looks/Titles/Captions).
- [x] **Media library** (issue #34, media-library): offline detection/badge, Relink/Consolidate,
  subclips + Smart Bins, sortable list columns, thumbnail-viewport culling, image-sequence import.
- [x] **Transcript & captions** (issue #29, transcript-captions): a persisted per-clip transcript
  survives save/reopen, click-to-seek/drag-select-and-cut, filler-word removal, karaoke-style caption
  animation, Windows-voices text-to-speech.
- [x] **Pro monitor, pro timeline & titles** (issues #35, #36, #37, pro-monitor/pro-timeline/text-titles):
  trim view + dynamic trim, Scopes, wipe compare, stills, multicam angle grid; asymmetric
  multi-roller trims, track header rename/colour/reorder, view presets, an overview strip, a Find
  window; Gallery ▸ Titles templates with keyframeable reveal/wave text animation.
- [x] **MCP tool catalogue growth**: 236 tools at wave-3-complete (up from ~50 pre-overhaul), every
  one traceable to a `ToolDef` row via `every_edit_op_has_a_tool`; see `docs/customizing.md`.

## In progress / open

- [x] wave-0c size-diet (issue #18) landed: release exe 15.23 MB -> 9.65 MB (-5.58 MB measured,
  correcting an earlier PR-body report that mistakenly said the main-crate `opt-level="s"` gate was
  skipped — `size_log.csv`'s `bc4c19e` row confirms it was kept, validated against
  `bench_4k_preview`/`headless_1000_clips_stays_fast`). Wave 1-3 feature work added back 2,045,440 B
  (+1.95 MB) across 23 workstreams, landing at 11.60 MB — still over the ~10 MB target. No
  `chore/se-engine-split` follow-up was needed since the opt-level gate held; getting back under
  ~10 MB stays an open goal, to be traded off against future feature work, not addressed by
  docs-refresh (docs-only, Δ exe ≈ 0 KB).
- [x] **UI/UX overhaul** — 23 workstreams in 4 waves, planned 2026-09-04 in
  [plans/ui-overhaul/README.md](plans/ui-overhaul/README.md) (one issue-ready file per workstream
  under `plans/ui-overhaul/issues/`; GitHub issues #16–#38, labels `ui-overhaul` + `wave-N`). Waves
  0-3 (all 22 non-docs workstreams) are merged into main; wave 4 (docs-refresh, #38, this PR) is the
  serial closer documenting what actually shipped. All six originally-planned Luau `-- @on` hook
  events (`selection_changed`, `import`, `export_done`, `project_open`, `project_save`,
  `marker_added`) landed real `fire_hook` call sites — verified by grep against the merged tree, not
  assumed from the plan; see ARCHITECTURE.md's "Registries" section and `docs/customizing.md` for
  the owning file:line of each.
- [ ] (add more as they're identified — via `/se-goal` or `/goal`)

## UX principles

Source: user-collected design research on video-editor UX, condensed here (full transcript in
`C:\Users\Kash\Downloads\simple-editor-plans\UX Design Tips for Video Editors.md`). The
one-line takeaway: **Progressive Fluidity** — a clean, automated surface for a beginner, with
professional depth exactly one gesture or keystroke away. Never force a choice between "simple"
and "powerful."

**General intuitiveness**
- Use existing mental models (standard icons, Space to play/pause, Ctrl+Z). Don't reinvent
  conventions users already have.
- Progressive disclosure: show only what's needed for the immediate task; advanced controls are
  one click deeper, not gone.
- High forgiveness: non-destructive editing, cheap undo/redo, autosave, warn-before-destroy
  instead of block-before-destroy.
- Every interaction gets immediate feedback (cursor state, hover state, progress with an ETA).
- Strict visual hierarchy so the eye lands on the primary action first.

**Timeline specifically**
- Hierarchical, predictable snapping: playhead > cursor > selected clip edges > adjacent clips >
  markers — with a visible snap-guide line *before* release, not a surprise after. This is the
  #1 complaint about DaVinci Resolve; don't reproduce it.
- Snap magnetism should weaken as you zoom in, so single-frame precision doesn't fight the magnet.
- Primary track can be magnetic/ripple-friendly; secondary tracks (B-roll, music, SFX) default
  to position-locked so a main-track edit never silently desyncs downstream audio (CapCut's
  biggest complaint).
- Fast waveform/thumbnail generation — cut decisions should be visual, not scrub-to-guess.
- Avoid Avid-style manual track patching; infer target track from what's under the cursor.

**Contextual UI / inspector**
- One contextual inspector that shows controls relevant to the current selection (select audio →
  gain/EQ/pan; select text → typography) instead of a wall of always-visible panels.
- Let panels that are docked-but-hidden auto-surface their tab on selection instead of jumping
  panels around the screen (avoids disorienting power users' muscle memory).
- Every panel gets a pin/lock so a user who wants a static layout can opt out of auto-switching
  per-panel, not just globally.
- Offer both a "Dynamic Contextual" layout mode and a "Granular Explicit" (classic multi-panel)
  mode, chosen at first run and toggleable later (this project already leans this way with
  layout presets — keep extending it, don't regress toward one fixed layout).

**Direct manipulation**
- Prefer dragging on the preview canvas (move/scale/crop/rotate with visible bounding boxes)
  over numeric X/Y/scale fields in a side panel. Numeric fields can still exist for precision,
  but dragging should always work first.
- Contextual cursor zones on a clip (top = move, bottom = split, edges = trim, corners = fade)
  reduce tool-swapping fatigue (Select/Razor/Slip hotkey juggling).
- Hover previews on transitions/LUTs/presets before committing.

**Keyboard-first, mouse-friendly**
- J-K-L shuttle should work (standard industry muscle memory).
- A command palette (Ctrl+K) that can execute any action by name — don't make features
  discoverable only through menu-hunting.
- Keybindings must be remappable and have a visual reference (this project already has a
  keybindings/hotkeys system — keep it discoverable, not buried).

**AI/automation as co-pilot, not black box**
- Automate tedious work (captioning, silence removal, color match, proxy generation) but always
  expose the result as editable keyframes/nodes/text — never an opaque one-shot result the user
  can't adjust.

## Things to avoid at all costs

- Erratic/unpredictable snapping with no visual cue (top DaVinci Resolve complaint).
- A main-track edit silently breaking downstream audio/B-roll sync (top CapCut complaint).
- Timeline/scrub lag from missing waveform caching or no proxy pipeline (top Premiere complaint).
- Forcing modal tool-switching for routine trim/cut/select operations (top Avid complaint).
- Panels that jump around the screen based on selection (disorients power users).
- Adding a dependency, or bumping egui, to solve a problem stdlib/existing deps already cover.
