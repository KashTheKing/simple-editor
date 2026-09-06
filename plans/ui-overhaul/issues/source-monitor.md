# Source monitor: Pane::Source, three-point editing, smart edits, Source Tape

**Workstream:** `source-monitor` · **Issue:** [#32](https://github.com/KashTheKing/simple-editor/issues/32) · **Wave:** 2 · **Branch/worktree:** `feat/source-monitor` → `../simple-editor-wt/source-monitor` · **Depends on:** trim-model, player-rate-loop, registries-schema-hooks · **~1120 new lines · Δ exe ≈ +70 KB**

Read [agents.md](../../../agents.md), [goals.md](../../../goals.md), [notes.md](../../../notes.md) and the master plan [README.md](../README.md) (registry protocol, keymap, modifier table) before starting.

## Summary

Promote the ad-hoc library preview (App.lib_preview/start_lib_preview/draw_lib_preview/lib_preview_frame, app.rs:78-87,235-240,4380-4621, ~240 lines) into a dockable Pane::Source with its own Player, src_in/src_out marks, and Space/JKL routing while focused. Implement App::place_asset(asset, at, track, DropMode) as the single-asset funnel over trim-model's ranged primitives, plus App::place_assets(ids, at, track, DropMode) — a thin chaining wrapper (advances `at` to each placed clip's end, mirroring insert_at's own loop) — for the multi-id call sites insert_at actually serves (app.rs:2456,2554,2800,2815,3735,4271; verified by direct grep — all six pass Vec<Id>, three from library multi-select). DropMode::Overwrite branches to replace_clip when the drop lands fully within one existing clip's span (per the skeleton's own modifier table: "Overwrite (on a clip body: Replace edit keeping duration/effects/transform)"), else falls back to overwrite_in — calling trim-model's Project::replace_clip fn directly, not through a duplicate MCP tool. Add three-point edit verbs, Match Frame (F) + Reveal in Library, smart-edit buttons (Append at End / Ripple Overwrite / Close Up / Place on Top) with a nearest-cut indicator, subclip-from-marks, and Source Tape (a transient Project built from the filtered library bin). Delete lib_preview.rs's ~240 lines once Pane::Source replaces it.

## Motivation

Closes gap-matrix rows 42 (three-point/source marks), 43 (match frame + reveal in library), 49 (smart edit buttons + indicator), 107 (Source Tape), 36 (splice from source), 86 (subclips from marks UI half); realizes architecture hook 7 (source monitor state + dockable two-up) and critique fix 0 (Replace edit). Delivers the Avid/Premiere/Resolve source-side workflow entirely through modifiers/buttons/hotkeys (no new Tool), with every verb reachable via MCP per the non-negotiable AI-tooling requirement — reusing trim-model's already-registered timeline.splice/overwrite/lift/extract/replace tools rather than re-declaring them.

## In scope

- Pane::Source drawer: own Player, transport, scrub bar with I/O tick marks, Space/JKL routing while focused
- src_in/src_out marks; three-point splice/overwrite/lift/extract/append/place-on-top over trim-model's ranged ops (calling Project:: fns directly, not via new duplicate ToolDefs)
- App::place_asset(asset, at, track, DropMode) single-id impl + App::place_assets(ids, at, track, DropMode) chaining wrapper, replacing insert_at + all its call sites
- DropMode::Overwrite on a single-clip-body drop invokes replace_clip (keeps duration/effects/transform); elsewhere overwrite_in
- Match Frame (F), Reveal in Library (Ctrl+Shift+R)
- Smart-edit buttons (Append at End, Ripple Overwrite, Close Up, Place on Top) + nearest-cut smart indicator
- Subclip from marks (Project::add_subclip call site)
- Source Tape: transient Project over the filtered library bin, played through the same SourceState.player
- MCP tools for the verbs this workstream actually owns (source.*, timeline.place, timeline.match_frame, timeline.smart_edit); Luau exposure via editor.tool
- Delete src/ui/app/lib_preview.rs (superseded), delete the old draw_lib_preview override branch in the Preview pane arm

## Out of scope

- Trim/roll/slip/slide gestures on the timeline (timeline-trim-gestures, wave 2)
- Track lock/ripple/magnetic flags themselves (trim-model, wave 1 — this WS only calls locked_of/ripple_tracks)
- Canvas transform handles, hover-preview alt-render channel (canvas-handles-monitor, wave 2)
- Library columns, Smart Bins persistence, offline-media badges (media-library, wave 2)
- Multicam, trim view, scopes (pro-monitor, wave 3)
- New Tool button of any kind (non-modal principle)
- General multi-asset chain placement beyond mirroring insert_at's existing behaviour (place_assets is a straight port, not a redesign)
- New MCP ToolDef rows for splice/overwrite/lift/extract/replace_clip — trim-model already registers timeline.splice/overwrite/lift/extract/replace; this workstream's code calls those Project:: fns directly

## Files

| Op | Path | What |
|---|---|---|
| create | src/ui/source_ui.rs | SourceState, show()/SourceResponse, scrub-bar tick marks over preview::scrub_bar, transport, smart-edit button row |
| create | src/ui/app/source_pane.rs | PANE_DRAWERS entry for Pane::Source: owns SourceState, calls source_ui::show, wires Space/JKL when focused |
| create | src/ui/app/source_ctl.rs | act_source(Action)->bool: MatchFrame, RevealInLibrary, AppendAtEnd, RippleOverwrite, CloseUp, PlaceOnTop, SourceTape, mark-in/out routing; smart_indicator() |
| modify | src/ui/app/edit_ops.rs | fill the 0b DropMode/place_asset stub: place_asset(asset: Id, ..) single-id impl calling trim-model's splice_in/overwrite_in/replace_clip/insert_asset_clips_ranged directly; place_assets(ids: &[Id], ..) chaining wrapper that calls place_asset per id and advances `at` to each result's end (mirrors insert_at's `t = c.end()` loop, app.rs:1106-1112, confirmed by grep); DropMode::Overwrite branches to replace_clip when the drop point falls fully within one existing clip's span on the target track, else overwrite_in; delete insert_at, redirect its 6 call sites — app.rs:2456,3735,4271 (single Vec<Id> from one import/recording) and app.rs:2554,2800,2815 (library multi-select, library.rs sel_ids) — confirmed present at exactly these lines by grep — all to place_assets(.., DropMode::Place) |
| create | src/ui/app/tools_source.rs | pub const TOOLS: &[ToolDef] — source.* verbs (open/mark/get/focus/tape/insert/subclip) plus timeline.place/match_frame/smart_edit ONLY. timeline.splice/overwrite/lift/extract/replace are already registered by trim-model's tools_trim.rs (rows 36-38 of its own feature list) — this file's act_source/place_asset call those Project:: fns directly instead of re-declaring ToolDef rows for them, avoiding the duplicate-tool-name build failure tool_names_unique_and_namespaced would otherwise catch |
| delete | src/ui/app/lib_preview.rs | LibPreview struct + start_lib_preview/lib_preview_frame/draw_lib_preview (app.rs:78-87,235-240,4380-4621 as split into this file by wave 0a) — fully superseded by SourceState |
| modify | src/ui/app/panes.rs | Pane::Preview arm: delete the lib_preview override branch (former app.rs:2277-2278); Pane::Library arm: replace insert_at(resp.add_to_timeline,...) at former app.rs:2554/2800/2815 with place_assets(&resp.add_to_timeline,..,DropMode::Place); single-click preview (former app.rs:2549-2550) now calls App::open_in_source instead of start_lib_preview. COORDINATION (mutually confirmed with canvas-handles-monitor's plan, resolving the audited asymmetric-ownership finding): source-monitor is the sole wave-2 owner of the Pane::Preview and Pane::Library arms it edits here (it is deleting the lib_preview branch and must control both arms atomically). canvas-handles-monitor's plan states the identical resolution verbatim (its own changelog: 'reconcile: accepted source-monitor's stated resolution for the panes.rs conflict verbatim'): canvas-handles-monitor's one-line Pane::Effects mask_target set (self.preview.mask_target = MaskTarget::Effect(i), alongside the existing mask_for wiring at app.rs:2697) is not part of either workstream's wave-2 panes.rs edit — it lands as a same-day follow-up PR rebased onto this workstream's merged panes.rs, never a concurrent same-file co-edit. |
| modify | src/ui/app/drops.rs | handle_drops' insert_at calls (former app.rs:2456,3735,4271 sites) become place_assets(&ids,..,DropMode) with DropMode from modifiers (Ctrl=Splice, Alt=Overwrite, Shift=OnTop, none=Place) |
| modify | src/model/ops/assets.rs | add subclip_from_marks(asset, in, out, name) thin wrapper over the 0b add_subclip stub, used by source_ctl |
| modify | src/ui/library.rs | single-click preview path (LibraryResponse.preview) now targets Pane::Source instead of the deleted lib_preview draw path; add a 'Source Tape' toggle on the filter bar returning LibraryResponse.tape_filter: Option<Vec<Id>>. COORDINATION: media-library is library.rs's exclusive wave-2 owner per registry_protocol (its own files[] lists 'src/ui/library.rs (wave 2 onward)'); this narrow edit lands as a same-day follow-up PR rebased onto media-library's merged library.rs changes, not a concurrent same-PR co-edit — resolves the audited same-wave library.rs conflict |
| modify | src/ui/app/mod.rs | register this workstream's own pre-seeded ws:source-monitor marker lines only: mod source_ui; mod source_pane; mod source_ctl; mod tools_source; one row each in PANE_DRAWERS (Pane::Source), ACT_HANDLERS (source_ctl::act_source), TOOL_TABLES (tools_source::TOOLS) — no other section touched |

## Model changes

- Project::add_subclip(asset: Id, in: f64, out: f64, name: Option<String>) -> Id — already a wave-0b schema-first stub per skeleton; this WS gives it a real body (new Asset row referencing the parent + range) and calls it from subclip_from_marks
- No new Track/Clip fields — three-point ops reuse trim-model's Project::{splice_in, overwrite_range, overwrite_in, lift_range, extract_range, replace_clip, append_at_end, insert_asset_clips_ranged} verbatim, called directly rather than through duplicate MCP wrappers

## Engine changes

- None — Source Tape and the source Player are UI-side (an extra playback::Player instance over a synthetic Project, same pattern as today's LibPreview, no new decode path)

## UI changes

- New Pane::Source (already pre-declared by wave 0b in Pane::ALL/glyph/title/preset builders per skeleton decision 'New panes'): two-up dockable, tab-stacked hidden behind Library in the Simple workspace, never in ROUND3
- Source pane: transport row (Play/Pause/Stop/step, matches LibPreview's existing button row app.rs:4465-4489), scrub bar with I/O tick marks (extends preview::scrub_bar, preview.rs:182, with mark rendering), smart-edit button row (4 glyph buttons + smart-indicator readout), Source/Record toggle in the pane's own tab area
- Library: single click still previews (now opens Pane::Source and surfaces it via App::surface, per contextual-ui's reveal_auto), double-click/Enter still adds via place_assets
- Clip context menu (timeline): 'Match Frame', 'Reveal in Library', 'Replace with Library Selection' entries (bound as menu-only per skeleton keymap notes — chords land where noted below)

## New types and functions

- `pub struct SourceState { pub player: Player, pub asset: Option<Id>, pub path: PathBuf, pub duration: f64, pub fps: f64, pub has_video: bool, pub is_image: bool, pub src_in: Option<f64>, pub src_out: Option<f64>, pub tape: Option<(Project, Vec<Id>)>, heartbeat: crate::ui::heartbeat::Heartbeat }` — src/ui/source_ui.rs: Replaces LibPreview 1:1 (same fields as app.rs:78-87) plus marks + optional tape project
- `pub struct SourceCtx<'a> { pub palette: &'a Palette, pub settings: &'a mut Settings, pub thumbs: &'a mut ThumbCache, pub focused: bool }` — src/ui/source_ui.rs: Borrow-split params mirroring draw_lib_preview's field destructure at app.rs:4533
- `pub struct SourceResponse { pub actions: Vec<Action>, pub insert: Option<DropMode>, pub settings_changed: bool, pub close: bool }` — src/ui/source_ui.rs: Button clicks bubble intents up instead of mutating App directly (mirrors LibraryResponse's shape)
- `pub fn show(ui: &mut egui::Ui, st: &mut SourceState, c: SourceCtx) -> SourceResponse` — src/ui/source_ui.rs: Draws transport/scrub/marks/smart-edit row; body ports draw_lib_preview's transport+scrub logic (app.rs:4452-4601) plus mark ticks and smart buttons
- `impl App { pub(crate) fn open_in_source(&mut self, ctx: &egui::Context, path: PathBuf); fn source_frame(&mut self, ctx: &egui::Context) -> Option<library::PreviewFrame> }` — src/ui/app/source_pane.rs: 1:1 port of start_lib_preview (app.rs:4380-4407) and lib_preview_frame (app.rs:4412-4433), renamed onto SourceState; called once per frame from the PANE_DRAWERS entry, mirroring the app.rs:6071 tick
- `impl App { pub(crate) fn place_asset(&mut self, asset: Id, at: f64, track: Option<usize>, mode: DropMode) -> Vec<Id> }` — src/ui/app/edit_ops.rs: Place=insert_asset_clips_ranged(None); Splice=splice_in (honours ripple_tracks); Overwrite: if the drop point falls fully within one existing clip's span on the target track, calls replace_clip (keeps duration/effects/transform) — otherwise overwrite_in; OnTop=insert on a freshly added track above every existing track of that kind. Pushes one undo, calls after_edit. Calls trim-model's Project:: fns directly, not through a duplicate ToolDef. Fills the 0b stub the skeleton pre-declares in edit_ops.rs
- `impl App { pub(crate) fn place_assets(&mut self, ids: &[Id], at: f64, track: Option<usize>, mode: DropMode) -> Vec<Id> }` — src/ui/app/edit_ops.rs: Multi-id chaining wrapper: calls place_asset per id, advancing `at` to each placed clip's end — reproduces insert_at's own loop (app.rs:1106-1112, confirmed by grep) verbatim. All 6 former insert_at call sites (single-file drops import multiple streams, and library multi-select genuinely selects several assets) route here, not through place_asset alone
- `impl App { fn act_source(&mut self, a: Action) -> bool }` — src/ui/app/source_ctl.rs: MatchFrame: clip under playhead -> open_in_source(asset.path) + seek(src_time); RevealInLibrary: select+scroll library to asset; AppendAtEnd: place_asset(source.asset, project.duration(), track_of_kind, Place); RippleOverwrite: overwrite_range then splice remainder; CloseUp: extract_range(gap) on the track under playhead; PlaceOnTop: place_asset(.., OnTop); SourceTape: builds Project via source_tape()
- `impl App { fn smart_indicator(&self) -> Option<f64> /* nearest edit-point/cut to the playhead within a UI-pixel threshold */ }` — src/ui/app/source_ctl.rs: Read-only helper painted next to the smart-edit buttons; no mutation
- `pub fn source_tape(assets: &[Id], project: &Project) -> (Project, Vec<Id>) /* filtered bin's clips appended end-to-end on V1/A1 of a synthetic Project at the parent's format; second value = per-clip start offsets for cut ticks */` — src/ui/app/source_ctl.rs: Transient project fed to SourceState.player.set_project; released (dropped) when the tape toggle turns off
- `impl Project { pub fn subclip_from_marks(&mut self, asset: Id, in_t: f64, out_t: f64, name: Option<String>) -> Id }` — src/model/ops/assets.rs: Thin call into the 0b add_subclip stub with the range from SourceState.src_in/src_out
- `pub enum DropMode { Place, Splice, Overwrite, OnTop }` — src/ui/app/edit_ops.rs: Pre-declared empty by wave 0b per skeleton; this WS is its sole real implementor

## Actions and hotkeys

| Action id | Label | Chord | Note |
|---|---|---|---|
| match_frame | Match Frame | F | skeleton keymap: free (F11=fullscreen, Ctrl+Shift+F=export frame). Clip under playhead -> open_in_source at its src time |
| reveal_in_library | Reveal in Library | Ctrl+Shift+R | skeleton keymap: free (Ctrl+R retime, Shift+R freeze, Ctrl+Alt+R voiceover). Selects + scrolls to the clip's asset |
| append_at_end | Append at End |  | smart-edit button only, per skeleton |
| ripple_overwrite | Ripple Overwrite |  | smart-edit button only |
| close_up | Close Up |  | smart-edit button only |
| place_on_top | Place on Top |  | smart-edit button / Shift-drop, per skeleton drop modifier table |
| source_tape | Source Tape |  | Source pane toggle only |
| splice | Splice (Insert) at Playhead | Shift+V | owned/registered by trim-model; source-side splice button dispatches the same Action (bare V is ToolSelect today, hotkeys.rs:144, so Shift+V is free) — no duplicate row added here |
| overwrite | Overwrite at Playhead | B | same as above: trim-model owns the Action row; source button reuses it. hotkeys.rs:73 has only Ctrl+B (Split) today, bare B free |
| toggle_source | Show / Hide Source Monitor |  | View menu / palette only, per skeleton (Ctrl+Num row full) |

## New glyphs

- Glyph::Append — smart-edit 'Append at End' icon
- Glyph::CloseUp — smart-edit 'Close Up' icon
- Glyph::PlaceOnTop — smart-edit 'Place on Top' icon
- Glyph::SourceRecord — Source/Record toggle in the pane tab
- Glyph::Tape — Source Tape toggle

## MCP tools (required — every capability must be scriptable)

| Tool | Kind | Args | Description | Maps to |
|---|---|---|---|---|
| source.open | ui | asset_id:integer:false:, path:string:false:one of asset_id/path required | Open an asset (or file path) in the source monitor | App::open_in_source |
| source.mark | ui | in:number:false:, out:number:false: | Set src_in/src_out on the open source clip (omit a field to clear it) | SourceState.src_in/src_out — kind:ui: UI-only state that never enters Project JSON, so it never triggers run_tool_undoable's snapshot/diff/undo path; per mcp_parity's own scheme (Mutate = project JSON changes, Ui = everything else) it belongs with source.open/source.focus, not Mutate |
| source.get | read |  | Current source: asset id, path, duration, fps, marks, playhead | SourceState fields |
| source.focus | ui |  | Give the source monitor keyboard focus (so Space/JKL route there) | surface(Pane::Source) + focus flag |
| source.tape | ui | asset_ids:array:false:filtered bin; omitted = current library filter | Build/refresh Source Tape from a set of assets | source_tape() + SourceState.tape swap — kind:ui: building a transient Project from already-loaded asset metadata is fast synchronous work (no decode, no progress to report), same shape as source.open, not a McpJob |
| source.insert | mutate | mode:string:true:place\|splice\|overwrite\|top\|append, at:number:false:defaults to playhead, track:integer:false: | Insert the open source clip's marked range into the timeline | App::place_asset with the open source's asset+range |
| source.subclip | mutate | name:string:false: | Create a Library subclip asset from the current in/out marks | Project::subclip_from_marks |
| timeline.place | mutate | asset_id:integer:true:, at:number:true:, track:integer:false:, mode:string:false:place\|splice\|overwrite\|top, in:number:false:, out:number:false: | General-purpose placement funnel (superset of timeline.add_clip for asset drops) — the only new timeline.* tool this workstream registers | App::place_asset |
| timeline.match_frame | ui | clip_id:integer:false:defaults to clip under playhead | Open the clip's source asset in the source monitor at the matching source time | act_source(MatchFrame) |
| timeline.smart_edit | mutate | kind:string:true:append\|ripple_overwrite\|close_up\|place_on_top | Run one of the four smart-edit compositions at the playhead | act_source dispatch |

**Luau:** Every tool above is callable from Luau via editor.tool('source.insert', {mode='splice', at=...}) etc. (mechanical — same bridge every ToolDef gets). Splice/overwrite/lift/extract/replace_clip stay reachable under trim-model's existing timeline.splice/overwrite/lift/extract/replace names — this workstream adds no duplicate. No new -- @on hook needed; source.mark/source.tape are ui-state tools so scripts polling source.get() see marks immediately, matching the existing editor.tool() contract.

## Tests

| Test | File | Asserts |
|---|---|---|
| place_asset_place_matches_insert_at | src/ui/app/edit_ops.rs | DropMode::Place produces identical clip layout to the old insert_at for a single multi-stream asset (regression pin before deleting insert_at) |
| place_assets_chains_multiple_ids_like_insert_at | src/ui/app/edit_ops.rs | place_assets(&[a,b,c], t, ..) advances the placement point to each result's end exactly like insert_at's `t = c.end()` loop — covers the library multi-select and multi-stream-import call sites |
| place_asset_splice_ripples_only_flagged_tracks | src/ui/app/edit_ops.rs | Splice on a project with one ripple track and one locked track shifts only the ripple track's downstream clips |
| place_asset_overwrite_on_clip_body_replaces | src/ui/app/edit_ops.rs | Overwrite whose drop point falls fully within one existing clip's span calls replace_clip (duration/effects/transform unchanged); a drop spanning a gap or multiple clips falls back to overwrite_in |
| place_asset_on_top_adds_new_track | src/ui/app/edit_ops.rs | OnTop always creates a track above the highest existing track of that kind, never reuses one |
| match_frame_seeks_source_time | src/ui/app/source_ctl.rs | act_source(MatchFrame) on a clip trimmed mid-asset opens the correct asset and seeks to clip.src_time(playhead), not 0 |
| smart_edit_append_at_end_uses_project_duration | src/ui/app/source_ctl.rs | AppendAtEnd places at project.duration() on the track matching the source asset's kind |
| smart_edit_close_up_extracts_only_under_playhead_track | src/ui/app/source_ctl.rs | CloseUp calls extract_range scoped to the track under the pointer, not every track |
| smart_indicator_finds_nearest_cut | src/ui/app/source_ctl.rs | smart_indicator returns the nearer of the previous/next edit point within threshold, None beyond it |
| subclip_from_marks_creates_ranged_asset | src/model/ops/assets.rs | subclip records the parent asset id and [in,out) range, duration matches out-in |
| source_tape_appends_in_filter_order_on_v1 | src/ui/app/source_ctl.rs | source_tape lays out clips end-to-end on track 0 in the given asset order; offsets vector matches cumulative durations |
| every_edit_op_has_a_tool_covers_place_asset_and_source_verbs | src/ui/app/tools_registry_tests.rs | structural scan (existing 0b test) finds place_asset/place_assets in OP_TOOLS with a live ToolDef name, and confirms splice_in/overwrite_in/lift_range/extract_range/replace_clip remain covered by trim-model's tools without a second, conflicting ToolDef name |
| no_duplicate_tool_names_with_trim_model | src/ui/app/tools_registry_tests.rs | tools_source::TOOLS contains none of timeline.splice/timeline.overwrite/timeline.lift/timeline.extract/timeline.replace — pins the audit fix so a future edit can't silently re-add the duplicate |
| source_pane_no_idle_repaint | src/ui/source_ui.rs | assert_no_idle_repaint harness: Pane::Source open, paused, 30 headless frames with no input request no repaint (idle-CPU gate) |
| every_glyph_paints_a_picture_covers_source_glyphs | src/ui/tools.rs | existing structural test extended: Append/CloseUp/PlaceOnTop/SourceRecord/Tape all draw without panicking |
| lib_preview_deleted_no_dead_refs | src/ui/app/panes.rs | compile-time: no remaining reference to LibPreview/start_lib_preview/draw_lib_preview exists after deletion |

## Verification checklist

- [ ] cargo test — all new tests above green, plus every existing library/timeline/model test unaffected
- [ ] cargo run -- --selftest
- [ ] cargo run -- <video> --screenshot x.ppm -> ffmpeg to PNG: Source pane open beside Preview with visible in/out ticks and smart-edit row
- [ ] manual: I/O in the Source pane sets marks (not the timeline's own I/O — verify focused-pane routing); Shift+V/B from a source-focused pane splice/overwrite at playhead using the marked range
- [ ] manual: F on a timeline clip opens its source asset and seeks to the matching frame; Ctrl+Shift+R reveals it in Library
- [ ] manual: Alt-drop a library asset onto an existing clip's body and confirm replace_clip semantics (duration/effects/transform kept); Alt-drop onto a gap/multiple clips and confirm overwrite_in semantics instead
- [ ] manual: toggle Source Tape with 3+ filtered library assets, confirm playback advances across the appended clips and cut ticks align
- [ ] manual: library multi-select 'Add to timeline' with 2+ assets chains them end-to-end exactly as before (place_assets regression)
- [ ] grep for a second ToolDef named timeline.splice/overwrite/lift/extract/replace anywhere in tools_source.rs returns nothing (duplicate-tool audit fix)
- [ ] confirm library.rs and panes.rs PRs land rebased onto media-library's / this workstream's own merged changes respectively, per the coordination notes in files[], not as concurrent same-file edits — including canvas-handles-monitor's mask_target follow-up landing after this workstream's panes.rs merge
- [ ] scripts/size.ps1 -Note source-monitor: delta within the +70 KB estimate (est_new_lines 1120 @ 0.08 KB/line minus lib_preview.rs's ~240-line deletion already booked in the wave-2 total; 5 fewer ToolDef rows than the prior draft)
- [ ] idle-repaint selftest step stays green with Pane::Source open and paused

## Acceptance criteria

- [ ] Pane::Source shows a live transport for a library asset with working I/O marks and a scrub bar with tick marks at the marks
- [ ] Space/J/K/L route to whichever of Source/Preview/Timeline currently has focus, never both at once
- [ ] Splice/Overwrite/Lift/Extract/Replace/Append all reachable from: a source-panel button, the existing hotkey (where the skeleton assigns one), and an MCP tool — three bindings of one Project:: fn, per the skeleton's core principle, using trim-model's already-registered tool names rather than a duplicate
- [ ] insert_at is deleted; every former caller compiles against place_asset (single id) or place_assets (multi id, chained) with an explicit DropMode — no former call site silently drops any asset beyond the first
- [ ] Alt-drop landing fully within an existing clip's body invokes replace_clip (duration/effects/transform kept); Alt-drop onto a gap or spanning multiple clips uses overwrite_in, matching the skeleton's own modifier table
- [ ] lib_preview.rs is deleted; grep for LibPreview/start_lib_preview/draw_lib_preview/lib_preview_frame returns nothing
- [ ] tools_source.rs registers no ToolDef named timeline.splice/overwrite/lift/extract/replace — tool_names_unique_and_namespaced passes against trim-model's tools without modification
- [ ] Match Frame opens the correct source asset at the correct source time; Reveal in Library scrolls/selects the right asset row
- [ ] Source Tape plays a synthetic sequence of the filtered bin without touching the main project or its undo stack
- [ ] every_edit_op_has_a_tool and ui_action_covers_every_action structural tests (0b) pass with this workstream's additions
- [ ] assert_no_idle_repaint passes with Pane::Source open and idle
- [ ] library.rs and panes.rs changes land in the rebase order specified in files[] (follow-up PRs, not concurrent co-edits) — no merge conflict or silent overwrite with media-library or canvas-handles-monitor; canvas-handles-monitor's Pane::Effects mask_target one-liner is confirmed (both plans' text) to land as a same-day follow-up PR rebased onto this workstream's merged panes.rs, not a concurrent wave-2 edit
- [ ] scripts/size.ps1 delta is within +70 KB of the estimate, or the PR body names the offset per the size gate's own threshold

## Risks

| Risk | Mitigation |
|---|---|
| Two Player instances alive at once (timeline Player + SourceState.player) doubles decode/GPU pipeline cost while Source is open | Source player pauses whenever the timeline plays and vice versa (existing app.rs:4381 pattern: start_lib_preview already calls self.player.pause()); document as accepted memory cost per pro design's own risk note |
| Splice near the head of a long timeline ripples every downstream clip -> one large video_dirty_spans invalidation | Accepted, once per op (matches pro design's noted risk); not a hot path |
| Deleting insert_at's 6 call sites in one PR risks missing one, or routing a multi-id site through single-id place_asset and silently dropping all but the first asset | place_assets_chains_multiple_ids_like_insert_at regression test, a full-repo grep for insert_at before the PR is opened, and an explicit per-site note (single-file-import sites vs. library multi-select sites) in the edit_ops.rs file entry so no site is miscategorized |
| Space/I/O routing ambiguity between Source pane and Timeline when neither is explicitly clicked this frame | Reuse the existing rule from LibPreview's design note (app.rs:4435-4437): last-focused-pane wins, timeline is the fallback; pin with a routing test |
| Pane::Source must already exist from wave 0b (Pane::ALL, preset builders, ROUND3 exclusion) or every preset test breaks | depends_on trim-model + player-rate-loop (wave 1) does not itself guarantee wave-0b landed first in this agent's branch base — verify Pane::Source compiles before writing source_pane.rs; if missing, flag as a wave-0 gap rather than silently patching around it |
| Detecting 'drop point falls fully within one clip's span' for the Overwrite->replace_clip branch needs a track+time lookup that doesn't exist yet as a named helper | Reuse Project's existing clip-at-time query (read the model/ops/tracks.rs or clip.rs query fn before writing place_asset; do not add a new duplicate scan) — verify the exact fn name/signature at implementation time since this plan does not name one |
| library.rs and panes.rs are edited by more than one wave-2 workstream (media-library + source-monitor on library.rs; source-monitor's own panes.rs edit sits next to canvas-handles-monitor's wave-2 preview work) — an unordered concurrent landing would produce a merge conflict or a silently overwritten arm | Both files now carry an explicit coordination note in files[]: media-library is library.rs's exclusive wave-2 owner (source-monitor's edit is a same-day follow-up PR rebased onto it); source-monitor is the sole wave-2 owner of the specific Pane::Preview/Pane::Library arms it edits in panes.rs, and canvas-handles-monitor's plan now states this identical resolution verbatim (its changelog: 'reconcile: accepted source-monitor's stated resolution for the panes.rs conflict verbatim') — canvas-handles-monitor's one-line Pane::Effects mask_target set lands as a same-day follow-up PR rebased onto this workstream's merged panes.rs, never a concurrent co-edit; both plans mutually confirm this — resolves the audited same-wave file conflict and its asymmetric-ownership follow-up finding |

## Suggested implementation order

1. 1. Verify wave-0b prerequisites exist in the branch base: Pane::Source in Pane::ALL/presets, DropMode/place_asset stub in edit_ops.rs, ToolDef/TOOL_TABLES, add_subclip stub — if any is missing, stop and flag rather than re-deriving them
2. 2. Port LibPreview -> SourceState in source_ui.rs (mechanical rename + field carry-over from app.rs:78-87,4380-4433), keep behaviour identical, get it compiling and screenshot-matching the old lib_preview view
3. 3. Add src_in/src_out fields + mark UI (I/O keys routed here when Source is focused) and tick-mark rendering on the scrub bar
4. 4. Implement place_asset (single id) over trim-model's ranged ops, calling them directly (no new ToolDef), including the Overwrite->replace_clip clip-body branch; write its regression/behaviour tests
5. 5. Implement place_assets as a thin chaining wrapper over place_asset; write the chaining regression test; migrate all 6 insert_at call sites (3 single-file-import, 3 library multi-select) to place_assets; delete insert_at
6. 6. Wire drop modifiers (Ctrl/Alt/Shift) in drops.rs to place_assets' DropMode
7. 7. Implement act_source: MatchFrame, RevealInLibrary, smart-edit compositions, smart_indicator; wire hotkeys F and Ctrl+Shift+R
8. 8. Implement subclip_from_marks and Source Tape (source_tape fn + pane toggle)
9. 9. Write tools_source.rs ToolDef rows (source.*, timeline.place/match_frame/smart_edit only); register in app/mod.rs's ws:source-monitor marker lines (mod decls + PANE_DRAWERS/ACT_HANDLERS/TOOL_TABLES); run every_edit_op_has_a_tool and no_duplicate_tool_names_with_trim_model
10. 10. Delete lib_preview.rs and the Preview-pane override branch; fix any remaining references
11. 11. Land the library.rs follow-up PR after media-library merges, and confirm panes.rs's Pane::Preview/Library arms have no pending canvas-handles-monitor edit before landing (per the mutually confirmed panes.rs handoff)
12. 12. Full verification pass (tests, selftest, screenshots, size gate, idle-repaint) and PR

## Deliberate simplifications (`// ponytail:`)

- SourceState reuses LibPreview's exact field set instead of redesigning the source player — it already works and is proven; only marks/tape are additive
- place_assets is a straight port of insert_at's own loop, not a generalized multi-asset placement engine — it exists only to keep the 6 real call sites working; add a richer batch-placement API only if a future workstream needs one
- The Overwrite->replace_clip branch reuses whatever clip-at-time query already exists in the model rather than adding a new one — read model/ops before writing this branch (see risks)
- Source Tape is a Vec<Id> + one synthetic Project, not a persisted feature — no Settings/Project field, no serialization, dropped on close (ponytail: in-memory only; a saved 'source tape sets' feature would need a Settings field, add if users ask)
- smart_indicator is a pure read (no caching layer) — recomputed each frame off the existing clip list; add a cache only if a profiled frame budget test flags it
- No new Tool button, no new modal — every verb is a button in the existing pane pattern or a hotkey already free in hotkeys.rs, matching the non-modal principle without inventing a gesture grammar of its own
- timeline.place/match_frame/smart_edit are the ONLY new timeline.* MCP tools this workstream adds — splice/overwrite/lift/extract/replace ride on trim-model's existing tools; do not add a convenience alias later without checking tools_trim.rs first

## Review trail

- FIXED (finding 1, confirmed by reading app.rs:1106-1112 and all 6 call sites 2456/2554/2800/2815/3735/4271 — re-verified live against HEAD 6b92982 by grep, all six confirmed present at exactly these lines): insert_at is genuinely Vec<Id>-chaining at every call site, not just the library multi-select ones. Added App::place_assets(ids, at, track, mode) as a thin wrapper chaining place_asset per id via the same `at = c.end()` advance insert_at used, and routed ALL 6 former insert_at sites through it. (Unchanged from prior revision — carried forward.)
- FIXED (finding 2, confirmed against the skeleton's own modifier_table Drop/Alt row — carried forward from prior revision, unchanged): DropMode::Overwrite branches to replace_clip when the drop point falls fully within one existing clip's span, falling back to overwrite_in otherwise.
- FIXED (finding 3 from prior revision, unchanged): src/ui/app/mod.rs is in files[], scoped to this workstream's own pre-seeded ws:source-monitor marker lines only.
- FIXED (finding 4 from prior revision, unchanged): source.mark and source.tape stay classified kind:ui, not mutate/job.
- FIXED (new audit finding, blocker — duplicate MCP tool names): tools_source.rs was declaring ToolDef rows named timeline.splice/timeline.overwrite/timeline.lift/timeline.extract that byte-for-byte duplicate trim-model's already-registered tools_trim.rs rows (both citing the same Project:: fns) — this would fail registries-schema-hooks' own tool_names_unique_and_namespaced structural test. Removed all four from mcp_tools and from the tools_source.rs files[] description; place_asset/act_source now call trim-model's Project::splice_in/overwrite_in/lift_range/extract_range directly instead of going through a second ToolDef. Added a no_duplicate_tool_names_with_trim_model regression test. Reduced size_delta_kb 71->70 and est_new_lines 1130->1120 to reflect 4 fewer ToolDef rows.
- FIXED (new audit finding, major — duplicate MCP tool name for the same fn): timeline.replace_clip (this plan) and trim-model's timeline.replace both mapped to Project::replace_clip. Per the audit's own resolution rule (later-landing workstream drops its duplicate), removed timeline.replace_clip from mcp_tools; the Overwrite->replace_clip branch in place_asset still calls Project::replace_clip directly, it is just no longer separately exposed as a second MCP tool name.
- FIXED (new audit finding, major — same-wave file conflicts on library.rs and panes.rs, unresolved on either side): added explicit coordination notes to both files[] entries. library.rs: media-library is the exclusive wave-2 owner per its own files[] ('src/ui/library.rs (wave 2 onward)'); this workstream's single-click-preview and Source-Tape-toggle edit lands as a same-day follow-up PR rebased onto media-library's merged library.rs changes, not a concurrent same-PR co-edit. panes.rs: source-monitor is declared the sole owner of the specific Pane::Preview/Pane::Library arms it touches (deleting lib_preview, redirecting insert_at calls); any canvas-handles-monitor panes.rs edit this wave is resequenced as a follow-up rebased onto this PR. Added a matching risks[] entry and a verification/acceptance-criteria line so the sequencing is checked before merge, mirroring the pattern forgiveness itself proposed for its own menus.rs conflict but never closed out.
- reconcile: (major) the panes.rs ownership statement was asymmetric — this plan unilaterally declared sole wave-2 ownership while canvas-handles-monitor's plan still hedged between 'a skeleton-grant, a wave-0b hook, or a coordinated deferral.' canvas-handles-monitor's plan has since been revised (its own changelog: 'reconcile: accepted source-monitor's stated resolution for the panes.rs conflict verbatim') to drop that hedge and state the identical resolution this plan states. Reworded this plan's panes.rs files[] entry and its matching risks[] mitigation to name that mutual confirmation explicitly and cite canvas-handles-monitor's specific deferred edit (the Pane::Effects mask_target one-liner, self.preview.mask_target = MaskTarget::Effect(i) alongside the existing mask_for wiring at app.rs:2697) by name, so both plans now read as one agreed handoff rather than one side's unilateral claim. Added a matching acceptance-criteria clause. No functional/code change — wording only, to close the reconcile finding.
- UNCHANGED beyond the six items above plus this reconcile pass: everything else — scope_in/out, glyphs, settings/project fields (still empty), luau's mechanical-bridge description, all remaining ponytail_notes, all remaining tests/risks/acceptance criteria not touched by the fixes.
