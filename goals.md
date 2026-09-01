# Goals — Simple Editor

Root priorities for the project, a running log of what's already achieved, and UX principles to
design against. Agents append here via `/se-goal` (record only) or `/goal` (record + implement).
Users can also just edit this file or tell an agent to. Read this before proposing anything that
trades size/deps/speed for a feature, or before designing a UI panel.

## Core goals (the non-negotiables)

- **Small binary.** Target ~10 MB release exe. Currently **12.97 MB** after adding mlua/Luau
  (flagged, over budget) — see [simple-editor-stack memory] for history. Any change that grows
  the binary should say by how much.
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
- [x] Luau scripting embedded (`editor.tool()` bridges into the MCP tool catalogue)

## In progress / open

- [ ] Get release binary back under ~10 MB (currently 12.97 MB)
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
