# Simple Editor: "simple surface, pro depth" UI overhaul + docs site

_Approved 2026-09-27. Pre-flight landed 2026-09-28 (#71, #72, #62 regenerated on main, #74 = the library-toolbar WIP; its split_zone part was dropped as superseded by #73). One GitHub issue per workstream, label `simplify`._

## Context

The engine and features are strong (191 Actions, 236 MCP tools, trims, color, audio, captions,
multicam…), but the 2026-09 overhaul bolted every feature onto the screen:
- 19 panes and 6 top-right "workspaces" that only swap panel arrangements (`layout.rs:35`,
  `layout_ctl.rs:160`). Switching changes nothing but the tree and throws away your customisation.
- A 15-button Tools pane (`tools.rs:598`) that eats a whole row of the Preview column.
- 4 icon buttons on every pane header (`layout.rs:984`) that repeat the tab's right-click menu.
- Toolbars of 10–20 inline buttons per pane (Library, Subtitles, Markers, Curves, Transitions,
  Effects stack with 11 buttons per effect, Mixer, Planner rows).
- Duplicated surfaces: the timeline's own "Main" strip under the Timeline tab
  (`timeline/mod.rs:636`); Library bottom preview vs the Source monitor; Transitions pane vs the
  Gallery's Transitions tab; Effects pane's applied stack vs the Inspector's Effects section;
  Transitions "Set type/easing" vs the Inspector vs right-click; Inspector "Open sequence" /
  "Markers" / "Open node editor" / Save / Export buttons; menu-bar timecode vs transport timecode;
  12 pane-toggle Actions + 19 "Show X" palette rows + View checkboxes.
- Glyphs that read wrong: Pin = umbrella (`tools.rs:1874`), Hourglass = 5 meanings, Mask = a
  "contrast" half-circle, Wrench = Tools. The Inspector's `?` buttons are a missing glyph: `∿` isn't
  in Segoe UI (`inspector.rs:1499`, `theme.rs:289`); `✕ ◀ ▶` have the same risk.
- The 4-arrow cursor over every tab bar: egui_tiles hard-codes `CursorIcon::Grab` on the empty tab
  strip (`egui_tiles-0.14.1/src/container/tabs.rs:269`) and winit maps Grab to IDC_SIZEALL.
- Bugs you hit (record from playhead, Multicam window, nested-sequence audio, lists that don't
  scroll, panels that switch on when opened) plus ~12 more found while mapping hotkeys.

**Goal:** Premiere / Resolve / CapCut-grade layout, dead simple on the surface: the primary action
visible, everything else one right-click, keystroke or Ctrl+K away. **No functionality deleted.**
Then a Docusaurus docs site on GitHub Pages with a screenshot tutorial of every part of the app.

**Your decisions (2026-09-27):**
- Top tabs become **Resolve-style pages: Edit · Color · Audio · Export**, each with its own default
  layout that you can still rearrange, add panels to, move and undock.
- Panels are fixed + resizable by default, and still undockable/movable when you want.
- Niche features (Planner, Moodboard, Tracking, Nodes, Curves, Multicam, Screen recorder, History,
  social guides) are **hidden, not deleted**: Window menu, right-click menus, Ctrl+K.
- Tutorial: Docusaurus site on GitHub Pages.

## Design rules (every workstream)

1. **One home per function.** Each verb has one visible home; everything else reaches it through
   its Action (menu, right-click, hotkey, palette). Delete duplicate *surfaces*, never the Action or
   MCP tool.
2. **A pane header is one row at most**: search/filter/view plus the single most common verb.
   Everything else goes in that surface's right-click menu.
3. **Every surface has a right-click menu** built from Actions through one shared helper, so
   labels, icons, shortcut text and enabled state match everywhere.
4. **Opening a panel never changes behaviour or data.** Panels activate on use.
5. **Anything that can overflow scrolls.**
6. **Native first:** Windows' own icon font replaces hand-drawn glyphs (0 bytes in the exe); no new
   crates; egui 0.33 stays pinned.
7. Repo rules stay: a worktree per workstream, ToolDef parity tests, `assert_no_idle_repaint_*`
   per new pane, `scripts/size.ps1`, and a real screenshot for every UI change.

## Target screens

```
┌ File Edit Clip Timeline Playback Window Help ───── [ Edit | Color | Audio | Export ] ─────────┐
│ Library │ Effects │ Transitions │ Gallery  + │ Source │ Preview               + │ Inspector   + │
│ [search…] [Filter▾] [View▾] [+ Import]      │ ┃↖┃                              │ ● Clip name ◉ │
│  media list / grid                          │ ┃⌗┃      viewer                  │ V1 · 0:00 · 52s│
│  (right-click for everything else)          │ ┃T┃                              │ ▸ Transform    │
│                                             │ ┃◻┃                              │ ▸ Opacity      │
│                                             │ 00:00:12:04  ⏮ ◀ ▶ ▶ ⏭   Fit ▾  ⛶ │ ▸ Speed / Audio│
├─────────────────────────────────────────────┴──────────────────────────────────┴───────────────┤
│ Main │ Sequence 1 ×  [↖ Select|✂ Blade|⇔ Rate] [magnet][link]              ────○──── zoom  + │
│ V1 🔒 👁 │ ███ clip ███████████                                                                 │
│ A1 🔒 🔊 │ ▂▃▅ audio ▅▃▂                                                                        │
└──────────────────────────────────────────────────────────────────────────────────────────────┘
```
- **Edit:** as above. Hidden tabs behind Library: Subtitles, Markers, Auto-cut, Planner, Moodboard,
  History, Tracking, Jobs.
- **Color:** Gallery (Looks/LUTs) | Preview | Scopes on top; a thin Timeline; Inspector (Color
  section first) | Nodes · Curves on the bottom.
- **Audio:** small Preview | Mixer · Subtitles | Inspector (Audio section first) on top; Timeline
  with tall audio tracks below.
- **Export:** Export settings | Preview | Jobs (render queue) on top; Timeline with In/Out below.
- Each page remembers its own arrangement; right-click a page tab to reset it.
- Pane header: tabs plus one small **`+`** (add a hidden panel to this group). Tab right-click:
  Maximise (`` ` `` or double-click), Undock, Close, "Stay on this tab" (today's pin), Set icon ▸.
- Panels are locked by default (tabs click, they don't drag); Window ▸ Layout ▸ **Unlock panels**
  turns drag-to-redock on. Undock always works from the tab's right-click.

## Pre-flight (main tree, serial, needs your OK)

1. Land open PR **#62** (em-dash text; touches strings everywhere, including `library.rs`) first.
2. **Uncommitted WIP on `main`** (Library More/Filter/View/Actions menus, `split_zone` default
   off, Source moved into the Preview tab group). The files were edited today 21:37–21:49, so make
   sure no other session is still writing them. Then move the work to `fix/library-toolbar-wip`,
   run `cargo test`, open a PR and merge.
3. Merge **#71** (job-completion hitches) and **#72** (Jobs pane, which becomes the Export page's
   queue). Remove stale worktrees (`library-preview-progress-bar` is 151 behind;
   `auto-cut-selection-markers` already landed as #69).
4. Commit this plan as `plans/simplify/README.md` plus `plans/simplify/issues/<ws>.md`, and file
   GitHub issues labelled `simplify` + `wave-N`.

## Wave 0 (three serial PRs)

### 0a `bugfixes` (first, because you're hitting these now)
1. **Record from playhead.** The checkbox is never read (`capture_ui.rs:34,246`; `jobs.rs:86`
   always plays). Pass it into `start_voiceover`, and seek + play only when ticked. Don't rewind to
   0 when recording starts at the timeline end (`playback.rs:388`). Wire up Retake (`voice_start`
   is never set, `resp.retake` never read). The Draw tool's Record calls `seek` +
   `player.play()` directly instead of queueing PlayPause, which a focused Source monitor steals
   (`actions.rs:654`). Place the take from ffmpeg's first sample, not process spawn
   (`capture.rs:186`).
2. **Multicam Angles window.** `open` is re-created `true` every frame (`tools_monitor.rs:251`), so
   keep a dismissed-clip id. Only open it on request (Clip ▸ Multicam angles…) and only for ≥ 2
   angles. Today every nested sequence counts as multicam.
3. **Nested sequences' audio sits on the video track** (`model/ops/sequences.rs:102` makes only a
   V clip; Nest also pulls the A-track clips out). `insert_sequence_clip`, which drop, MCP, Nest and
   multicam all use, adds a linked companion Sequence clip on an audio track (audio-only nests get
   only the A clip). Bump `Project::VERSION` 2→3 and migrate old projects in `from_json` (tracks,
   sequences, `main_stash`). The mixer rule then becomes "Sequence clips sound only on audio tracks"
   (`engine/mixer.rs:153,173`). The Inspector audio section and the clip menu's Bus submenu check
   the track kind locally (`timeline/mod.rs:1127`); `is_visual()` is not rewritten (32 callers).
   `unnest` (`trim.rs:464`) removes the companion and accepts either id. Cache is safe
   (`video_dirty_spans` skips audio tracks) and export uses the same mixer.
4. **Panels that switch on when opened:**
   - Mixer creates a "Main" bus while drawing (`mixer_ui.rs:128`), silently switching the whole mix
     to the bus graph with no undo. Show a placeholder strip until the first bus edit.
   - Auto-cut starts detecting on open (`autocut_ui.rs:59` `active: true`). Default to false;
     Detect turns it on.
   - The tracking box grabs drags at the canvas centre whenever its pane is visible
     (`preview.rs:1205`). Only after "Place box"; poll the job from a frame hook so a hidden tab
     doesn't stall it.
   - Subtitles ▸ Transcribe only finishes while the section is open (`subtitles_ui.rs:801`). Route
     it through `App::transcribe_clip(…, gen_cues)`, which the always-on `transcript_ctl::tick`
     finishes.
   - A hidden Source monitor keeps Space/JKL (`source_pane.rs:53`). Add a generic "pane drawn last
     frame" flag set (generalising `tracking_shown`/`autocut_shown`, `mod.rs:1041`) and require it.
     0c reuses the same flags for the Scopes readback gate.
5. **Lists that don't scroll:** Auto-cut, Subtitles (upper sections squeeze the cue list),
   Transitions, Tracking, Settings tabs, Export window, Settings ▸ Appearance ▸ Icons
   (`max_height` collapses to 0). Wrap them in `ScrollArea`.
6. **Keyboard bugs** (they span `hotkeys.rs`, `tools.rs` and `app/mod.rs`, so they land here, not
   in wave 1):
   - Bare-key bindings match modifiers exactly (today Shift+K = Stop and Shift+C picks Cut,
     `hotkeys.rs:428`).
   - Snap: ToggleSnap default N → S, delete `handle_snap_hotkey` and its RESERVED rows.
   - AddShape loses its dead Shift+S.
   - Backspace aliases to Delete only when the hovered pane isn't Curves, Nodes or Subtitles
     (`mod.rs:1317`).
   - DuplicateClips joins `is_late` so the node editor's Ctrl+D works.
   - Delete works on a selected transition (`mod.rs:1625` also checks `sel_transitions`).

### 0b `ui-kit`
Owns `theme.rs`, the `tools.rs` Glyph section (enum/ALL/name/draw_glyph/action_glyph),
`ui/mod.rs`, `app/menus.rs` `menu_item`, `app/jobs.rs`, `layout.rs` (pane-rect and tab-bar-rect
recording only), `main.rs`, a new `app/tools_uikit.rs`.
- **Native icons.** Add font family `icons` = `SegoeIcons.ttf` → `segmdl2.ttf` → Segoe UI (the
  last fallback is required: an empty family panics, `epaint fonts.rs:808`). Append `seguisym.ttf`
  to Proportional so `∿ ✕ ◀ ▶ ◆` render. `Glyph::icon() -> Option<char>`; `draw_glyph` paints the
  char and the vector bodies it replaces are deleted. Vector stays only for roll/slip cursors,
  `Poly(n)` and `Letter`. Give distinct symbols to Pin, Rate-stretch, Mask, History, Recent and
  Proxy. Near-duplicate variants map to the same char, not merged (unknown override names would
  silently become "no icon", `layout.rs:86`). Menus use `Button::new((RichText icon, label))`
  instead of the space-padding hack (`menus.rs:16-31`). Test: every mapped codepoint exists
  (`Fonts::has_glyph`). Review a rendered glyph-sheet PNG by eye; nudge with `FontTweak.y_offset`.
- **Cursor.** Record each tab-bar rect in `top_bar_right_ui`. After `tree.ui()`, turn `Grab` back
  into `Default` only when the pointer is inside one of them and nothing is being dragged
  (`// ponytail:` egui_tiles tabs.rs:269 hard-codes it; upgrade path is an egui_tiles bump).
  (`visuals.interact_cursor` is dead in egui 0.33, so it isn't used.)
- **One menu helper, no context plumbing.** `ui::menu::action_item/action_menu` read a
  thread-local snapshot (shortcut text, icon overrides, disabled set) published once per frame by
  `App::update`, and push Actions onto a thread-local queue that `update()` drains. This is the
  existing `inspector.rs:105` / `confirm.rs:54` pattern, so no wave-1 context struct changes.
  `App::menu_item` becomes a wrapper. Long menus scroll (`max_height`).
- **Automation**, used by every later PR and by the docs:
  - MCP `ui.screenshot {path, pane?}` reuses `jobs.rs:157-190` + `mcp::png_encode`, crops to the
    recorded pane rect, and tags the request with `UserData` so it doesn't trigger the
    `--screenshot` app exit.
  - MCP `ui.input {events:[click|rclick|move|key|scroll]}` queues events drained one per frame by
    `App::raw_input_hook`, sets `RawInput.modifiers`, and replies via `ToolOutcome::Job`.
  - A `--size WxH` CLI flag.
- **Freeze after 0b:** the `Tool` enum, `glyph_text_button`, `ui::menu` signatures.

### 0c `pages`
Owns `layout.rs`, `app/layout_ctl.rs`, `app/menus.rs`, `onboarding.rs`, `home.rs`, `settings.rs`,
`app/tools_layout.rs`, `app/boot.rs`, `app/mod.rs` (layout load, registries, markers),
`app/panes.rs` (Tools/Presets arms), `app/windows.rs`, `app/tools_monitor.rs` +
`app/monitor.rs` (scopes gate), `scopes_ui.rs` + `export_ui.rs` (split window bodies into plain
body fns), `keymaps.rs`, `goals.md`.
- **Pages.** `PAGES = [Edit, Color, Audio, Export]` with four builders; delete the
  Simple/Colorist/Text/Deliver/Fast-cut builders. `Settings.layout` stays the *current* page's
  tree; `Settings.page_layouts: BTreeMap<String, String>` holds the others. On switch: unmaximise,
  stash the old tree, clear `layout.undo/redo`, and strip `LAYOUT_STEP` entries from the app undo
  stack (`actions.rs:48`) so Ctrl+Z can't replay another page's tree. Popped windows are per page.
  One-time migration: save the old tree as a layout profile named "Before update", start from the
  new Edit default, and map the old workspace name to a page. `Workspace1..4` become the page
  Actions (ids kept so rebinds survive; 5 and 6 removed, since `Hotkeys::from_settings` already
  ignores unknown ids). `layout.workspace` keeps accepting old names; add `layout.page`.
- **Page switcher** centred in the menu bar with text labels; right-click → Reset page layout.
- **Pane chrome.** `top_bar_right_ui` draws only `+`. Tab right-click as listed above.
  Double-click a tab maximises. Tabs use `Sense::click` (`layout.rs:875`) unless "Unlock panels" is
  on.
- **Sequence tabs in the Timeline's tab.** The Timeline's `tab_ui` paints "Main" plus the sequence
  being edited (the model has one `project.editing`) with ×. Clicks return through Behaviour
  vectors like hide/pop. A popped-out Timeline shows the sequence in its window title.
- **Panes.** Add `Pane::Scopes` and `Pane::Export` through `PANE_DRAWERS` using the split body
  fns. Scopes readback runs only while the Scopes pane was drawn last frame. Ctrl+E opens the Export
  page. Remove `Pane::Tools` from `Pane::ALL`/`ROUND3`, strip its tile on load, and delete
  `layout.reveal(Pane::Tools)` (`mod.rs:1225`, `panes.rs:79`) and the dead Presets arm
  (`panes.rs:299-356`). Add `assert_no_idle_repaint_scopes/export` tests and ToolDef rows.
- **Layout mode leaves the UI.** Remove the View radios, the Ctrl+Shift+G default and the
  onboarding Dynamic/Classic card. Settings ▸ General gets "Switch panel tabs to follow the
  selection" (same field). Record this change against goals.md:159-161 in the same PR.
- **Menu bar:**
  - **File**: New, Open…, Open Recent ▸, Save, Save As… · Import ▸ (Media…, Timeline XML/EDL…,
    From URL…) · Export ▸ (Video… Ctrl+E, Quick Export Ctrl+M, Frame…, Premiere/Resolve XML…,
    Lossless cut…, Overwrite original…, Markers…, Style summary…) · Record ▸ (Voiceover…,
    Screen…) · Settings… · Exit.
  - **Edit**: Undo, Redo · Cut, Copy, Paste, Paste Special ▸ (Insert, On new top track, First free
    track, Attributes…), Copy Attributes · Delete, Ripple Delete · Select All, Deselect, Select ▸
    (Forward, Backward, Under playhead) · Find…
  - **Clip**: Split, Duplicate, Enable, Link · Speed/Retime…, Freeze frame · Add ▸ (Text, Shape,
    Adjustment layer, Mask, Subtitle, Container) · Transition ▸ (At cut, Last used, At end) · Nest,
    Un-nest, Create multicam, Multicam angles… · Match frame, Reveal in Library · Save as
    template…, Flow motion.
  - **Timeline**: Mark ▸ (In, Out, Clip, Clear, Go to In/Out) · Lift, Extract, Trim to In/Out,
    Close gap · Trim ▸ (edit point, ±1, ±10, extend, top, tail, slip) · Add marker · Auto ▸
    (Auto-cut silence…, Scene cuts, Detect beats, Split at beats, Remove fillers,
    Transcribe/Captions, Auto duck, Normalize, Match loudness, Auto colour, Colour match) · Tracks ▸
    · Zoom in/out/fit · Snapping ✓ · Render selection, Render in place.
  - **Playback**: Play/Pause, J/K/L, Play In→Out, Loop, Play around, Step ±1/±10, Prev/Next cut,
    Start/End · Proxies ✓, Movie mode ✓, Playback resolution ▸, Fullscreen.
  - **Window**: Pages ▸ · every panel as a checkbox (main panels first, niche ones under "More ▸")
    · Scopes · Social guides ▸ · Layout ▸ (Unlock panels, Reset page, Save/Load/Export/Import
    profile) · Maximise panel · Scripts ▸ (only when scripts exist).
  - **Help**: Tutorial & docs (opens the site), Keyboard shortcuts F1, Command palette Ctrl+K,
    What's new, Show welcome, version/ffmpeg lines.
  - The menu-bar timecode and "editing: Main > X" breadcrumb go (transport and sequence tabs show
    them). home/onboarding hard-coded shortcut strings read `hotkeys.text(a)`.
- **Pre-seed `// ---- ws:<name> ----` markers** for the six wave-1 workstreams in the `actions!`
  macro, `group()`, the five registries, `Settings` + `Default`, and `App`. New Actions, settings
  and App fields in wave 1 go only inside the owner's marker section.

## Wave 1 (six concurrent worktrees, after 0c)

Each workstream converts its own hard-coded shortcut strings to `hotkeys.text(a)` and uses
`ui::menu` for every right-click menu. `panes.rs`: a workstream edits only its own panes' arms.

**1. `timeline-surface`**: owns `timeline/*`, `app/timeline_pane.rs`, `app/trim_actions.rs`.
- Delete the inner "Main" strip and the "Detailed ▾ / Overview" row. One toolbar row: segmented
  [Select V · Blade C · Rate stretch R] · Snap · Linked selection · spacer · zoom slider. View
  presets and Overview move to ruler right-click ▸ View ▸.
- Track header: name, lock, eye/mute. Solo shows on hover; colour, sync/ripple, magnetic and rename
  are in the header right-click (already there). Double-click the name to rename.
- Right-click menus regrouped (the clip menu is 32 flat items today): Cut/Copy/Paste/Delete/Ripple
  delete · Split, Duplicate, Enable, Link · Speed ▸ · Add ▸ (Marker, Transition at start/end, Mask)
  · Audio ▸ · Nest/Container ▸ · Transcript ▸ · Label ▸ · Match frame, Reveal in Library · Save as
  template. New menus: gap (Close gap, Paste), seam (Roll, Add transition here), ruler (Mark
  In/Out, Clear, Add marker, View ▸, Back to Main), empty subtitle lane.
- Collapse the timeline's `Act` enum duplicates (Split, Delete, Link, Enable, AddMarker, AddTrack,
  track flags) into the existing Actions, so each has one code path. `RippleDeleteInOut` and
  Extract share `trim_actions::act`.
- Fixes: remove the mislabeled "Convert to Adjustment Layer" (it adds a new layer); track Actions
  act on the track under the cursor, not the first selected clip's (`trim_actions.rs:110`).

**2. `viewer-surface`**: owns `preview.rs`, `app/preview_pane.rs`, `source_ui.rs`,
`app/source_pane.rs`, `app/source_ctl.rs`, `multicam_ui.rs`, and `tools.rs` outside the Glyph
section (`Tool` variants frozen).
- **Tools pane → viewer tool rail**: a slim vertical icon rail on the Preview's left edge with
  Select, Crop, Text, Shape ▾ (flyout: rect, ellipse, triangle, polygon, star, line, arrow), Draw
  and Mask ▾. Fill/stroke/width/sides float at the top of the viewer only while a shape/draw tool
  is active. Blade and Rate stretch live on the timeline toolbar; Marker is `M`; Spacer is
  palette/hotkey only.
- **Transport, one row**: timecode (click to type) · ⏮ ◀| ▶ |▶ ⏭ · Zoom ▾ (Fit/50/100/200) · ⛶.
  Stop (= K), In/Out/Clear (I/O/Alt+X), the "100 %" quality dropdown, movie mode, Proxy and Guides
  move to right-click.
- **Viewer right-click**: over a clip, Reset transform, Fit/Fill, Crop handles, Add mask, Add text
  here; always, Mark In/Out, Playback resolution ▸, Proxies ✓, Movie mode ✓, Guides ▸, Scopes,
  Background ▸, Snap to canvas ✓.
- **Source monitor**: same transport plus Insert/Overwrite; Append, Ripple overwrite, Close up,
  Place on top, Source tape and Subclip move to right-click.

**3. `library-surface`**: owns `library.rs`, `app/library_pane.rs`, `app/media_sync.rs`.
- One header row: search · Filter ▾ · View ▾ · `+ Import`. "Imported | Global" becomes a small
  "Project | Browse" switch. New ▾, Import URL, Link folder, Remove unused, More ▾, Consolidate and
  the selection strip move to right-click: **empty area** (new: Import…, Import from URL…, New
  folder, New sequence, New adjustment layer, Link folder…, Remove unused (N), Consolidate…, Sort ▸,
  Columns ▸) and **item** (existing menu + Open in Source, Rename, Info…). Column header
  right-click = Columns ▸.
- Delete the bottom inline preview; the Source monitor is the one preview. Description and tags move
  to right-click ▸ Info…. Single-click shows in Source, double-click plays.
- Fixes: Space over the Library plays the Source instead of adding clips (`library.rs:237`);
  sequences can be renamed and deleted (`LibOp::SeqRename/SeqDelete` have no UI).

**4. `inspector-surface`**: owns `inspector.rs`, `inspector_audio.rs`, `inspector_text.rs`,
`color_ui.rs`, `effects_ui.rs`, the keyframe button in `ui/mod.rs`, `curves.rs`, `autocut_ui.rs`,
`retime.rs`, and the Inspector/Effects/Curves/Auto-cut arms of `panes.rs`.
- Header: label dot · editable name · enable toggle, then one muted line "V1 · start · duration".
- Sections collapsed except the first: Transform (Position; Scale with a link toggle that reveals
  X/Y; Rotation), Opacity & blend (+ fades), Speed, Audio, Color, Effects (the one place the applied
  stack is edited), Text, Shape, Mask, Path. On the Color/Audio pages the matching section opens
  first.
- Removed duplicates: Open sequence (double-click the clip), Open node editor (Effects right-click),
  the Markers section (Markers pane / M), the read-only Transitions list, project-level
  Save/Export/Export Frame buttons (File menu), the Retime button (right-click the speed field).
- Keyframes: one ◆ per property. "✕ keys", "N keys" and the ∿ link menu move to the diamond's
  right-click (Clear, Link to path/expression, Prev/Next key). Inspector and Effects share one
  keyframe widget.
- Effects stack row: enable, name, drag handle. Mask, shader, copy, paste, up, down and remove move
  to right-click. The Effects **pane** becomes catalogue-only.
- Curves pane: three header rows become one (target ▾ · Motion ▾ · ⋯ menu).
- Auto-cut is opened from Timeline ▸ Auto ▸ and the clip right-click, not shown by default.

**5. `side-panels`**: owns `transitions_ui.rs`, `gallery.rs`, `app/gallery_ctl.rs`,
`markers_ui.rs`, `mixer_ui.rs`, `subtitles_ui.rs`, `transcript_ui.rs`, `app/transcript_ctl.rs`,
`planner.rs`, `moodboard_ui.rs`, `history_ui.rs`, `tracking_ui.rs`, `capture_ui.rs`, and those
panes' `panes.rs` arms.
- Transitions: catalogue + default duration only. Set type/easing, Add at start/end and the
  per-transition rows move to the Inspector (transition selected) and right-click. The Gallery's
  duplicate Transitions tab goes.
- Gallery: card right-click (Apply to selection, Preview); fix the double-nested ScrollArea so the
  tabs stay put.
- Markers: one row (label filter · search) + list. Everything else goes to row right-click
  (rename, label, delete, snap to nearest clip, link to clip) and empty-area right-click (Add at
  playhead, Add on selected clip, Copy as list, Export ▸, Import…). Remove "Go" (clicking already
  seeks).
- Mixer: strip = name, meter, fader, pan, M, S. A trailing `+` strip adds a bus. Routing, mono,
  output and filters go to strip right-click; filter ▲▼ become drag/right-click.
- Subtitles: one row [+ Add · Transcribe… · Style ▾ · ⋯]. Import, Export SRT/VTT, Burn in, To text
  clips, Delete in range and Clear all go in the ⋯ menu and list right-click; cue-row ▶/Split/T/✕
  go to right-click. Transcript becomes a tab inside the pane.
- Planner/Moodboard/History/Tracking are hidden by default; their inline row buttons move to
  right-click.

**6. `keys-actions`**: owns `hotkeys.rs` (inside its marker section plus `group()`), `keymaps.rs`,
`cheatsheet.rs`, `palette.rs`, `app/palette_ctl.rs`, `settings_ui/*`, `app/actions.rs`,
`app/playback_ctl.rs`.
- Every Action gets a real group (93 are in "Other" today), so F1 reads well. F1 also lists the
  out-of-table keys and mouse gestures (the timeline modifier table).
- Palette: drop duplicate Toggle/Show rows, humanise raw tool names, honour icon overrides; the
  cheat sheet and welcome toast stop hard-coding "Ctrl+K".
- Keymap presets stop leaving QuickExport, Overwrite, PlayToOut and AddTransition unbound.
- Wire or remove `UndoSettings`; remove the dead `ToggleSource` branch.

## Wave 2: `docs-site` (after wave 1 merges)
- `website/` Docusaurus (classic preset, docs-only, `baseUrl: /simple-editor/`), `node_modules`
  git-ignored. `.github/workflows/docs.yml` builds and deploys to GitHub Pages on push to `main`
  (you enable Pages → "GitHub Actions" in repo settings).
- **Scripted screenshots:** `scripts/docs-shots.ps1` launches a clean profile (`APPDATA` and
  `LOCALAPPDATA` pointed at scratch, `onboarded: true`, `last_seen_version` set, `mcp_enabled` on a
  non-default port, `--size 1600x900`). It opens a demo project built from `--selftest` media and
  drives each state over MCP (`layout.page`, `selection.set`, `ui.action`, `ui.input` for
  right-click menus), then `ui.screenshot` into `website/static/img/tutorial/*.png`. Re-run it
  whenever the UI changes.
- **Pages:** Getting started · The interface (pages, panels, moving/undocking/resetting) · Library
  & importing · Viewer & playback (JKL, marks, tool rail, on-canvas transform) · Timeline (tools,
  cutting, trimming with the modifier table, snapping, tracks, sequences, markers) · Inspector &
  keyframes · Effects, transitions & titles · Captions & transcript · Color page · Audio page ·
  Export page · Right-click reference (every surface) · Keyboard shortcuts (generated from
  `hotkeys.get`) · Advanced (auto-cut, multicam, tracking, nodes, curves, planner) · Scripting & MCP
  (from `docs/customizing.md`).
- Help ▸ Tutorial & docs links to the site; README gets a fresh screenshot and a docs link;
  CHANGELOG, goals.md, notes.md and ARCHITECTURE.md are updated.

## Size
Roughly neutral. Deleting ~1,000 lines of vector glyphs, the Tools strip layout, the Library inline
preview and the toolbars offsets the new menus, pages and two pane drawers. Icon fonts load from
`%WINDIR%\Fonts`. Every PR runs `scripts/size.ps1`; more than +64 KB needs a reason line.

## Verification (every PR)
1. `cargo test`, plus new tests:
   - each page's layout round-trips through a switch, and undo can't replay another page's tree;
   - `+` adds a hidden pane; a sequence-tab click switches `project.editing`;
   - nested-sequence insert creates a linked A clip, only one copy is audible, muting the A track
     silences it and muting V doesn't, a v2 project gets its companion on load, and undoing after
     deleting the companion keeps it deleted;
   - `from_playhead` placement; the multicam window stays closed;
   - drawing Mixer/Auto-cut/Tracking/Subtitles leaves `Project::to_json` unchanged;
   - bare-key exact-modifier matching; every mapped icon codepoint exists;
   - `assert_no_idle_repaint_scopes/export`;
   - `every_edit_op_has_a_tool`, `ui_action_covers_every_action` and `reserved_chords_are_free`
     stay green.
2. `cargo run -- --selftest`.
3. Look at real screenshots of the changed surface: `ui.screenshot` from 0b on (context menus via
   `ui.input` rclick), or `--screenshot` → PNG before that.
4. `scripts/size.ps1 -Note <ws>`.
5. Live smoke over MCP (`/se-coedit`): switch pages, rearrange/unlock/undock/reset, right-click
   every surface, record a voiceover from the playhead, nest a clip with audio and mute its audio
   track, open and close the Multicam window.

Docs: `npm run build` in `website/` passes with no broken links, the Pages workflow deploys, and
you click through the published site in the browser pane.

**Needs your explicit OK when we get there:** merging the pre-flight PRs, pushing branches and
opening PRs, and enabling GitHub Pages.
