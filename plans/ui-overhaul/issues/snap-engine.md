# Timeline snap engine, cursor zones, edit-point seams, in/out handles, cue drag, middle-mouse pan

**Workstream:** `snap-engine` · **Issue:** [#22](https://github.com/KashTheKing/simple-editor/issues/22) · **Wave:** 1 · **Branch/worktree:** `feat/snap-engine` → `../simple-editor-wt/snap-engine` · **Depends on:** size-diet · **~1165 new lines · Δ exe ≈ +96 KB**

Read [agents.md](../../../agents.md), [goals.md](../../../goals.md), [notes.md](../../../notes.md) and the master plan [README.md](../README.md) (registry protocol, keymap, modifier table) before starting.

## Summary

Everything on the timeline that needs no trim primitive, scoped to the skeleton's actual wave-1 assignment (narrower than the p2 drafts — view presets/minimap/dupe/rename moved to wave-3 pro-timeline; gap-select/rate-stretch/drops moved to wave-2 trim-gestures). Ships: a tiered snap.rs (playhead>cursor>selected-edge>adjacent-edge>marker>transition-edge>in/out>zero) with a zoom+fps-aware threshold and a visible accent guide during any existing drag; a frozen, fully-tested arm(mods,zone,flags,tool)->GestureKind decision table in arm.rs that wave-2/3 will wire the rest of; clip-body top/bottom cursor zones with click-to-split on tall rows; seam click -> EditPoint selection; draggable snapped in/out ruler handles; a fixed, snapped, undo-on-release cue-body drag; middle-mouse pan; an empty-timeline hint; and the audit-flagged fix to Body/Shift click-to-add-to-selection, which today is unimplemented and identical to a plain click. All read-only against the model (no Project/Track/Clip changes) — pure UI-layer additions plus 5 MCP tools and 2 unused-until-wave-2 cursor glyphs.

## Motivation

Serves goals.md's #1 Resolve complaint (visible snap guide) and the skeleton's snap-engine workstream verbatim: rows 8,19-23,29(UI half),41,51 of the gap matrix plus critique risk 0 (frozen modifier table) and risk 14 (explicit cursor tier) and critique-18 (middle-mouse pan). Reconciles infra's trim-model-snapping, beginner's snap-engine and pro's timeline-core designs down to the skeleton's narrower wave-1 scope. Additionally closes a source-verified audit gap: the modifier_table's Body/Shift row ('click = add link group to selection') has no matching code anywhere in the plan set, so it was silently ornamental — fixed here since snap-engine is the wave-1 owner already touching this exact click-handling block.

## In scope

- Tiered snap engine (snap.rs) with markers/transition-edge candidates and a zoom+fps-aware threshold
- Visible accent guide line while any existing gesture snaps
- Frozen arm(mods,zone,flags,tool)->GestureKind decision table with full row-by-row test coverage
- Clip-body top/bottom cursor zones + hairline click-to-split on tall rows
- Body/Shift click-to-add-to-selection fix: explicit mods.shift arm in the click-handling block (today's timeline.rs:2318-2336), previously missing so Shift+click behaved identically to a plain click
- Seam detection + EditPoint/Side click-selection
- Draggable, snapped, clamped ruler in/out handles
- Subtitle cue body drag + fixed release-only undo, both snapped
- Middle-mouse timeline panning
- Empty-timeline hint
- RollCursor/SlipCursor glyph registration (unused pending wave 2)
- 5 MCP ToolDef rows for the above

## Out of scope

- Implementing Ripple/Roll/Slip/Slide/Segment/RateStretch gestures themselves (timeline-trim-gestures, wave 2)
- Gap selection + Close Gap, drop modifiers/DropMode (timeline-trim-gestures, wave 2)
- Adapting the drag-start auto-select block (today's :2500-2507, currently `if !mods.ctrl` only) for Ctrl+Alt/Ctrl+Shift/Ctrl+edge Edge- and Drop-zone meanings — those zones stay tested-but-not-yet-called (arm() covers them) until timeline-trim-gestures wires Edge/Drop in wave 2, so their selection-at-drag-start interaction is that workstream's problem to name, not introduced or worsened here
- Timeline view presets, overview minimap, boring/dupe paint passes, track header rename/colour/reorder (pro-timeline, wave 3)
- Track.locked/ripple/magnetic field definitions (registries-schema-hooks, wave 0b) and the trim primitives that consume them (trim-model, wave 1 sibling)
- Any hotkey/Action row or chord — this workstream owns no keymap entries

## Files

| Op | Path | What |
|---|---|---|
| create | src/ui/timeline/snap.rs | Post-wave0a mod.rs still holds nearest/snap_target/snap_playhead/snap_time (current timeline.rs:443-480, SNAP_PX at :67). Move them here, add SnapKind, tiered snap::target(), snap_thr(zoom,fps), marker_candidates()/transition_edge_candidates() iterators. Old 5-arg snap_target/snap_time/snap_playhead keep their exact signatures as thin wrappers over target() so all 11 existing mod.rs call sites (today's lines 1471,1476,1488,2123,2132,2170,2620,2657,2691,2809,2833) need zero signature changes. |
| create | src/ui/timeline/arm.rs | New. Zone enum, TrackFlags{locked,ripple,magnetic}, GestureKind enum, pure fn arm(mods: Modifiers, zone: Zone, flags: TrackFlags, tool: Tool) -> Option<GestureKind> encoding the skeleton's frozen modifier_table (Body/Shift row included, returning MoveNoOverlap for the drag case; the click-only add-to-selection behavior is a separate selection update in mod.rs, not a drag GestureKind). One #[test] per table row (~26). |
| modify | src/ui/timeline/cue_lane.rs | Already exists post wave-0a (pure move of today's timeline.rs:1900-2077 subtitle-lane code). Replace sub_trim:Option<(Id,bool)> with CueDrag{id,op:CueOp{Trim{right:bool},Move},before:Project,changed:bool}; cue body Sense::click()->click_and_drag() (today's line 1984); route trim/move through snap::target instead of raw state.time_at (today's 2064-2080); move the undo push from drag-start (today's 2027-2028 bug) to release-if-changed. |
| modify | src/ui/timeline/mod.rs | Exclusive-wave-1 owner. Drag.snapped:Option<f64> + 1-line capture at the 5 arms that already call snap_target (today's 2620,2657,2691,2809,2833) + guide vline paint inside the existing `if let (Some(drag),Some(pos))` block (after the match, before its close) + at the dnd ghost (today's ~2172-2208). Split clip body interact (today's ~1462-1476) into top/bottom sub-rects when rect.height()>=2*MIN_TRACK_H: top=Grab+existing select/move, bottom=Crosshair+hairline-at-snap(pointer)+click=Act::SplitAt (Tool::Cut/Marker unchanged, still whole-body). AUDIT FIX: in the click-handling block (today's :2318-2336, `if mods.ctrl {...} else if mods.alt {...} else {expand_links}`), add an explicit `else if mods.shift` arm before the plain-click fallback that pushes the clicked clip's expand_links group into `state.sel`/selection without clearing the existing selection (additive, distinct from Ctrl's toggle-out) — previously absent, so Shift+click on a clip body was indistinguishable from a plain click. Add EditPoint{track,t,side:Side}, TimelineState.edit_point, seam detection (adjacent same-track clips whose end/start times coincide) + click sets edit_point via arm(Zone::Seam,...). Add Gesture::InOut{out:bool,changed:bool} + interact rects on the ruler at x_at(in)/x_at(out) (after ruler markers, today's ~1876-1904) driving p.in_point/out_point through snap::target, clamped in<=out, right-click clears. Middle-mouse pan: extend the existing lanes_resp handling (today's ~2127-2142) with dragged_by(PointerButton::Middle) adjusting scroll_x/scroll_y, gated on state.drag.is_none(). Empty-state hint: in show(), when c.project.tracks.iter().all(\|t\| t.clips.is_empty()), paint a dashed rect + weak label over `lanes`. |
| modify | src/ui/timeline/tests.rs | Append new tests (list below); extend every_tool_snaps (today's :3216) and headless_1000_clips_stays_fast (today's :3850) with marker/transition-edge and zone/hairline cases; update the one cue-undo test whose count deliberately changes. |
| modify | src/ui/app/tools_timeline.rs | Add pub const TOOLS: &[ToolDef] rows: timeline.snap_get, timeline.snap_set, timeline.snap_query, timeline.zones, timeline.set_in_out (bodies call into snap::target/arm/Project.in_point/out_point). |
| modify | src/ui/tools.rs | ws:snap-engine section in Glyph enum + ALL + name()/from_name()/draw_glyph(): add RollCursor, SlipCursor (small painted glyphs; unused by any drag until wave-2 wires Roll/Slip). |
| modify | src/settings.rs | ws:snap-engine section: `pub snap_markers: bool` on Settings, relying on the container-level `#[serde(default)]` already at line 143 (same as the sibling `snap` field, which carries no per-field attribute) + `snap_markers: true` in impl Default for Settings; gates markers/clip-markers as snap candidates in snap::target(). |
| modify | src/ui/app/mod.rs | ws:snap-engine line in TOOL_TABLES appending tools_timeline::TOOLS (one line, only if not already present from the wave-0 stub). |

## Model changes

- None. No Project/Clip/Track field or fn changes — this workstream only reads existing pub fields (Project.markers, Track.transitions, Clip.markers, in_point/out_point) from the UI layer.

## Engine changes

- None — this workstream is UI-layer only (src/ui/timeline/*); it reads existing Project fields (markers, tracks[].transitions, in_point/out_point) but adds no engine/model API.

## UI changes

- Accent guide vline painted while any drag (Move/Trim/Stretch/Marker/Spacer/new InOut/new cue-move) is snapped to a candidate, and at the dnd asset-drop ghost time.
- Clip body splits into a top Grab/move zone and a bottom Crosshair/hairline/click-to-split zone on rows >= 2x MIN_TRACK_H; unchanged single zone on shorter rows.
- Shift-click on a clip body now adds its link group to the current selection without clearing it (previously a no-op distinct from plain click; fixes the audit-flagged gap).
- Seam (two abutting clip edges) becomes clickable, selecting an EditPoint with Both/Left/Right side per plain/Ctrl/Alt.
- Ruler grows draggable in/out handles (snapped, clamped, right-click clears).
- Subtitle cue lane: cue bodies become draggable (snapped); cue-edge trim's undo timing is fixed.
- Middle-mouse drag pans the timeline.
- Empty timeline shows a dashed drop-hint.
- Two new (currently unused) cursor glyphs, RollCursor and SlipCursor, added to the shared Glyph catalogue.

## New types and functions

- `pub enum SnapKind { Zero, Playhead, Cursor, SelectedEdge, ClipEdge, Marker, TransitionEdge, InOut }` — src/ui/timeline/snap.rs: Which tier a hit came from; used by timeline.snap_query and internal tier ordering.
- `pub(crate) fn target(p: &Project, t: f64, thr: f64, playhead: f64, exclude: &[Id], selected: &[Id], cursor: Option<f64>) -> Option<(f64, SnapKind)>` — src/ui/timeline/snap.rs: Tiered engine: tries playhead, then cursor, then selected-clip edges, then adjacent-clip edges, then markers, then transition edges, then in/out/zero, in order; first tier with a hit inside thr wins (nearest within that tier).
- `pub(crate) fn snap_target(t: f64, thr: f64, p: &Project, playhead: f64, exclude: &[Id]) -> Option<f64>` — src/ui/timeline/snap.rs: Back-compat wrapper over target() with selected=&[] cursor=None; keeps the 11 existing call sites' signature and return type unchanged while gaining markers/transitions for free.
- `pub(crate) fn snap_thr(zoom: f32, fps: f64) -> f64` — src/ui/timeline/snap.rs: Replaces the flat SNAP_PX/zoom constant; shrinks with zoom and floors at one physical frame (1/fps) so far-zoomed-in drags are frame-quantised only, never magnetic.
- `fn marker_candidates(p: &Project) -> impl Iterator<Item = f64> + '_; fn transition_edge_candidates(p: &Project) -> impl Iterator<Item = f64> + '_` — src/ui/timeline/snap.rs: Project markers filtered to p.editing + every clip's local markers (c.start + m.t), and every transition's (cut-half, cut+half) window edges via Track::transition_clips + Transition::cut_half.
- `pub enum Zone { Body, BodyBottom, EdgeStart, EdgeEnd, Seam, Fade, VolumeLine, Key, Marker, TransitionEdge, Lane, RulerInOut, Drop, LegacyTool }  pub struct TrackFlags { pub locked: bool, pub ripple: bool, pub magnetic: bool }  pub enum GestureKind { MoveNoOverlap, MagneticMove, Slip, Slide, Segment, Trim, RippleTrim, Roll, RateStretch, MultiRippleTrim, SplitAt, SeamBoth, SeamLeft, SeamRight, SeamAddToSet, GapSelect, RubberBandAdd, Pan, RulerInOutDrag, DropSplice, DropOverwrite, DropPlaceOnTop, DropDefault, LegacyToolGesture }` — src/ui/timeline/arm.rs: The frozen modifier table as data + one pure decision fn, including the Body/Shift row (drag=MoveNoOverlap); wave-2/3 gestures.rs consumes GestureKind for zones this workstream does not itself wire up (Edge modifiers, Drop, Lane gap-select).
- `pub fn arm(mods: egui::Modifiers, zone: Zone, flags: TrackFlags, tool: Tool) -> Option<GestureKind>` — src/ui/timeline/arm.rs: Pure lookup over the skeleton's modifier_table; one test per row pins it before any gesture ships.
- `pub struct EditPoint { pub track: usize, pub t: f64, pub side: Side }  pub enum Side { Left, Right, Both }  pub edit_point: Option<EditPoint> (field on TimelineState)` — src/ui/timeline/mod.rs: Seam-click selection consumed later by trim-model's keyboard trim actions (U/Shift+U/extend/etc.) in a different, already-existing file.
- `struct CueDrag { id: Id, op: CueOp, before: Project, changed: bool }  enum CueOp { Trim { right: bool }, Move }` — src/ui/timeline/cue_lane.rs: Replaces sub_trim:Option<(Id,bool)>; unifies cue trim and the new cue move under the same before-snapshot + release-if-changed undo convention already used by clip gestures, fixing the undo-at-drag-start bug.

## New glyphs

- RollCursor
- SlipCursor

## Persisted fields

**Settings:**

- snap_markers: bool (default true) — ws:snap-engine section on Settings; when false, snap.rs's marker/clip-marker candidates are skipped. No per-field serde attribute: Settings already carries a container-level #[serde(default)], same as the sibling `snap` field.

**Project (.sedit):**

- (none)

## MCP tools (required — every capability must be scriptable)

| Tool | Kind | Args | Description | Maps to |
|---|---|---|---|---|
| timeline.snap_get | read | none | Current snapping-enabled state. | Settings.snap |
| timeline.snap_set | ui | enabled:bool:true:turn snapping on/off | Toggle snapping (mirrors the bare-S hotkey); not project data, no undo. | Settings.snap |
| timeline.snap_query | read | t:f64:true:pointer time to test; exclude_ids:array<u64>:false:clip ids to exclude from candidates | Runs the tiered snap engine and returns the hit and its tier. | snap::target() -> {t, kind} |
| timeline.zones | read | x:f32:true:screen x; y:f32:true:screen y | Debug hit-test: which Zone a point would arm (Body/BodyBottom/Edge/Seam/Lane/RulerInOut/...). | arm.rs Zone hit-test + arm() |
| timeline.set_in_out | mutate | in:f64:false:new in point (seconds); out:f64:false:new out point (seconds) | Sets in/out, snapped, clamped in<=out; one undo pushed only if changed. | Project.in_point / Project.out_point |

**Luau:** No new Luau surface beyond the 5 ToolDefs' standard editor.tool(name,args) exposure (auto-flattened into editor.tools()/tools/list/palette per the wave-0b registry). timeline.snap_query and timeline.zones are read-only and safe for `-- @on selection_changed` scripts that want to align a ghost preview to the same candidates the UI snaps to.

## Tests

| Test | File | Asserts |
|---|---|---|
| snap_guide_line_paints_mid_drag | src/ui/timeline/tests.rs | Dragging a clip within thr of a candidate paints exactly one accent-colour LineSegment at x_at(target); dragging with nothing in range paints none. |
| tiered_snap_prefers_playhead_over_far_edge | src/ui/timeline/snap.rs | With the playhead and a clip edge both plausibly within a wide thr, target() returns the playhead (SnapKind::Playhead), not the edge. |
| markers_and_transitions_are_snap_candidates | src/ui/timeline/tests.rs | A project marker and a transition edge each produce a hit within thr; setting snap_markers=false removes the marker hit but not the transition one. |
| snap_thr_shrinks_with_zoom_and_floors_at_one_frame | src/ui/timeline/snap.rs | snap_thr(zoom,fps) decreases as zoom increases and never returns a value finer than 1/fps. |
| bottom_zone_click_splits_top_zone_moves | src/ui/timeline/tests.rs | On a row >=2x MIN_TRACK_H, clicking the lower half of a clip body splits it at the snapped hairline time; clicking+dragging the upper half moves it; on a default-height row the whole body still moves on drag (no split). |
| shift_click_adds_link_group_without_clearing | src/ui/timeline/tests.rs | AUDIT FIX regression test. Given an existing selection {A}, Shift-clicking clip B's body (B not linked to A) results in selection {A, B} (B's full link group added, A retained); Ctrl-click on a selected clip still toggles it out (unchanged); a plain click still replaces selection with the clicked link group (unchanged). |
| seam_click_selects_edit_point_sides | src/ui/timeline/tests.rs | Plain/Ctrl/Alt clicks on a seam between two abutting clips set TimelineState.edit_point to Side::Both/Left/Right respectively. |
| inout_handles_drag_and_clamp | src/ui/timeline/tests.rs | Dragging the in handle past the out point clamps in<=out; exactly one undo is pushed if the value changed, zero if released at the same time. |
| cue_body_drag_one_undo_only_if_moved | src/ui/timeline/cue_lane.rs | Dragging a cue body and releasing at the press point pushes 0 undos; moving it pushes exactly 1 and the new position is frame/marker-snapped. |
| cue_trim_undo_on_release_not_press | src/ui/timeline/cue_lane.rs | Trimming a cue edge and releasing without moving it pushes 0 undos (previously pushed 1 at drag start). |
| middle_mouse_pans_without_selecting | src/ui/timeline/tests.rs | A middle-button drag on the lanes changes scroll_x/scroll_y, starts no Move/band gesture, and leaves selection untouched. |
| empty_timeline_shows_hint | src/ui/timeline/tests.rs | A project with zero clips paints the dashed-rect hint text; adding one clip removes it. |
| arm_modifier_table_rows | src/ui/timeline/arm.rs | One assertion per row of the skeleton's modifier_table, including Body/Shift: given (mods, zone, flags, tool), arm() returns the documented GestureKind or None. |
| every_glyph_paints_a_picture | src/ui/tools.rs | Existing structural test extended for free: RollCursor and SlipCursor are in Glyph::ALL and draw_glyph paints something non-empty for each. |
| headless_1000_clips_stays_fast | src/ui/timeline/tests.rs | Existing perf test re-asserted < 10 ms/frame with zone-split, hairline and seam hit-testing added, confirming they're gated on the existing `detailed` flag. |
| every_tool_snaps | src/ui/timeline/tests.rs | Existing test extended with a marker-candidate and a transition-edge-candidate case alongside its current in/out-point case. |

## Verification checklist

- [ ] cargo test — full suite green; only the one named cue-undo assertion changes value, nothing else regresses
- [ ] cargo test --lib timeline:: (or a name substring like `cargo test snap_` / `cargo test arm_` / `cargo test shift_click`) — new snap.rs/arm.rs/timeline tests above all pass, arm_modifier_table_rows covers every skeleton row incl. Body/Shift; note cargo test filters match the fully-qualified test path (module::tests::name), not filesystem paths, so a `/`-containing filter matches nothing
- [ ] cargo run -- --selftest — idle-repaint step stays green with guide/hairline/hint all hidden at rest
- [ ] Manual/harness screenshot mid-drag: accent guide visible at the snapped x; disappears once zoom passes 1 frame per pixel and only frame-quantisation remains
- [ ] Manual: with clip A selected, Shift-click clip B — both selected; Ctrl-click B again — B toggles out, A remains; plain-click A — selection collapses to A alone (confirms the audit fix didn't regress Ctrl/plain paths)
- [ ] MCP: tools/list gains exactly timeline.snap_get/snap_set/snap_query/zones/set_in_out; each round-trips through run_tool_undoable (set_in_out pushes 0/1 undo correctly; the other 4 push none)
- [ ] scripts/size.ps1 -Note snap-engine — delta within the ~95 KB budget; PR body carries a `size:` line
- [ ] cargo fmt --check on touched files only (do not reformat the whole crate)

## Acceptance criteria

- [ ] Dragging a clip/trim/stretch/marker/spacer near a candidate (playhead, cursor, selected/adjacent edges, marker, transition edge, in/out, 0) shows one accent vline for the whole gesture and disappears on release; no vline when nothing is within thr.
- [ ] snap::target() tries tiers in order (playhead>cursor>selected-edge>adjacent-edge>marker>transition-edge>in/out>zero) and returns the first tier's nearest hit within thr; markers/transition edges are candidates project-wide for the open sequence only.
- [ ] snap_thr(zoom,fps) shrinks with zoom and floors at 1 physical frame (sub-frame cutoff); existing 11 snap_target/snap_time/snap_playhead call sites compile unchanged and keep passing their pinned tests.
- [ ] On rows >=2x MIN_TRACK_H, hovering a clip's bottom half shows a crosshair+hairline snapped to the pointer time and a plain click there splits that one clip/track; the top half (and the whole clip on short rows) keeps existing select+drag-to-move behaviour untouched.
- [ ] Shift-click on a clip body adds its link group to the current selection without clearing it (audit fix); Ctrl-click's toggle-out and plain click's replace-selection behavior are unchanged and covered by regression tests.
- [ ] Clicking a seam (two clips' abutting edges) sets TimelineState.edit_point to Side::Both; Ctrl-click sets Left, Alt-click sets Right; nothing else on the timeline changes selection.
- [ ] Ruler in/out handles are draggable (snapped), clamp in<=out, push exactly one undo per gesture only if the value actually changed, and right-click clears that mark.
- [ ] Dragging a subtitle cue body moves it (snapped) with 0 undos if released at the press point and exactly 1 if moved; the existing edge-trim's undo-at-drag-start bug is fixed to the same undo-on-release-if-changed rule.
- [ ] Middle-mouse drag on the lane area pans scroll_x/scroll_y and starts no selection/move/band gesture.
- [ ] An empty project (zero clips across all tracks) paints a dashed-rect hint with an Open/Import hotkey hint; it disappears once any clip exists.
- [ ] arm(mods, zone, flags, tool) is a pure fn in arm.rs with one passing unit test per modifier_table row from the skeleton including Body/Shift, compiled and exercised even though wave-2/3 wire most of its GestureKind results into real drags.
- [ ] cargo test passes with the existing 657+ tests unchanged in name/count except the one deliberately-updated cue-undo assertion, plus all new tests below; headless_1000_clips_stays_fast stays < 10 ms with hairline/zone/seam checks added; every_glyph_paints_a_picture covers RollCursor/SlipCursor; MCP tools/list gains exactly 5 rows, each round-tripping through run_tool_undoable.

## Risks

| Risk | Mitigation |
|---|---|
| Splitting the clip body interact into top/bottom sub-rects could steal hit-test priority from the marker-flag rects, which today are deliberately registered AFTER the body specifically so small flags win (comment at today's timeline.rs:1449). | Keep the exact same call-site position for the (now two) body interacts; do not move the later marker_hits registration. Add a regression test that a marker flag still wins a click over an underlying bottom-zone hairline. |
| Changing thr from the flat SNAP_PX/zoom constant to snap_thr(zoom,fps) could shift results in the two existing pinned unit tests (nearest_within_threshold, snap_targets_exclude_moving_clips) that assert exact thresholds. | snap_thr matches the old formula exactly except for the new high-zoom floor; re-run both tests unchanged first and only adjust if the floor actually triggers at their zoom (it won't at typical test zooms). |
| cue_lane.rs's undo-timing fix deliberately changes a currently-passing pinned assertion (undo pushed at drag start, not release). | Named explicitly in this plan and the PR body; update that one assertion to 'undo only on release if changed', do not silently let it regress. |
| 11 existing snap_target/snap_time/snap_playhead call sites are compiled against the back-compat wrapper; a future workstream editing mod.rs's gesture match block without noticing snap.rs's split could reintroduce the old inline fns. | Delete the old fn bodies from mod.rs in the same commit that creates snap.rs (compiler catches any leftover duplicate immediately). |
| Adding Drag.snapped and a guide-paint call could perturb the wave-0a pixel-identical screenshot baseline other workstreams rely on. | Guide only paints while state.drag.is_some() (an active gesture); the at-rest screenshot used for baselines has no drag in progress, so it is unaffected. |
| AUDIT FIX RISK: adding a Shift branch to the click-handling `if/else if` chain could shadow or reorder the existing Ctrl/Alt branches if inserted in the wrong position, silently changing Ctrl or Alt behavior. | Insert the Shift arm as an additional `else if` strictly between the existing Ctrl and Alt checks and the final plain-click fallback (order: ctrl, alt, shift, plain); the new shift_click_adds_link_group_without_clearing test plus the existing Ctrl/Alt selection tests must all stay green in the same run. |
| The pre-existing drag-start auto-select block (today's :2500-2507, `if !mods.ctrl`) does not currently special-case Shift either; leaving it untouched while fixing only the click branch could produce a moment of inconsistent selection state between mouse-down and click-resolution for a Shift+drag gesture. | Out of scope for this workstream (Body/Shift's drag path is unaffected — it already resolves to Move via arm() regardless of the click-time selection edit); flagged here for timeline-trim-gestures (wave 2) to verify when it next touches this hunk, not silently absorbed. |
| RECONCILE RISK: timeline-trim-gestures (wave 2, depends_on snap-engine) independently re-implements this same Body/Shift click fix via its own click_adds_or_toggles helper, citing the shift arm as 'verified absent today' — a citation that is stale by the time it branches, since it depends_on and therefore builds atop snap-engine's already-merged wave-1 fix. | Not this workstream's diff to change (mod.rs's click-handling block, once merged here, is snap-engine's landed output — the duplicate lives entirely in timeline-trim-gestures' plan). Recorded here as the authoritative source-of-truth pointer: snap-engine wave 1 is sole owner/author of the mods.shift arm in mod.rs; timeline-trim-gestures must inherit it (same treatment it already gives middle-mouse-pan and the empty-timeline hint) rather than re-author it. |

## Suggested implementation order

1. 1. snap.rs: move existing fns verbatim, add SnapKind/tiers/candidates/snap_thr behind back-compat wrappers; port nearest_within_threshold/snap_targets_exclude_moving_clips tests unchanged
2. 2. arm.rs: Zone/TrackFlags/GestureKind + arm() + 26 modifier-table row tests (incl. Body/Shift -> MoveNoOverlap)
3. 3. mod.rs: Drag.snapped + guide paint at the 5 existing snap_target arms + dnd-ghost guide
4. 4. mod.rs: body top/bottom zone split + hairline + click-split (>=2x MIN_TRACK_H only)
5. 5. mod.rs AUDIT FIX: add the `else if mods.shift` arm to the click-handling block (today's :2318-2336), between the existing Ctrl and Alt checks and the plain fallback; add shift_click_adds_link_group_without_clearing test
6. 6. mod.rs: EditPoint/Side + seam detection + click wired through arm(Zone::Seam)
7. 7. mod.rs: Gesture::InOut + ruler handle interacts + clamp + release-if-changed undo
8. 8. cue_lane.rs: CueDrag (fixes undo-at-drag-start bug) + cue body move, both through snap::target
9. 9. mod.rs: middle-mouse pan on lanes_resp
10. 10. mod.rs: empty-state hint
11. 11. tools.rs: RollCursor/SlipCursor glyphs
12. 12. settings.rs: snap_markers field (no per-field serde attribute; relies on Settings' container-level #[serde(default)])
13. 13. tools_timeline.rs: 5 ToolDef rows + app/mod.rs TOOL_TABLES line
14. 14. tests.rs: new tests + extend every_tool_snaps/headless_1000_clips_stays_fast; update the one deliberately-changed cue-undo assertion
15. 15. cargo fmt (touched files only), cargo test, cargo run -- --selftest, scripts/size.ps1 -Note snap-engine, open PR with `size:` line

## Deliberate simplifications (`// ponytail:`)

- Drag.snapped is Option<f64>, not Option<(f64,SnapKind)> — the guide line is a plain accent vline, no per-kind label; kind is only surfaced via timeline.snap_query for scripts/tests. Add a label when a user actually asks for one.
- No thread-local snap recorder (pro design's approach) — Drag already exists on every gesture that needs a guide, so one field on it is simpler than a global.
- Body top/bottom split only activates at row height >= 2x MIN_TRACK_H; default/short rows keep the single select+move zone unchanged, protecting existing pinned hit-test tests without a feature flag.
- Seam click is a single Option<EditPoint>, not a Vec — Shift-click 'add to set' (asymmetric multi-roller trim) is explicitly pro-timeline's wave-3 job (distinct from the Body/Shift add-to-selection fix, which is about clip selection, not edit-point sets).
- RollCursor/SlipCursor glyphs are added now (registry protocol requires the owner to seed them once) but paint nowhere until timeline-trim-gestures (wave 2) implements Roll/Slip.
- arm() covers every modifier_table row and is fully tested, but this workstream only WIRES it into Body/BodyBottom/Seam/RulerInOut/Lane(pan) — Edge and Drop zones stay 'tested but not yet called' until wave 2/3 land, same as any registry stub.
- snap_markers uses no per-field serde attribute — Settings' struct-level #[serde(default)] already back-fills missing bools on old project/settings files, exactly like the sibling `snap` field; a bespoke `fn tru()` would have needed a new pub helper for no behavioural gain.
- AUDIT FIX ponytail note: the Body/Shift click fix is a one-branch insertion into an existing if/else chain, not a rewrite of the selection model — no new SelectionKind or multi-mode selection abstraction was added for it.

## Review trail

- Finding 1 (settings.rs serde default) — APPLIED [prior round]. `#[serde(default = "tru")]` was invalid: serde resolves that path in settings.rs's own scope, which has no `tru` fn. It was also redundant given Settings' container-level `#[serde(default)]`. Removed the attribute; `snap_markers` is a plain field + Default entry, matching the sibling `snap` field.
- Finding 2 (cargo test filter path) — APPLIED [prior round]. `cargo test src/ui/timeline` would match zero tests: cargo's positional filter substring-matches the fully-qualified test path (module::tests::name), never a filesystem path. Replaced with `cargo test --lib timeline::` (or a name substring) in verification[].
- Finding 3 (modifier_table Body/Shift row unimplemented) — APPLIED, prior round. Audit: the frozen modifier_table's Body/Shift row ('click = add link group to selection') has no matching code in the real click-handling block (timeline.rs:2318-2336, which branches only on mods.ctrl/mods.alt with no mods.shift arm), and no workstream's diff — in this plan or any sibling — touched that block to add it, despite Shift being wired for rubber-band selection elsewhere. Verified against source: confirmed no `mods.shift` check exists in the cited block or its neighbors. Since snap-engine already exclusively owns timeline/mod.rs in wave 1 and is the first workstream to touch nearby click-handling (body zone split, seam click), the fix is added here rather than deferred: an explicit `else if mods.shift` arm added between the existing Ctrl/Alt checks and the plain-click fallback, additive to selection without clearing it. Added to files[] (mod.rs entry), scope_in, ui_changes, a new regression test (shift_click_adds_link_group_without_clearing), a new risk + mitigation (branch-ordering), a new implementation_order step (5), an acceptance criterion, and a manual verification step. Did NOT touch the second issue named in the audit's narrative (the drag-start auto-select block at :2500-2507 not special-casing Ctrl+Alt/Ctrl+Shift/Ctrl+edge) since only the Body/Shift row was given as a formal finding with a required fix; that block's Edge/Drop-zone modifiers are explicitly out of this workstream's wiring scope per the existing ponytail_notes (tested-but-uncalled until wave 2/3), so it is called out as an explicit risk/scope_out item for timeline-trim-gestures to inherit rather than silently absorbed or silently ignored.
- No findings were rejected in the prior round; the one given audit fix was verified against the real click-handling code structure described and applied as scoped. Everything else preserved unchanged.
- reconcile: Residual finding (major, workstreams snap-engine+timeline-trim-gestures) — VERIFIED AGAINST SOURCE, no change made to this plan. Re-read src/ui/timeline.rs's real click-handling block (current lines ~2317-2334: `if mods.ctrl {...} else if mods.alt {...} else { expand_links }` on the `if let Some(cid) = click` arm) — confirmed no `mods.shift` check exists anywhere in that block or its neighbors today, matching this plan's original audit citation and the fix already scoped into files[] mod.rs entry, scope_in, ui_changes, implementation_order step 5, the shift_click_adds_link_group_without_clearing test, and the branch-ordering risk. That fix stands unchanged: snap-engine (wave 1, exclusive owner of timeline/mod.rs this wave) is sole author of the mods.shift arm, landed here.
- reconcile: The residual finding's actual defect — timeline-trim-gestures (wave 2, depends_on: snap-engine) re-adding its own mods.shift arm via a new click_adds_or_toggles helper and citing the block as 'verified absent today' — is a staleness bug in timeline-trim-gestures' own citation (it branches after snap-engine merges its wave-1 fix, so 'today' is no longer accurate by the time trim-gestures' diff applies) and its own duplicate diff, not a defect in this plan. No file in snap-engine's plan needs editing to fix a duplication that occurs entirely inside a sibling workstream's plan. Recorded as a new risk (RECONCILE RISK) with mitigation pointing timeline-trim-gestures at the correct fix: drop click_adds_or_toggles and inherit snap-engine's landed mods.shift arm the same way it already correctly disclaims middle-mouse-pan and the empty-timeline hint as 'owned by snap-engine wave 1, inherited not re-authored.' The corresponding edit to timeline-trim-gestures' own plan (removing click_adds_or_toggles and its duplicate mod.rs diff/test) is out of scope for this file and must be applied to that workstream's plan directly.
- reconcile: Everything else in the plan (snap tiering, arm() table for other zones, cue-lane fix, in/out handles, pan, empty-state, MCP tools, waves, files, tests, acceptance criteria) preserved unchanged from the prior revision — this reconcile pass added two changelog/risk entries and touched no other field.
