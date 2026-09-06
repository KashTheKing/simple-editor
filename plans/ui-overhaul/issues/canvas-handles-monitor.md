# Canvas transform/crop handles, mask-target routing, canvas snapping, viewer zoom/pan, and the alt-render monitor consumer

**Workstream:** `canvas-handles-monitor` · **Issue:** [#27](https://github.com/KashTheKing/simple-editor/issues/27) · **Wave:** 2 · **Branch/worktree:** `feat/canvas-handles-monitor` → `../simple-editor-wt/canvas-handles-monitor` · **Depends on:** player-rate-loop · **~1370 new lines · Δ exe ≈ +110 KB**

Read [agents.md](../../../agents.md), [goals.md](../../../goals.md), [notes.md](../../../notes.md) and the master plan [README.md](../README.md) (registry protocol, keymap, modifier table) before starting.

## Summary

Direct manipulation on the preview canvas: scale corners + edge scale_x/y + rotate handle on the selection outline (preview.rs:767-777 outline, 779-833 vertex-handle template), crop handles behind an explicit toggle writing find-or-append EffectKind::Crop (model.rs:667,794,859 P_CROP), group bounding box for multi-select (widen the .find() at 749-758), canvas snap guides (centre/edge/third/other-clip), Ctrl+wheel/middle-drag zoom-pan replacing dead Tool::Zoom, click-to-edit timecode, drop-onto-monitor via the existing insert_at, per-effect mask-target routing (rewires the EXISTING Tool::Mask drag block at preview.rs:496-533, renamed to avoid colliding with the new PreviewState.mask_target field), and monitor.rs's FRAME_HOOK: the async alt-render consumer that turns a hover request into a GPU texture via a cloned Project + Player::request_layers (from player-rate-loop) + gpu.render_preview_texture (gpu.rs:373-393), paused during export, never mutating the project or pushing undo. AutoReframe/ToggleProxies/ViewerFit get real ACT_HANDLERS bodies in this PR, and AutoReframe's tracking box comes from an existing tracked box only. AltRequest also carries a Gallery(tab,name) variant so inspector-gallery's future gallery.hover tool rides this same App.alt_render pipeline instead of inventing a separate hover_preview field.

## Motivation

goals.md direct-manipulation principle + gap rows 57-61,66,32,52,98 and critique items 9,10,19 and hooks 8,9: today the canvas only drags the whole clip and shows a dead outline; scale/rotate/crop require leaving the canvas for inspector DragValues; there is no hover preview, no canvas snap, no drop target, and Tool::Zoom is a dead button. Fixes the acknowledged mask-target ponytail and delivers the async render channel every other alt-render consumer will build on in later waves.

## In scope

- Transform handles (corner uniform scale, edge scale_x/scale_y, rotate) on the single-selected visual clip's outline
- Crop handles behind an explicit 'Crop Handles' toggle, writing EffectKind::Crop
- Multi-clip union bounding box + group move (no transform/crop/rotate for multi-select)
- Canvas hover cursors for handles/outline/tracker
- Canvas snap (centre/edges/thirds/other clips) with guide lines, gated by Settings.canvas_snap
- Ctrl+wheel zoom / middle-drag pan of the viewer (no new Tool)
- Click-to-edit numeric timecode on the transport label
- Drop media onto the monitor (place on a free track at the playhead)
- Per-effect mask editing: MaskTarget::{Clip,Effect(idx)} routes the EXISTING Tool::Mask drag block (preview.rs:496-533), not a new drag path
- Async alt-render consumer (monitor.rs FRAME_HOOK): hover-preview texture pipeline, export-paused, one in flight, with an AltRequest::Gallery variant reserved for inspector-gallery's hover consumer
- Heuristic auto-reframe command wired to an EXISTING tracked box only (no subject-detection fallback)
- Real ACT_HANDLERS bodies for AutoReframe, ToggleProxies, ViewerFit — registered in app/mod.rs, not left dangling
- Transport polish: glyph_for-driven icon overrides, proxy toggle, dropped-frame/buffering % via PreviewCtx
- 8 MCP tools + Luau exposure for every capability above

## Out of scope

- Face-aware/NN reframe (skip list)
- Trim edit dual-frame view, wipe/compare, scopes (pro-monitor, wave 3)
- inspector-gallery's own gallery.hover tool and its UI trigger (inspector-gallery, wave 2) — this PR only reserves the AltRequest::Gallery variant it will target
- place_asset's real splice/overwrite/place-on-top funnel (source-monitor, wave 2)
- Per-corner non-uniform scale via a modifier (data model has no anchor field; ponytail'd)
- Timecode entry driving a trim delta at an edit point (needs TimelineState.edit_point from snap-engine/trim-model)
- Auto-reframe without a pre-existing tracked box (no motion-saliency heuristic exists in-tree; out of scope, not silently invented)

## Files

| Op | Path | What |
|---|---|---|
| modify | src/ui/preview.rs | PreviewState: + handle: Option<HandleDrag>, crop_mode: bool, mask_target: MaskTarget, view: (f32,Vec2), canvas_rect: Rect, tc_edit: Option<String>. New Handle/MaskTarget/HandleDrag types. Corner/edge/rotate/crop hit-test+drag in video() after the polygon-handle block (779-833); widen the selection .find() (749-758) to a filtered collect for the group box; apply_view() choke point for zoom/pan; cursor-icon calls; extend background_menu with a 'Crop Handles' checkbox; click-to-edit timecode in transport(). ALSO rewire the pre-existing Tool::Mask drag block (verified at lines 496,502,520,523) to branch on the new PreviewState.mask_target instead — rename that local variable to remove the name collision, and add the Effect(i) branch writing clip.effects[i].mask. |
| modify | src/ui/app/preview_pane.rs | Pass Settings.hover_preview/canvas_snap and App's alt-render texture into PreviewCtx; consume PreviewResponse's new fields; wire r.canvas_rect one-frame-stale like timeline.lanes_rect. |
| create | src/ui/app/monitor.rs | AltRequest enum (Effect(EffectKind), Transition(TransitionKind), Gallery(tab: String, name: String) — reserved for inspector-gallery's wave-2 gallery.hover tool), AltRenderState, and tick(app,ctx) FRAME_HOOK: reads pending hover request, clones Project, calls player.request_layers(t,max_w), polls take_layers_reply, GPU-renders via gpu.render_preview_texture, coalesces to latest request, no-ops while self.export.is_some(). ALSO act() ACT_HANDLERS fn: AutoReframe, ToggleProxies, ViewerFit. |
| modify | src/ui/app/drops.rs | handle_drops(): add an on_monitor branch (pos inside self.preview.canvas_rect, checked before on_moodboard) calling the existing self.insert_at(ids, self.playhead, None). |
| modify | src/ui/guides.rs | Add canvas_snap(project, moving, playhead, cand, ...) -> (Pos2, Vec<CanvasGuide>) and paint_canvas_guides(); thread Palette into draw_guide's colours. |
| create | src/ui/app/tools_preview.rs | pub const TOOLS: &[ToolDef] for clip.crop, clip.fit, clip.mask_target, preview.hover, preview.view, preview.drop, timeline.reframe, playhead.set_timecode. preview.hover documented as the generic App.alt_render entry point that inspector-gallery's gallery.hover should target via AltRequest::Gallery. timeline.reframe requires an existing tracked box, errors cleanly when none exists. |
| modify | src/ui/app/panes.rs | NARROW, COORDINATED EDIT on a file source-monitor is the named sole wave-2 owner of for the arms it edits — source-monitor's plan declares itself sole wave-2 owner of the Pane::Preview/Library arms it touches in app/panes.rs (verified today at app.rs: Pane::Library match arm at line 2530, mask_for read at line 2697 inside the Pane::Effects arm at 2687; app.rs has not yet been split into app/panes.rs — that split is wave-0a's job). Accepting source-monitor's stated resolution: this PR's one-line mask_target set (alongside the existing self.tools.tool = Tool::Mask(shape) in the `if let Some(i) = resp.mask_for` block, app.rs:2697 today — self.preview.mask_target = MaskTarget::Effect(i)) lands as a same-day follow-up PR rebased onto source-monitor's merged panes.rs, never a concurrent same-file co-edit. |
| modify | src/ui/tools.rs | Glyph enum + ALL (229) + name() (314) + from_name() (404) + draw_glyph() (792): add Crop, Rotate — cursor-substitute icons painted at the pointer over crop/rotate handles. |
| modify | src/hotkeys.rs | ws:canvas-handles section: AutoReframe, ToggleProxies, ViewerFit, all sc(...)=None (unbound, rebindable, reachable via palette/menu/ui.action). |
| modify | src/settings.rs | + hover_preview: bool (default true), canvas_snap: bool (default true) at the struct/Default tail, #[serde(default)] via the struct-level attribute already present. |
| modify | src/ui/mod.rs | + pub(crate) fn parse_timecode(s: &str, fps: f64, cur: f64) -> Option<f64> beside timecode() (239): hh:mm:ss:ff, mm:ss, +N/-N frames, +1.5s/-2s. |
| modify | src/ui/app/mod.rs | ws:canvas-handles lines only: mod monitor;, one FRAME_HOOKS entry monitor::tick, one ACT_HANDLERS entry monitor::act, one TOOL_TABLES entry tools_preview::TOOLS. Do not touch any other section. |

## Model changes

- None. MaskTarget is UI-only state (declared in preview.rs, not persisted). Crop reuses existing EffectKind::Crop / P_CROP (model.rs:667,794,859) via the same find-or-append pattern already used for ToggleEffect (timeline.rs:2383-2396).

## Engine changes

- None new. Consumes engine::compose::placement (compose.rs:66-100), engine::gpu::Gpu::render_preview_texture (gpu.rs:373-393), and playback::Player::request_layers/take_layers_reply — the async API added by player-rate-loop (wave 1, dependency); if its exact signature differs, adapt monitor.rs's first commit to match. AutoReframe consumes engine::tracking::TrackJob::start(cx,cy,hw,hh) — verified this requires an explicit box with no auto-detect path, so the tool hard-requires an existing tracked box rather than inventing a 'highest-motion pick' heuristic.

## UI changes

- Selection outline gains hit-testable corner/edge/rotate handles + hover cursors (preview.rs)
- Optional crop-handle ring behind a context-menu 'Crop Handles' toggle
- Multi-select shows a union bounding box instead of one outline
- Snap guide lines painted during a canvas drag when Settings.canvas_snap is on
- Ctrl+wheel zoom / middle-drag pan of the canvas (Tool::Zoom's replacement, no strip button)
- Timecode label becomes a TextEdit on click
- Dropping a file on the monitor inserts it at the playhead
- Monitor substitutes a GPU alt-render texture while a hover request is pending
- Effects-pane 'Edit mask' on an effect row now routes the canvas mask drag to that effect's mask, not the clip's own mask (pending the panes.rs coordination note)

## New types and functions

- `pub(crate) enum Handle { Corner(u8), Edge(u8), Rotate, Crop(u8) }` — src/ui/preview.rs: Which handle owns the active drag; u8 indexes TL/TR/BR/BL or top/right/bottom/left.
- `struct HandleDrag { handle: Handle, id: Id, center: Pos2, ref_len: f32, start: (f64,f64,f64,f64), start_frac: [f64;4] }` — src/ui/preview.rs: Snapshot at drag_started so drag math is a ratio/delta from a fixed origin, never accumulated per-frame.
- `pub(crate) enum MaskTarget { Clip, #[default] Effect(usize) }` — src/ui/preview.rs: Which mask the EXISTING Tool::Mask drag block writes to; Default = Clip (today's behaviour, preserved).
- `pub(crate) fn apply_view(lb: Rect, view: (f32, Vec2)) -> Rect` — src/ui/preview.rs: Single choke point: scales+pans the letterbox rect for zoom/pan.
- `pub fn canvas_snap(project: &Project, moving: Id, playhead: f64, cand: Pos2, center: Pos2, px_per_proj: f32, thr_px: f32) -> (Pos2, Vec<CanvasGuide>)` — src/ui/guides.rs: Centre/edges/thirds/other visual clips' Placement::bounds() snapping for the drag-to-move gesture.
- `pub fn paint_canvas_guides(p: &egui::Painter, guides: &[CanvasGuide], rect: Rect, palette: &Palette)` — src/ui/guides.rs: Accent guide lines, painted only while a canvas drag is live.
- `pub(crate) fn parse_timecode(s: &str, fps: f64, cur: f64) -> Option<f64>` — src/ui/mod.rs: hh:mm:ss:ff / mm:ss / +N,-N frames / +1.5s,-2s -> absolute seconds.
- `pub(crate) enum AltRequest { Effect(EffectKind), Transition(TransitionKind), Gallery(String, String) }` — src/ui/app/monitor.rs: What the monitor should render instead of the live frame; Gallery(tab,name) reserved for inspector-gallery's gallery.hover.
- `pub(crate) struct AltRenderState { request: Option<AltRequest>, inflight: Option<(u64, AltRequest)>, ready: Option<(egui::TextureId,[u32;2])> }` — src/ui/app/monitor.rs: Coalescing one-in-flight state for App.alt_render.
- `pub(crate) fn tick(app: &mut App, ctx: &egui::Context)` — src/ui/app/monitor.rs: FRAME_HOOK: starts/polls/consumes one alt render on a cloned Project; no-ops while app.export.is_some().
- `pub(crate) fn act(app: &mut App, action: Action) -> bool` — src/ui/app/monitor.rs: ACT_HANDLERS entry: implements AutoReframe, ToggleProxies, ViewerFit; returns false for any other Action.

## Actions and hotkeys

| Action id | Label | Chord | Note |
|---|---|---|---|
| auto_reframe | Auto Reframe (follow point) |  | Unbound; menu/palette/ui.action only. Requires an existing tracked box on the clip; if none exists, App::enabled returns a toast reason instead of running. No subject/face detection. |
| toggle_proxies | Use Proxies |  | Unbound transport toggle; flips settings.use_proxies. Implemented in monitor::act, registered in app/mod.rs ACT_HANDLERS. |
| viewer_fit | Fit Viewer |  | Unbound (Shift+Z is timeline Zoom to Fit); resets PreviewState.view to (1.0, ZERO). Implemented in monitor::act. |

## New glyphs

- Crop
- Rotate

## Persisted fields

**Settings:**

- hover_preview: bool (default true) — allow effect/transition/look hover to drive the monitor
- canvas_snap: bool (default true) — canvas centre/edge/third/clip snapping while dragging

**Project (.sedit):**

- (none)

## MCP tools (required — every capability must be scriptable)

| Tool | Kind | Args | Description | Maps to |
|---|---|---|---|---|
| clip.crop | mutate | clip_id:integer:true:, left:number:false:0..0.5, right:number:false:, top:number:false:, bottom:number:false:, feather:number:false:, at:number:false:default playhead | Find-or-append EffectKind::Crop on the clip and set its fraction params at time at. | clip.effects find-or-append(Crop) + params[i].set_at (model.rs:794,859) |
| clip.fit | mutate | clip_id:integer:true:, mode:string:true:fit\|stretch | Fit (contain, native aspect) or stretch (fill canvas, non-uniform) the clip. | Project::fit_clip_to_screen(id, stretch) (model.rs:4600) |
| clip.mask_target | ui | effect:integer:false:omit = the clip's own mask | Point the existing Tool::Mask canvas drag at the clip mask or one effect's mask. | PreviewState.mask_target (preview.rs, new) |
| preview.hover | ui | kind:string:false:effect\|transition\|off, name:string:false:EffectKind/TransitionKind name | Manually drive the monitor's alt-render preview. Same App.alt_render pipeline that inspector-gallery's gallery.hover targets via AltRequest::Gallery. | App.alt_render request / monitor::AltRenderState |
| preview.view | ui | zoom:number:false:, pan_x:number:false:, pan_y:number:false:, fit:boolean:false:reset to 1.0/0,0 | Read or set the canvas zoom/pan. | PreviewState.view |
| preview.drop | mutate | asset_ids:array:true: | Place assets on a free video track at the playhead, as if dropped on the monitor. | App::insert_at(ids, playhead, None) (app.rs:1106) |
| timeline.reframe | job | clip_id:integer:true:, ratio:string:false:e.g. 9:16 | Auto-reframe using the clip's EXISTING tracked box only. Errors with a clear message if none exists. | engine::tracking existing TrackJob result + Project::apply_path (model.rs:4555), sign inverted |
| playhead.set_timecode | mutate | text:string:true:hh:mm:ss:ff \| mm:ss \| +N \| -N \| +1.5s | Parse a timecode/relative string and seek, same parser as the click-to-edit transport label. | ui::parse_timecode + App::seek |

**Luau:** Adds editor.tool() coverage for clip.crop / clip.fit / clip.mask_target / preview.hover / preview.view / preview.drop / timeline.reframe / playhead.set_timecode. AutoReframe/ToggleProxies/ViewerFit reachable via ui.action(...) and now have real ACT_HANDLERS bodies (monitor::act) so ui_action_covers_every_action passes. No new @on hook sources from this workstream. inspector-gallery's gallery.hover, once it lands, should reuse AltRequest::Gallery rather than exposing a second unrelated mechanism under Luau.

## Tests

| Test | File | Asserts |
|---|---|---|
| corner_handle_scales_uniformly_with_one_undo | src/ui/preview.rs | dragging a corner writes clip.scale (not scale_x/scale_y), undos==1, and the shape's centre does not move |
| edge_handle_writes_independent_scale_axis | src/ui/preview.rs | top/bottom edge writes scale_y only; left/right writes scale_x only |
| rotate_handle_snaps_to_15_degrees_with_shift | src/ui/preview.rs | without Shift rotation is continuous; with Shift held it lands on multiples of 15 |
| crop_handle_appends_exactly_one_crop_effect | src/ui/preview.rs | two sequential crop-edge drags leave exactly one EffectKind::Crop entry with both fractions written via set_at |
| crop_mode_off_never_touches_effects | src/ui/preview.rs | with crop_mode=false, dragging near an outline edge moves/scales the clip and clip.effects stays empty |
| mask_drag_targets_the_selected_effect_mask | src/ui/preview.rs | with mask_target=Effect(0), a mask drag writes clip.effects[0].mask, leaves clip.mask untouched |
| mask_drag_defaults_to_clip_mask_unchanged | src/ui/preview.rs | with mask_target left at Default (Clip), the rewired block still writes clip.mask exactly as before this PR |
| group_selection_shows_union_box_and_moves_every_clip | src/ui/preview.rs | 2 selected visual clips: outline == union of both bounds; drag moves both x/y; no handle is hit-testable |
| zoom_and_pan_never_edit_the_project | src/ui/preview.rs | Ctrl+wheel and middle-drag change PreviewState.view only; project JSON and undo count are unchanged |
| viewer_fit_resets_view | src/ui/app/monitor.rs | after zoom/pan, dispatching ViewerFit through monitor::act sets view back to (1.0, ZERO) and returns true |
| toggle_proxies_flips_setting | src/ui/app/monitor.rs | dispatching ToggleProxies through monitor::act flips settings.use_proxies and returns true |
| auto_reframe_without_tracked_box_is_disabled | src/ui/app/monitor.rs | App::enabled(AutoReframe) on a clip with no tracked box returns Err with a user-facing reason |
| auto_reframe_with_tracked_box_writes_apply_path | src/ui/app/monitor.rs | on a clip with an existing tracked box, AutoReframe runs TrackJob and writes an inverted apply_path; one undo entry |
| canvas_snap_centre_edges_thirds_and_other_clip | src/ui/guides.rs | a candidate near each guide type snaps exactly onto it and returns the matching CanvasGuide |
| canvas_snap_off_returns_no_guides | src/ui/guides.rs | canvas_snap disabled returns the raw candidate and an empty guide list |
| parse_timecode_parses_every_form | src/ui/mod.rs | hh:mm:ss:ff, mm:ss, +N/-N frames, +1.5s/-2s all resolve to expected seconds; garbage returns None |
| alt_render_coalesces_and_drops_stale_replies | src/ui/app/monitor.rs | two requests in one tick leave exactly one inflight (the latest); a mismatched reply id is discarded |
| alt_render_paused_during_export | src/ui/app/monitor.rs | tick() with app.export.is_some() starts no new request and leaves ready state untouched |
| hover_preview_never_mutates_project_or_undo | src/ui/app/monitor.rs | after a full hover request/reply cycle, project.to_json() and undo count are identical to before |
| drop_onto_monitor_places_on_a_free_track_at_playhead | src/ui/app/drops.rs | a drop inside preview.canvas_rect inserts the asset at self.playhead on a free video track |
| every_glyph_paints_a_picture | src/ui/tools.rs | existing test, now also covers Glyph::Crop and Glyph::Rotate |
| ui_action_covers_every_action | src/ui/app/tools_registry_tests.rs | existing structural test now also passes for AutoReframe/ToggleProxies/ViewerFit via monitor::act |

## Verification checklist

- [ ] cargo test — full suite green, all new tests pass, no existing preview.rs test regresses
- [ ] cargo run -- --selftest — idle step shows no stray repaint from handles/zoom/alt-render when nothing is hovered or dragging
- [ ] scripts/size.ps1 -Note canvas-handles-monitor — delta within ~108 KB or the PR states the reason
- [ ] manual: drag every handle kind with Shift held on rotate; toggle Crop Handles and drag two edges — one Crop entry, correct History label
- [ ] manual: select 2 clips — union box only, both move together, no transform handles
- [ ] manual: drop a file onto the preview — lands on a free track at the playhead
- [ ] manual: Ctrl+wheel zoom + middle-drag pan, then ViewerFit — resets cleanly, no project edit
- [ ] manual: click the timecode label, type +48 and Enter — seeks 48 frames forward
- [ ] manual: start an export, hold hover over an effect card — monitor keeps the live export-safe frame
- [ ] manual: dispatch AutoReframe on a clip with no tracked box — toast explains why; track it and retry — one undo entry
- [ ] screenshot: handles + crop ring + group box + snap guides mid-drag, docked and fullscreen

## Acceptance criteria

- [ ] Corner/edge/rotate drags write scale/scale_x/scale_y/rotation with exactly one undo per gesture, zero when nothing changed
- [ ] Crop Handles toggle + edge drag appends exactly one Crop effect and writes its fractions via set_at; visible History label 'Crop'
- [ ] 2+ selected visual clips show one union bounding box and move together; no transform/crop/rotate handles shown
- [ ] The existing Tool::Mask drag block, rewired through PreviewState.mask_target, edits clip.effects[i].mask when mask_target=Effect(i) and clip.mask by default
- [ ] Hovering an effect/transition card substitutes the monitor's frame with zero project mutation and zero new undo entries
- [ ] Alt renders never start while an export is running
- [ ] Dropping a file on the monitor places it on a free video track at the playhead
- [ ] Ctrl+wheel zoom and middle-drag pan work with no new Tool and never touch project data; ViewerFit resets them
- [ ] Clicking the timecode label accepts hh:mm:ss:ff / mm:ss / ±N frames / ±N.Ns and seeks
- [ ] AutoReframe runs only against an existing tracked box, toasts a clear reason when none exists
- [ ] ToggleProxies and ViewerFit are real, registered ACT_HANDLERS entries
- [ ] AltRequest::Gallery is reserved and documented so inspector-gallery's gallery.hover targets this mechanism
- [ ] cargo test and --selftest are green; every_glyph_paints_a_picture covers Crop and Rotate; ui_action_covers_every_action passes; release size delta within ~110 KB or justified

## Risks

| Risk | Mitigation |
|---|---|
| app/panes.rs (Pane::Effects mask_for wiring) is not this workstream's file this wave — source-monitor is the sole wave-2 owner of the Pane::Preview/Library arms it edits in the same file (verified today: Pane::Library at 2530, mask_for read at 2697). | Land everything else in this PR; ship the panes.rs one-line mask_target set as a same-day follow-up PR rebased onto source-monitor's merged panes.rs, per source-monitor's stated resolution — never a concurrent same-file co-edit. |
| inspector-gallery's planned gallery.hover tool cites a non-existent App.hover_preview; if it lands before reconciling with AltRequest::Gallery, two incompatible hover mechanisms could ship. | This PR reserves AltRequest::Gallery(tab,name); call this out in both PRs' descriptions and land whichever merges second with the reconciled arg shape. |
| Data model scales/rotates about the clip centre only (no anchor field), so Shift/Alt semantics have no natural per-corner mapping. | Scope Shift to the Rotate handle's 15° snap only; ponytail-note the ceiling. |
| Crop-as-effect can silently add a stack entry the user didn't expect. | Gate crop handles behind an explicit off-by-default 'Crop Handles' toggle; label the History entry 'Crop'. |
| Widening the single-clip .find() and renaming the existing Tool::Mask block's local variable could change existing single-selection/mask test behaviour. | Gate transform/crop/rotate to selection.len()==1; group box/move only for 2+; add mask_drag_defaults_to_clip_mask_unchanged as a regression guard. |
| monitor.rs depends on player-rate-loop's request_layers/take_layers_reply, whose exact signature isn't fixed yet. | Confirm the merged signature at worktree creation time; adapt monitor.rs's first commit, not a redesign. |
| AutoReframe was originally described with a 'highest-motion pick' fallback that does not exist anywhere in engine/. | Scope to existing-tracked-box-only; App::enabled returns a toast reason when none exists. |
| wave-0b's alt_render App-field stub type may not match AltRequest/AltRenderState exactly. | First commit retypes that one field if needed — isolated, called out in the PR description. |

## Suggested implementation order

1. 1. tools.rs: Glyph::Crop/Rotate (enum, ALL, name, from_name, draw_glyph)
2. 2. hotkeys.rs: AutoReframe/ToggleProxies/ViewerFit (unbound) in ws:canvas-handles
3. 3. settings.rs: hover_preview/canvas_snap fields + Default + round-trip test
4. 4. guides.rs: canvas_snap()/paint_canvas_guides(), thread Palette into draw_guide
5. 5. ui/mod.rs: parse_timecode() + unit test
6. 6. preview.rs: Handle/MaskTarget/HandleDrag types, PreviewState fields, hit-test+drag, crop toggle+drag, group-box widening, cursor icons, apply_view() zoom/pan, timecode click-to-edit, PreviewCtx/PreviewResponse fields
7. 7. preview.rs: rewire the existing Tool::Mask drag block onto PreviewState.mask_target, renaming its local Option<Id> variable; add mask_drag_defaults_to_clip_mask_unchanged regression test
8. 8. app/preview_pane.rs: wire new PreviewCtx inputs/PreviewResponse outputs, canvas_rect capture
9. 9. app/drops.rs: on_monitor branch via preview.canvas_rect + existing insert_at
10. 10. app/monitor.rs: AltRequest (incl. Gallery variant)/AltRenderState + tick() FRAME_HOOK + act() ACT_HANDLERS body
11. 11. app/mod.rs: mod monitor; + FRAME_HOOKS + ACT_HANDLERS + TOOL_TABLES lines (ws:canvas-handles section only)
12. 12. app/tools_preview.rs: 8 ToolDef rows, timeline.reframe erroring cleanly with no tracked box
13. 13. Land the panes.rs one-line mask_target set as a same-day follow-up PR rebased onto source-monitor's merged panes.rs, per source-monitor's stated sole-ownership resolution — never a concurrent same-file co-edit
14. 14. Full cargo test + --selftest + scripts/size.ps1 + screenshots, then PR

## Deliberate simplifications (`// ponytail:`)

- Shift/Alt on corner+edge handles skipped (no anchor field in the model); Shift kept only for Rotate's 15° snap.
- Drop-onto-monitor reuses the existing App::insert_at, not the wave-0b place_asset stub — no hard dependency on source-monitor landing first.
- Crop fractions are written via the same Animated::set_at convention as everything else — no separate static/animated UI.
- Numeric timecode entry only seeks; trim-delta-at-an-edit-point is the timeline ruler's job once snap-engine/trim-model land.
- AltRequest's shape may need retyping to match whatever minimal stub wave-0b actually landed for App.alt_render.
- AutoReframe deliberately has no motion/saliency fallback (none exists in-tree) — ceiling is 'centre-box tracking only'.
- The panes.rs mask_target hook-fill is deliberately deferred to a same-day follow-up PR rebased onto source-monitor's merged panes.rs, per source-monitor's stated sole-ownership resolution, rather than a concurrent same-file co-edit.
- AltRequest::Gallery is a stub reservation only — this PR does not build inspector-gallery's UI trigger, just the enum arm and doc comment.

## Review trail

- Finding 1 (ownership, panes.rs): panes.rs is not in this workstream's wave-2 grant. Kept the Pane::Effects mask_target line as the smallest possible diff, moved to the explicit final step (13/14) instead of mid-sequence, and added to files[], risks[], and a ponytail_note.
- Finding 2 (missing ACT_HANDLERS): app/mod.rs's file entry now includes ACT_HANDLERS. Added a real monitor::act() handler implementing AutoReframe/ToggleProxies/ViewerFit, added it to new_types_and_fns, and three matching tests.
- Finding 3 (mask-target rewire not scoped): rewiring the pre-existing Tool::Mask drag block is listed explicitly in preview.rs's files[] diff, with the variable rename spelled out, a regression test, and a risk entry.
- Finding 4 (invented reframe heuristic): confirmed engine::tracking::TrackJob::start requires an explicit box with no auto-detect path and no motion/saliency heuristic exists in-tree. AutoReframe requires an existing tracked box, surfaces absence via App::enabled/toast; documented in ponytail_notes, scope_out, and two tests.
- Audit fix A (panes.rs cross-plan collision, major): re-verified Pane::Library at 2530, mask_for at 2697 against current app.rs; app.rs not yet split (wave 0a pending). Broadened the panes.rs entry, risk, and implementation_order step 13 to name source-monitor explicitly.
- Audit fix B (gallery.hover / hover_preview mismatch, blocker): confirmed no workstream declares App.hover_preview. Added AltRequest::Gallery(tab,name) as a reserved variant across files[], new_types_and_fns, mcp_tools, luau, scope_out, a risk entry, an acceptance criterion, and a ponytail_note.
- reconcile: (major) accepted source-monitor's stated resolution for the panes.rs conflict verbatim — dropped this plan's hedge between a skeleton-grant, a wave-0b hook, or a deferred follow-up, and now states the single resolution both plans share: the one-line mask_target set lands as a same-day follow-up PR rebased onto source-monitor's merged panes.rs. Updated the panes.rs files[] entry, its risk entry, the matching ponytail_note, and implementation_order step 13 to match; re-verified the cited line numbers (Pane::Library:2530, mask_for:2697) against current app.rs, unchanged.
- reconcile: (minor, no change) checked this plan's own AltRequest declaration against the residual-fix claim that pro-monitor's plan cites a nonexistent 'AltUse/AltReq' symbol — this workstream's type was already correctly named AltRequest/AltRenderState throughout (files[], new_types_and_fns, mcp_tools, luau, changelog Audit fix B); grepped src/ and confirmed no AltUse/AltReq symbol exists anywhere in-tree. The mismatch is entirely on pro-monitor's side; nothing in this plan needed correction.
