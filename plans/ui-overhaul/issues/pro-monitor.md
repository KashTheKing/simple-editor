# Pro-monitor: trim view, dynamic trim, scopes, wipe compare, stills, multicam

**Workstream:** `pro-monitor` · **Issue:** [#35](https://github.com/KashTheKing/simple-editor/issues/35) · **Wave:** 3 · **Branch/worktree:** `feat/pro-monitor` → `../simple-editor-wt/pro-monitor` · **Depends on:** canvas-handles-monitor, snap-engine, audio-analysis, color-engine · **~1420 new lines · Δ exe ≈ +112 KB**

Read [agents.md](../../../agents.md), [goals.md](../../../goals.md), [notes.md](../../../notes.md) and the master plan [README.md](../README.md) (registry protocol, keymap, modifier table) before starting.

## Summary

Adds pro-depth monitor features on top of wave 1-2 primitives: dual-frame trim view + dynamic (JKL) trim, a Scopes window (waveform/parade/vectorscope/histogram from color-engine's frame_stats), wipe/side-by-side grade compare (via color-engine's bypass render, verified to exist before use), stills save/apply (reusing the existing EffectPreset system - zero new storage), and multicam create/sync/switch with a capped 4-angle grid (switch operates on the nested Sequence's own tracks via a shared split-helper, never touching Project.editing/main_stash), all wired through the wave-2 alt-render channel (AltRequest/AltRenderState) and exposed as 10 typed MCP tools. No new Pane, no new Tool, 7 unbound actions.

## Motivation

Closes gap-matrix rows 30/31 (trim view, dynamic trim), 62/63/64 (render-bar wiring consumer, scopes, wipe/bypass/stills), 74 (multicam UI half) and 84 (eyedropper wiring half) - pro-depth monitor features delivered as thin UI/engine glue over primitives wave 1-2 already built (Player.rate, snap-engine's edit_point, color-engine's frame_stats/render_frame_bypass, audio-analysis's xcorr_offset, canvas-handles-monitor's AltRequest alt-render channel). Non-modal and MCP-parity principles: no new Tool, no new dockable Pane, every capability a ToolDef row.

## In scope

- Trim edit view (dual-frame, drag-to-trim)
- Dynamic trim (JKL rate-drop composition) plus a direct MCP tool for the same composition
- Video scopes window (waveform/parade/vectorscope/histogram)
- Wipe/side-by-side compare against a bypass render
- Stills save/apply via existing EffectPreset storage
- Multicam create/sync/switch (operating on the nested Sequence's own tracks, no main_stash swap) + angle-grid window
- Eyedropper click-to-sample wiring on the monitor (chroma/qualifier)
- 10 MCP tools + palette/ui.action coverage for the 7 new actions

## Out of scope

- Any new dockable Pane (Scopes/TrimView/Multicam are windows/overlays only)
- New Tool button, new hotkey chords beyond the 7 unbound actions
- Grade-bypass persistent toggle (BypassGrade action) - owned by color-engine
- Track rename/colour/reorder, Find, timeline view presets - owned by pro-timeline
- PNG thumbnails for stills, LUT export of a still's grade
- FFT-based or unbounded cross-correlation - inherits audio-analysis's cap
- Scoping video_dirty_spans to a sub-span of a multicam switch - full clear is the accepted ceiling this wave

## Files

| Op | Path | What |
|---|---|---|
| create | src/model/ops/multicam.rs | impl Project { multicam_make, multicam_switch (calls split_tracks_at on self.sequence_mut(id).tracks directly), multicam_angles } + #[cfg(test)] mod tests. |
| modify | src/model/ops/editing.rs | Extract `fn split_tracks_at(tracks: &mut Vec<Track>, t: f64, only: Option<&[Id]>) -> Vec<Id>` out of the body of `Project::split_at` (behaviour-identical: split_at becomes `split_tracks_at(&mut self.tracks, t, only)`); multicam.rs reuses this helper on Sequence.tracks. Pure refactor, existing split_at tests pin no behaviour change. |
| modify | src/model/ops/mod.rs | Append `pub mod multicam;` in the alphabetical/append list (pure move target created by wave-0 split-god-files). |
| create | src/ui/scopes_ui.rs | ScopeKind enum, window() egui::Window drawer, paint_waveform/paint_parade/paint_vectorscope/paint_histogram pure painters over &FrameStats. |
| create | src/ui/multicam_ui.rs | Angle-grid egui::Window (<=4 alt-render textures at 1/4 size), 'Create Multicam' dialogless flow (selection count check + toast), angle click = Action::NextAngle/PrevAngle equivalents or direct switch. |
| create | src/ui/app/tools_monitor.rs | pub const TOOLS: &[ToolDef]; act_monitor(app,a)->bool; save_still(); the 10 MCP tool run fns. |
| modify | src/ui/preview.rs | Add PreviewState.compare: CompareMode, PreviewCtx.pick_mode/alt_trim/alt_compare texture fields, PreviewResponse.picked/still_clicked; paint_trim_view() and wipe/side-by-side compare painting inside video(); eyedropper click handling reusing the existing click-drag dispatch at video() (preview.rs:719-758). |
| modify | src/ui/app/preview_pane.rs | Wire App.monitor state into PreviewCtx construction; handle PreviewResponse.picked -> color.pick write-back; handle still_clicked -> save_still(). |
| modify | src/ui/app/monitor.rs | Extend AltRequest (built wave-2 canvas-handles-monitor; carries Effect/Transition/Gallery already) with TrimOut/TrimIn/Compare/Angle(u8) variants; monitor_tick FRAME_HOOK: dynamic-trim rate-drop detection, trim-view request refresh (only on edit_point/delta change), angle-grid requests (<=4, paused while export.is_some()). |
| modify | src/hotkeys.rs | Append 7 rows (ToggleTrimView, CompareWipe, SaveStill, ToggleScopes, MulticamCreate, NextAngle, PrevAngle), all `sc(...)=>None`, under `// ---- ws:pro-monitor ----`. |
| modify | src/settings.rs | Append `#[serde(default)] pub trim_view: bool` and `#[serde(default)] pub scopes: Vec<String>` (open scope-tab names) under `// ---- ws:pro-monitor ----` in the struct and Default. |
| modify | src/ui/tools.rs | Append Glyph::Compare, Glyph::Scope, Glyph::Grid4 to the enum, ALL, name(), from_name(), draw_glyph() under my section (~12 lines/glyph). |
| modify | src/ui/mod.rs | Append `pub mod multicam_ui;` and `pub mod scopes_ui;` to the alphabetical mod list. |
| modify | src/ui/app/mod.rs | Append `mod tools_monitor;`, my App-struct field (`monitor: MonitorState`) + Default/new literal, and one line each in WINDOW_DRAWERS (scopes window, multicam angle-grid window), ACT_HANDLERS (act_monitor), FRAME_HOOKS (monitor_tick), TOOL_TABLES (&tools_monitor::TOOLS), all under `// ---- ws:pro-monitor ----`. |

## Model changes

- src/model/ops/editing.rs: extract `fn split_tracks_at(tracks: &mut Vec<Track>, t: f64, only: Option<&[Id]>) -> Vec<Id>` from Project::split_at's body; split_at becomes a one-line caller over self.tracks. Confirmed necessary by reading source: split_at (model.rs:3383) is hardcoded to self.tracks, and a Sequence's own clips live in a separate Sequence.tracks field (model.rs:2849), unreachable from split_at without this extraction or a main_stash swap via open_sequence/close_sequence (model.rs:3826-3864). Extraction is chosen over a stash swap: it never touches Project.editing/main_stash, so it can't leak state if multicam_switch is interrupted mid-call.
- src/model/ops/multicam.rs (new): `pub fn multicam_make(&mut self, ids: &[Id], offsets: &[f64], name: impl Into<String>) -> Option<Id>` - builds on nest_selection (model.rs:3893) but shifts each source clip's effective start by its offset before nesting so angles land in sync; one track per angle, video tracks only (audio rides with its linked clip).
- `pub fn multicam_switch(&mut self, seq_clip: Id, t: f64, angle: usize) -> bool` - maps timeline t to the inner Sequence's local time via clip.start/src_in, looks up `self.sequence_mut(inner_seq_id)` and calls `split_tracks_at(&mut seq.tracks, local_t, None)` directly on that Sequence's own tracks (no open_sequence/close_sequence, no main_stash involvement), then sets `clip.enabled = (track_index == angle)` on every resulting clip with start >= local_t. Non-destructive: earlier segments keep their prior enabled state.
- `pub fn multicam_angles(&self, seq_clip: Id) -> Vec<(usize, String)>` - read-only (angle index, track name) for the angle-grid UI and multicam.switch validation.
- No changes to Track/Clip/Project schema - multicam reuses existing Sequence/Track/Clip fields (clip.sequence, clip.enabled) exactly as they exist today; zero new serde fields.

## Engine changes

- Consume (not own) src/playback.rs Player::request_layers/take_layers_reply and Player::rate() - built by wave-1 player-rate-loop; poll a rate 'was nonzero, now 0' transition for dynamic trim, gated on a small MonitorState field, not a new Cmd.
- Consume src/engine/gpu.rs GpuRenderer::frame_stats(...) and ::render_frame_bypass(&LayerSet,...) -> TextureId - both attributed to wave-1 color-engine but NOT present in gpu.rs today (verified: only render_frame/render_preview_texture/effect_preview exist at gpu.rs:345-403) and not committed in color-engine's own skeleton deliverables. Implementation-order step 1 now explicitly requires confirming both APIs exist on this worktree's base before building Scopes/Compare; if render_frame_bypass is missing, CompareWipe/SideBySide ship as a documented follow-up rather than a broken feature (ponytail_notes).
- Consume src/engine/analysis.rs xcorr_offset(a:&Peaks,b:&Peaks,max_lag_s:f64)->Option<f64> (wave-1 audio-analysis) fed by WaveformCache::get(path,stream) (src/media/waveform.rs:79) for multicam sync; no new Peaks math in this workstream.
- New src/model/ops/multicam.rs, reusing a `split_tracks_at` helper extracted from Project::split_at (see model_changes) instead of the previously-planned (and unworkable) 'call split_at on the inner Sequence' - split_at is hardcoded to self.tracks and has no path to a Sequence's own tracks short of a main_stash swap, which this design avoids entirely.

## UI changes

- Preview letterbox gains: dual-frame trim view (edit_point-gated), wipe/side-by-side compare overlay, eyedropper one-shot click mode.
- New Scopes egui::Window (waveform/parade/vectorscope/histogram tabs), opened/closed by Action::ToggleScopes.
- New Multicam angle-grid egui::Window (<=4 live alt-render thumbnails, click to switch).
- No new Pane, no new Tool button, no modal dialogs - all overlays/windows, all non-blocking.

## New types and functions

- `pub enum CompareMode { Off, Wipe(f32), SideBySide }` — src/ui/preview.rs: Preview-local compare state (PreviewState.compare); f32 = wipe split 0..1, dragged on the video rect.
- `pub enum PickTarget { Chroma, Qualifier }` — src/ui/preview.rs: Set by the (color-engine-owned) eyedropper button in Pane::Color; consumed by video() to enter one-shot pick mode on the next click.
- `pub(crate) struct MonitorState { alt: AltRenderExt, dyn_trim_armed: bool, last_rate: f64, scopes_open: bool, stats_cache: Option<(u64, FrameStats)> }` — src/ui/app/monitor.rs: App field (App.monitor) holding this workstream's per-frame state; stats_cache keyed by a frame-changed counter so scopes never force an extra readback.
- `fn monitor_tick(app: &mut App, ctx: &egui::Context)` — src/ui/app/monitor.rs: FRAME_HOOK: detects rate!=0 -> rate==0 with an edit_point selected (dynamic trim), refreshes trim-view/compare/angle alt-render requests only when their key changed, all paused while app.export.is_some().
- `fn act_monitor(app: &mut App, a: Action) -> bool` — src/ui/app/tools_monitor.rs: ACT_HANDLERS entry for the 7 actions; SaveStill/CompareWipe/ToggleScopes/ToggleTrimView are pure state flips, MulticamCreate/NextAngle/PrevAngle call into model/ops/multicam.rs with push_undo_labeled.
- `fn split_tracks_at(tracks: &mut Vec<Track>, t: f64, only: Option<&[Id]>) -> Vec<Id>` — src/model/ops/editing.rs: Extracted body of the former Project::split_at, generic over any track vec so multicam_switch can split a nested Sequence's own tracks without a main_stash swap; split_at becomes a 1-line caller over self.tracks.
- `pub struct FrameStatsView<'a>(&'a FrameStats)` — src/ui/scopes_ui.rs: Thin borrow wrapper so paint_waveform/paint_parade/paint_vectorscope/paint_histogram take one type; pure functions, unit-testable without egui.
- `pub fn window(ctx: &egui::Context, open: &mut Vec<String>, stats: Option<&FrameStats>, pal: &Palette)` — src/ui/scopes_ui.rs: The Scopes egui::Window; tabs from `open` (Settings.scopes), no repaint request when stats is unchanged from last call.
- `pub fn angle_grid(ctx: &egui::Context, angles: &[(usize,String)], textures: &[(usize, egui::TextureId)], cur: usize) -> Option<usize>` — src/ui/multicam_ui.rs: egui::Window painting up to 4 alt-render textures at 1/4 size; returns Some(angle) on click, wired to Project::multicam_switch.

## Actions and hotkeys

| Action id | Label | Chord | Note |
|---|---|---|---|
| toggle_trim_view | Trim View in Preview |  | Unbound by skeleton keymap; auto-shown in Preview whenever Settings.trim_view is on AND TimelineState.edit_point is Some. Palette/View-menu toggle only. |
| compare_wipe | Wipe Compare |  | Unbound. Cycles PreviewState.compare: Off -> Wipe(x=0.5) -> SideBySide -> Off. Requests an AltRequest::Compare bypass render via gpu.render_frame_bypass (color-engine, wave1) - see engine_changes for the pre-build verification requirement. |
| save_still | Save Still |  | Unbound. Captures the selected clip's effect stack into Settings.effect_presets via engine::presets::capture_effects - no new storage. |
| toggle_scopes | Show / Hide Scopes |  | Unbound. Opens/closes the Scopes egui::Window (not a Pane); Settings.scopes remembers which tabs were open. |
| multicam_create | Create Multicam from Selection |  | Unbound. Requires >=2 selected clips; computes xcorr offsets then calls Project::multicam_make. Angle grid capped at 4 (ponytail). |
| next_angle | Next Angle at Playhead |  | Unbound. Only enabled when the clip under the playhead is a multicam sequence clip (App::enabled checks clip.sequence != 0 and its track count). |
| prev_angle | Previous Angle at Playhead |  | Unbound. Mirrors next_angle. |

## New glyphs

- Compare
- Scope
- Grid4

## Persisted fields

**Settings:**

- trim_view: bool  #[serde(default)]  - Preview shows the dual-frame trim view when an edit_point is selected
- scopes: Vec<String>  #[serde(default)]  - which Scopes tabs were last open ("Waveform","Parade","Vectorscope","Histogram")

**Project (.sedit):**

- (none)

## MCP tools (required — every capability must be scriptable)

| Tool | Kind | Args | Description | Maps to |
|---|---|---|---|---|
| timeline.trim_view | ui | on:boolean:true:show/hide the dual-frame trim view | Toggle Settings.trim_view; the view only actually paints when an edit_point is also selected. | Settings.trim_view + PreviewCtx trim-view branch in preview.rs |
| timeline.dynamic_trim | mutate | edit_point_side:string:true:left\|right\|both, dt:number:true:seconds to apply (sign = direction) | Apply the same ripple/roll composition dynamic (JKL) trim performs on a rate-drop, without shuttling - closes the gap where dynamic trim had no direct script entry point. | the same ripple_trim/roll_edit calls monitor_tick's rate-drop detector makes, exposed directly, one push_undo_labeled("Dynamic trim") |
| preview.compare | ui | mode:string:true:off\|wipe\|side, x:number:false:split 0..1 for wipe | Set the monitor's compare mode against a bypass (ungraded) render. | PreviewState.compare + AltRequest::Compare via gpu.render_frame_bypass |
| stills.save | ui | clip_id:integer:false:default selection, name:string:false:default auto-numbered | Snapshot the clip's effect stack as a named preset (a 'Look'). | engine::presets::capture_effects -> Settings.effect_presets.push (no Project mutation, no undo) |
| stills.apply | mutate | clip_id:integer:true:target clip, name:string:true:preset name from stills.save | Paste a saved still's effect stack onto a clip. | engine::presets::apply_effects(preset, project, clip_id) - existing fn, unchanged |
| scopes.read | read | kind:string:false:waveform\|parade\|vectorscope\|histogram, default all | Current-frame statistics for an agent to reason about exposure/colour without a screenshot. | GpuRenderer::frame_stats() cached on App, JSON-serialised |
| multicam.sync | read | clip_ids:array:true:asset-backed clip ids to align | Dry-run: compute cross-correlation offsets without creating a sequence. | engine::analysis::xcorr_offset over WaveformCache peaks, pairwise vs clip_ids[0] |
| multicam.create | mutate | clip_ids:array:true:>=2 clips, name:string:false:sequence name | Sync + nest into a multicam sequence, one track per angle (capped at 4 angles for the grid). | Project::multicam_make(ids, offsets, name) |
| multicam.switch | mutate | seq_clip_id:integer:true:, t:number:false:default playhead, angle:integer:true:0-based | Switch the active angle from time t onward (split + enabled toggles, editable afterward). Note: mutates a nested Sequence's own tracks directly, one full playback-cache clear per switch (accepted ceiling, see risks). | Project::multicam_switch(seq_clip, t, angle) via split_tracks_at on Sequence.tracks |
| color.pick | mutate | x:number:true:0..1 in the letterbox, y:number:true:, target:string:true:chroma\|qualifier | Sample the frame at (x,y); if a matching effect is selected, writes its key colour / qualifier centre (one undo); otherwise just returns the sampled RGB. | frame_stats sample -> Effect P_CHROMA / EffectKind::Qualifier centre param (Qualifier from wave-1 color-engine) |

**Luau:** No new Luau surface beyond automatic inheritance: editor.tools() lists the 10 new tools (timeline.trim_view, timeline.dynamic_trim, preview.compare, stills.save, stills.apply, scopes.read, multicam.create, multicam.sync, multicam.switch, color.pick) via TOOL_TABLES; editor.action(id) covers the 7 new arg-free actions via the existing ui.action bridge. No `-- @on` hooks added; a user script can still react to selection_changed and call editor.tool('multicam.switch', {...}) itself, or call timeline.dynamic_trim directly instead of composing a JKL shuttle.

## Tests

| Test | File | Asserts |
|---|---|---|
| split_tracks_at_matches_old_split_at_behavior | src/model/ops/editing.rs | the extracted helper called via Project::split_at(t, only) over self.tracks produces byte-identical results to the pre-extraction implementation on the existing split_at test fixtures - pure refactor, zero behaviour change. |
| multicam_make_creates_one_track_per_angle_with_offsets | src/model/ops/multicam.rs | 3 clips with offsets [0,1.2,-0.4] nest into a sequence with 3 video tracks, each clip's start shifted so all three read frame-aligned at t=0 after offset compensation. |
| multicam_switch_splits_and_toggles_enabled_only_after_t | src/model/ops/multicam.rs | switching to angle 1 at t=5 leaves angle-0 clips enabled before 5s and disabled after; angle-1 enabled after 5s; clip ids before the split point are untouched. |
| multicam_switch_rejects_out_of_range_angle | src/model/ops/multicam.rs | angle >= track count returns false and mutates nothing. |
| multicam_switch_never_touches_project_editing_or_main_stash | src/model/ops/multicam.rs | before/after a multicam_switch call, project.editing and project.main_stash are unchanged (regression guard for the rejected open_sequence/close_sequence approach). |
| dynamic_trim_applies_on_rate_drop_with_edit_point_and_trim_view_on | src/ui/app/monitor.rs | simulated rate 4.0->0.0 with edit_point Some and Settings.trim_view=true calls ripple_trim/roll_edit with dt=playhead-cut and exactly one push_undo_labeled("Dynamic trim"). |
| dynamic_trim_noop_without_edit_point_or_trim_view_off | src/ui/app/monitor.rs | same rate transition with edit_point=None, or trim_view=false, pushes zero undo steps. |
| dynamic_trim_mcp_tool_matches_the_implicit_composition | src/ui/app/tools_monitor.rs | timeline.dynamic_trim(side, dt) produces the identical model mutation and undo label as the rate-drop path in monitor_tick, for the same edit_point/dt. |
| alt_render_requests_paused_during_export | src/ui/app/monitor.rs | with app.export=Some(..), monitor_tick issues zero new AltRequest::{TrimOut,TrimIn,Compare,Angle} requests even with an edit_point and open compare/scopes state. |
| trim_view_paints_two_textures_only_with_edit_point | src/ui/preview.rs | headless Harness: PreviewCtx with edit_point=None paints no trim-view region; with Some(..) and two alt textures ready, paints two side-by-side rects. |
| compare_wipe_drag_updates_split_x | src/ui/preview.rs | dragging across the video rect in Wipe mode updates CompareMode::Wipe(x) monotonically with pointer x, clamped 0..1. |
| eyedropper_click_returns_picked_color | src/ui/preview.rs | a click while pick_mode=Some(Chroma) returns PreviewResponse.picked=Some([r,g,b]) sampled from the cached FrameStats, and does not move the clip. |
| save_still_pushes_effect_preset | src/ui/app/tools_monitor.rs | Action::SaveStill on a selected clip appends exactly one EffectPreset to Settings.effect_presets with the clip's current effect JSON; no Project undo entry. |
| stills_apply_round_trips | src/ui/app/tools_monitor.rs | stills.save then stills.apply(other_clip) makes other_clip's effects equal the source clip's at save time, with one Project undo entry. |
| multicam_switch_forces_full_cache_clear | src/playback.rs (existing video_dirty_spans tests, extended) | video_dirty_spans(old, new) returns None across a multicam_switch (project.sequences JSON differs) - pins the accepted full-clear ceiling rather than silently assuming a bounded span. |
| scopes_paint_fns_match_known_histogram | src/ui/scopes_ui.rs | paint_waveform/paint_histogram over a synthetic FrameStats (known percentiles) produce bar heights within 1px of the expected mapping - pure function, no egui context needed. |
| assert_no_idle_repaint_on_scopes_window | src/ui/scopes_ui.rs | harness helper: Scopes window open with an unchanged FrameStats requests no repaint after the first frame. |
| assert_no_idle_repaint_on_multicam_angle_grid | src/ui/multicam_ui.rs | angle grid open, no pending alt-render replies, requests no repaint. |
| every_edit_op_has_a_tool_covers_multicam | src/ui/app/tools_registry_tests.rs (existing, extended) | multicam_make/multicam_switch appear in OP_TOOLS mapped to multicam.create/multicam.switch - structural parity, not a new test file. |
| mutate_rows_roll_back_on_error_covers_pro_monitor | src/ui/app/tools_registry_tests.rs (existing) | multicam.create/multicam.switch/color.pick/stills.apply/timeline.dynamic_trim called with garbage args leave project JSON unchanged and push no undo - existing structural test, extended by construction. |

## Verification checklist

- [ ] cargo test - full suite green including the tests above plus unchanged existing structural tests (every_glyph_paints_a_picture, no_duplicate_defaults, reserved_chords_are_free, every_edit_op_has_a_tool, mutate_rows_roll_back_on_error, tool_names_unique_and_namespaced, server_end_to_end tools/list count).
- [ ] cargo test --release bench_4k_preview -- --ignored --nocapture: within 10% of the wave-2 baseline with Scopes+TrimView+Angle-grid all open.
- [ ] cargo test headless_1000_clips_stays_fast: unaffected (multicam ops touch at most 4 tracks).
- [ ] cargo run -- --selftest: idle-repaint step stays green with Scopes window and Multicam angle-grid open and static.
- [ ] scripts/size.ps1 -Note pro-monitor: delta near 112 KB; PR body carries `size: +N KB` line if over.
- [ ] Screenshots: edit point selected + trim view; Scopes window (waveform+vectorscope); wipe compare mid-drag; multicam angle grid with 3 angles; eyedropper cursor over video.
- [ ] Manual: J-J-J shuttle to a selected cut, release - clip trims to playhead (dynamic trim). Ctrl+D duplicate + multicam switch mid-playback triggers one full playback-cache clear (accepted, not a per-frame path) and does not corrupt state; angle-grid renders pause during an active export.
- [ ] MCP smoke: multicam.sync then multicam.create on 3 selected clips returns a sequence id; timeline.dynamic_trim reproduces the same trim as a JKL shuttle-to-stop; scopes.read returns non-empty histogram after a frame renders (only if gpu.frame_stats exists on this worktree's base - else this bullet is deferred, see risks); stills.save then stills.apply round-trips a grade onto a second clip.

## Acceptance criteria

- [ ] Selecting a seam (edit_point set) with Settings.trim_view on shows outgoing/incoming frames side by side in the Preview letterbox; dragging either image trims via ripple_trim/roll_edit.
- [ ] Shuttling (J/L) to a stop with an edit_point selected and trim_view on applies a ripple/roll trim of dt=playhead-cut, one undo labelled 'Dynamic trim'; timeline.dynamic_trim MCP tool reproduces the identical mutation without shuttling.
- [ ] Ctrl+K palette or View menu 'Scopes' opens an egui::Window with Waveform/Parade/Vectorscope/Histogram tabs, fed by color-engine's gpu.frame_stats (verified present before building), repainting only on frame change.
- [ ] CompareWipe cycles Off->Wipe->SideBySide, rendering the graded frame against a bypass render (gpu.render_frame_bypass, verified present before building) with a draggable split.
- [ ] SaveStill on a selected clip appends an EffectPreset to Settings.effect_presets; stills.apply writes it onto another clip via the existing apply_effects, one undo.
- [ ] multicam.create on >=2 selected clips computes xcorr offsets (engine::analysis::xcorr_offset) and nests them one-track-per-angle; multicam.switch splits the inner sequence at t (via split_tracks_at on that Sequence's own tracks, no main_stash swap) and enables exactly one angle from t onward, triggering one full playback-cache clear (accepted); the angle grid egui::Window shows <=4 angles via alt-render.
- [ ] Eyedropper click on the monitor samples frame_stats and, when a ChromaKey/Qualifier effect is targeted, writes its key colour / centre with one undo.
- [ ] All 7 new actions compile with no default chord (unbound), pass no_duplicate_defaults and reserved_chords_are_free; every new Glyph passes every_glyph_paints_a_picture; every new Project mutator has a ToolDef row (every_edit_op_has_a_tool).
- [ ] cargo test, --selftest (idle step included), and scripts/size.ps1 all green; delta within ~112 KB of estimate or justified in the PR body.
- [ ] No new dockable Pane; TrimView/Scopes/Multicam-grid are Preview overlays / egui::Window only, never in Pane::ALL.
- [ ] multicam_switch_never_touches_project_editing_or_main_stash test passes, confirming the rejected open_sequence/close_sequence design was never needed.

## Risks

| Risk | Mitigation |
|---|---|
| color-engine's frame_stats/render_frame_bypass APIs are attributed to wave-1 but are not present in src/engine/gpu.rs today and are not explicitly committed in color-engine's own skeleton deliverables - Scopes and Wipe/SideBySide compare have no engine to call. | Implementation-order step 1 now requires confirming both APIs exist (by reading gpu.rs) on this worktree's base before writing Scopes/Compare code; if either is missing, file it back to color-engine as a required deliverable and ship Scopes/Compare behind a feature flag that no-ops until the API lands, rather than guessing a signature. |
| canvas-handles-monitor's alt-render type is confirmed AltRequest/AltRenderState (Effect/Transition/Gallery variants, App.alt_render field), but its PreviewCtx field names (pick_mode/alt/etc.) aren't pinned until that workstream's code actually lands. | Implementation order step 1 mandates reading the merged monitor.rs/preview.rs on this worktree's base branch before writing any new code, and renaming to match rather than re-deriving the channel. |
| Alt-render fan-out: Hover + TrimOut + TrimIn + Compare + up to 4 Angle requests could look like 8 concurrent consumers of one Player render-request queue. | Each use only requests while genuinely visible (trim-view needs an edit_point, compare needs mode!=Off, angle grid only requests on-screen angles); one in-flight request per AltRequest variant, newest wins (existing contract from canvas-handles-monitor), all paused while export.is_some(). |
| Dynamic-trim's implicit rate-drop detection could fire on an unrelated Stop press if the user had an old edit_point still selected. | Gate on Settings.trim_view being on AND edit_point selected AND the Preview pane being the last-hovered pane this frame (reuse SelectionKind/focus state already tracked by wave-0 registries) - documented as the exact arming condition, tested by dynamic_trim_noop_without_edit_point / _without_trim_view_on. timeline.dynamic_trim MCP tool gives scripts a non-implicit path to the same composition. |
| multicam_switch mutates a nested Sequence, and video_dirty_spans (playback.rs:791-808) clears the ENTIRE playback cache on any project.sequences JSON diff, before any per-track bound - confirmed by reading the source, not scoped to the switch's affected span. | Accepted ceiling this wave, not a per-frame hot path like J-K-L shuttle: documented in ponytail_notes and pinned by multicam_switch_forces_full_cache_clear; the earlier plan's 'doesn't stutter' claim is softened to 'triggers one full clear, does not corrupt state'. Upgrade path: scope video_dirty_spans to diff inside project.sequences per-sequence instead of a blanket clear - real engine work, not scoped here. |
| multicam_switch's split on every switch fragments the inner sequence's clips over a long interview edit. | Accepted (every NLE does this); a future 'flatten multicam' op is a join_through call away (trim-model, already built) - not built here. |
| Scopes/frame_stats readback runs every frame while the Scopes window is open on a busy timeline. | MonitorState.stats_cache keyed by App's existing frame-changed marker (from canvas-handles-monitor's hover-tick change detection); frame_stats is only recomputed when that marker moves, matching color-engine's own 'only when the frame changes' contract. |

## Suggested implementation order

1. Re-read the merged wave-2 src/ui/app/monitor.rs + src/ui/preview.rs on this worktree's base and reconcile this plan's PreviewCtx field-name guesses with whatever canvas-handles-monitor actually landed (the alt-render type itself is confirmed AltRequest/AltRenderState); separately confirm gpu.rs on this base actually has frame_stats and render_frame_bypass (color-engine's wave-1 deliverable) before writing Scopes/Compare code - if missing, stub them behind a no-op flag and flag color-engine.
2. src/model/ops/editing.rs: extract split_tracks_at(tracks, t, only) from Project::split_at's body; run existing split_at tests to confirm zero behaviour change.
3. src/model/ops/multicam.rs: multicam_make/multicam_switch (using split_tracks_at on self.sequence_mut(id).tracks, never main_stash)/multicam_angles + unit tests (headless, no UI).
4. src/ui/app/monitor.rs: extend AltRequest, monitor_tick FRAME_HOOK (dynamic trim + trim-view refresh + angle-grid requests, export-paused).
5. src/ui/preview.rs + preview_pane.rs: CompareMode, trim-view paint, eyedropper click, PreviewResponse plumbing.
6. src/ui/scopes_ui.rs: paint fns against a synthetic FrameStats fixture, then the egui::Window.
7. src/ui/multicam_ui.rs: angle-grid window + create-multicam entry point.
8. src/ui/app/tools_monitor.rs: act_monitor + 10 ToolDefs (save_still/apply reuse engine::presets verbatim; timeline.dynamic_trim reuses the same ripple_trim/roll_edit call as the rate-drop path).
9. Registry appends: hotkeys.rs, settings.rs, tools.rs (glyphs), ui/mod.rs, ui/app/mod.rs (last, one commit, minimal diff).
10. cargo test, --selftest, scripts/size.ps1 -Note pro-monitor, screenshot matrix, PR.

## Deliberate simplifications (`// ponytail:`)

- Stills reuse Settings.effect_presets + engine::presets::capture_effects/apply_effects verbatim instead of a new Still{} struct/field - a 'still' IS an EffectPreset plus, optionally, a PNG (skipped: no PNG capture wired yet, add via export_frame(with_effects) when the gallery wants thumbnails).
- Dynamic trim is a FRAME_HOOK watching a rate 2->0 transition, not an ACT_HANDLERS interception of Action::Stop - avoids short-circuit-OR ordering hazards with player-rate-loop's own Stop handler; timeline.dynamic_trim MCP tool covers scriptability so this stays a UI convenience, not a hidden-only path. Upgrade path: a dedicated Action::CommitDynamicTrim if this proves too implicit in testing.
- Multicam angle grid hard-capped at 4 angles at 1/4 render size (one alt-render request per visible angle, none for hidden ones) - more angles = scroll the grid later, not more concurrent renders.
- multicam_switch triggers a full playback-cache clear every time (video_dirty_spans clears on any project.sequences diff) - accepted this wave since switching is a deliberate user action, not a per-frame path; upgrade path: scope video_dirty_spans to diff per-sequence instead of a blanket clear.
- split_tracks_at is extracted from split_at rather than reusing a main_stash swap (open_sequence/close_sequence) for multicam_switch - the extraction is the smaller, safer diff: it can't leak Project.editing/main_stash state on an early return or panic mid-switch, and it costs one pure-refactor commit against an already-tested function.
- No PNG thumbnail on stills yet, no LUT export of a still's grade - both are one more engine::presets call away if the Gallery (inspector-gallery, a sibling workstream) wants them.
- Cross-correlation cap (max_lag_s) and any FFT upgrade are audio-analysis's (wave-1) ceiling, not re-litigated here; multicam.create just toasts if clip_ids.len() > 8 to keep pairwise xcorr calls bounded.

## Review trail

- wrong-path (multicam_switch): CONFIRMED by reading model.rs - split_at (3383) is hardcoded to self.tracks; Sequence.tracks (2849) is a separate field with no path from split_at short of a main_stash swap via open_sequence/close_sequence (3826-3864). Fixed by extracting a track-generic `split_tracks_at` helper from split_at's body and having multicam_switch call it directly on `self.sequence_mut(id).tracks` - avoids ever touching Project.editing/main_stash. Updated model_changes, engine_changes, files (new editing.rs modify entry), new_types_and_fns, tests (added split_tracks_at_matches_old_split_at_behavior and multicam_switch_never_touches_project_editing_or_main_stash), acceptance_criteria, and implementation_order accordingly.
- cache-invariant: CONFIRMED by reading playback.rs:791-808 - video_dirty_spans returns None (full clear) on any project.sequences JSON diff, before any per-track bound, so every multicam switch is a full cache clear. Softened the verification/acceptance claim from 'doesn't stutter' to 'triggers one full clear, does not corrupt state'; added a risk entry with mitigation (accepted ceiling, documented upgrade path); added test multicam_switch_forces_full_cache_clear to pin the behaviour instead of silently assuming a bounded span; added scope_out line making the non-scoping explicit.
- other (render_frame_bypass dependency): CONFIRMED by reading gpu.rs - only render_frame/render_preview_texture/effect_preview exist today; render_frame_bypass and frame_stats are unbuilt and not explicitly committed in color-engine's skeleton deliverables. Strengthened implementation_order step 1 to require verifying both APIs before building Scopes/Compare, added a risk entry with a stub/no-op fallback, and caveated the MCP smoke-test bullet in verification.
- mcp-gap (dynamic trim unreachable by script): valid - ponytail_notes explicitly rejected an Action for it, leaving zero script path. Added `timeline.dynamic_trim` as a 10th MCP tool that reuses the exact same ripple_trim/roll_edit call the implicit rate-drop path makes; updated mcp_tools, luau, tests, acceptance_criteria, ponytail_notes, scope_in, and est_new_lines/size_delta_kb left unchanged (one thin tool row, negligible size).
- other (naming): CONFIRMED cosmetic - corrected 'player-rate-loop-scrub' to the skeleton's actual workstream name 'player-rate-loop' in engine_changes; depends_on was already correct and untouched.
- No changes made for anything not cited by a finding - acceptance_criteria items 1/5/7/8/9/10, scope items, glyphs, settings_fields, project_fields, and most risks/tests are preserved verbatim from the original plan.
- reconcile: CONFIRMED by reading canvas-handles-monitor's plan - its actual (audit-confirmed) type is `AltRequest`/`AltRenderState` (Effect/Transition/Gallery variants, App.alt_render field, with Gallery added for inspector-gallery); no AltUse/AltReq symbol exists anywhere. Renamed every AltUse/AltReq reference in this plan (files[] monitor.rs, actions_and_hotkeys.compare_wipe, mcp_tools.preview.compare.maps_to, a test assertion, two risks, and two implementation_order steps) to AltRequest, adding TrimOut/TrimIn/Compare/Angle(u8) as new variants on that existing enum, mirroring how inspector-gallery added its own Gallery variant. The stale risk about the type name itself being unconfirmed was narrowed to only the still-open PreviewCtx field-name uncertainty.
