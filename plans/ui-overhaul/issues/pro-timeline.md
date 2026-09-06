# feat(pro-timeline): asymmetric trim, track rename/colour/reorder, view presets, overview strip, sequence tabs, boring/dupe bars, realtime render tint, Find

**Workstream:** `pro-timeline` · **Issue:** [#37](https://github.com/KashTheKing/simple-editor/issues/37) · **Wave:** 3 · **Branch/worktree:** `feat/pro-timeline` → `../simple-editor-wt/pro-timeline` · **Depends on:** trim-model, timeline-trim-gestures, audio-dsp-automation, export-deliver, source-monitor · **~1250 new lines · Δ exe ≈ +90 KB**

Read [agents.md](../../../agents.md), [goals.md](../../../goals.md), [notes.md](../../../notes.md) and the master plan [README.md](../README.md) (registry protocol, keymap, modifier table) before starting.

## Summary

Wave-3 pro-depth pass over the timeline: multi-roller asymmetric trim, header rename/colour/drag-reorder (via trim-model's existing ops, not new ones), per-project view presets, an inline overview minimap, a 2-level sequence tab strip with Un-nest, boring/dupe paint passes, a realtime-safety tint on the render bar (paint only), a volume-automation paint pass, and Ctrl+F Find. Builds only on primitives/actions already landed by trim-model, timeline-trim-gestures, audio-dsp-automation and source-monitor -- no new Pane, no new Track field, no duplicate MCP tools.

## Motivation

goals.md 'faster, tinier, easier, and deep for pros' + gap rows 27,45,46,47,48,50,62,72 + critique items 6 (dupe bars) and 13 (Find). Delivers the pro-timeline surface reserved for wave 3 without touching any file another workstream owns, and without duplicating trim-model's track-mutation ops or export-deliver's render queuing.

## In scope

- Asymmetric multi-track trim (multi-roller id set)
- Track header rename/colour/reorder UI (dispatches to trim-model's existing Project ops)
- Timeline view presets
- Inline overview minimap strip
- Sequence tab strip + Un-nest clip-menu entry
- Boring detector pacing bands
- Dupe-detection colour bars
- Render bar realtime-safety tint (paint only)
- Track/bus volume automation lane paint (paint only, not the gesture)
- Find (Ctrl+F) window over clips/markers/cues/sequences
- Match Frame / Reveal in Library clip-menu entries (dispatch only)

## Out of scope

- Multicam UI/engine (pro-monitor)
- Trim view / wipe compare / grade bypass / stills / scopes (pro-monitor)
- Volume automation drag gesture and Track/Bus.volume field itself (audio-dsp-automation)
- Track.color field, Project::rename_track / set_track_color / move_track (reorder) -- these are trim-model's ops per the audited plan cross-check; this workstream ONLY adds UI (header button/menu/drag) that calls them via App::run_tool_undoable("track.set"/"track.move")
- Un-nest, Match Frame, Reveal in Library model/Action definitions (trim-model, source-monitor -- this workstream only adds menu entries)
- New Pane variants (none added; overview and sequence tabs are inline)
- Render Selection action / request_prerender ranged pre-render and its MCP tool -- owned by export-deliver

## Files

| Op | Path | What |
|---|---|---|
| create | src/ui/find_ui.rs | Ctrl+F Find window (egui::Window, non-modal): substring search over clip names, marker names/notes, subtitle cues, sequence names. pub fn find(p:&Project,q:&str)->Vec<Hit>; pub fn window(ctx,state:&mut FindState,p:&Project)->Option<Jump>. |
| create | src/ui/app/tools_timeline_pro.rs | pub const TOOLS: &[ToolDef] for timeline.find/view_preset/timeline.dupes/timeline.pacing/timeline.overview ONLY + ACT_HANDLERS fn act() for Find/ToggleOverview/RenameTrack/SetTrackColor/ReorderTrack, where the last three call App::run_tool_undoable("track.rename"\|"track.set"\|"track.move", ...) against trim-model's existing tools rather than defining new ToolDef rows (no track.reorder/track.rename/track.set_color tool is added here -- see audit changelog). |
| modify | src/ui/timeline/mod.rs | (exclusive wave3 owner) add TimelineView struct + Settings-backed preset combo hook, sequence-tab strip draw call above ruler, overview strip draw call gated on Settings.overview, TimelineState.track_rename/track_drag fields, wire new Act variants (RenameTrack, SetTrackColor, ReorderTrack) into the match at end of show(), each forwarding to trim-model's model ops. |
| modify | src/ui/timeline/header.rs | (exclusive wave3 owner, created wave2 by timeline-trim-gestures) add colour swatch button reading/writing the EXISTING wave-0 Track.color: Option<[u8;3]> field + Rename/Colour context-menu rows next to existing Mute/Solo/Add/Remove Track rows (pattern verified at current monolith timeline.rs:1178-1219); drag-to-reorder via Sense::drag() on a small grip rect calling trim-model's Project::move_track, no new DragPayload variant. |
| modify | src/ui/timeline/menus.rs | (exclusive wave3 owner) clip_menu(): add is_sequence param -> 'Un-nest' row (Action::Unnest, from trim-model); add 'Match Frame' / 'Reveal in Library' rows (Action::MatchFrame/RevealInLibrary, from source-monitor) to every clip's menu; update the 4 call sites (verified at current monolith timeline.rs:1502,1563,4169,4239) to pass clip.kind==ClipKind::Sequence. |
| modify | src/ui/timeline/paint.rs | (exclusive wave3 owner) add paint_dupes(pp,p,rows,pal) grouping clips by (asset,src_in..src_out); paint_pacing(pp,p,ruler,thr,pal) ruler tint bands; paint_realtime_bar(pp,p,ruler,state,pal) merging Clip::has_effects() spans under the existing prerender bar (verified current monolith timeline.rs:1838-1844) with a danger tint where effects present & not yet in prerender's ready set -- paint only, does not queue a render; paint_automation(pp,p,ruler_row,pal) volume Animated curve line on header row (reads Track.volume/Bus.volume added by audio-dsp-automation), display-only for now (see ponytail_notes). |
| modify | src/ui/timeline/gestures.rs | (exclusive wave3 owner) extend the multi-roller Trim gesture: generalise ids to Vec<(Id,bool)> so Shift-click on additional seams adds rollers before drag (asymmetric multi-track trim, row 27 UI half); calls existing ripple_trim/roll_edit per pair (trim-model, timeline-trim-gestures own the primitives -- this file only assembles the id set). |
| modify | src/ui/timeline/tests.rs | (exclusive wave3 owner, append only) new tests listed below. |
| modify | src/ui/app/timeline_pane.rs | (exclusive wave3 owner) pass TimelineView preset + Settings.overview + realtime-bar data into TimelineCtx/show(). Does NOT add a request_prerender_range helper -- owned by export-deliver. |
| modify | src/hotkeys.rs | append under `// ---- ws:pro-timeline ----` at END of actions! table: Find (Ctrl+F), ToggleOverview (unbound), RenameTrack (unbound). RenderSelection is NOT added here -- reserved for export-deliver. |
| modify | src/settings.rs | append END fields: timeline_views: Vec<TimelineView> (#[serde(default)]), boring_thr: (f32,f32) (#[serde(default = "default_boring_thr")] = (1.5,20.0)), overview: bool (#[serde(default)]); add matching literals to impl Default for Settings (verified struct at settings.rs:145, Default impl at settings.rs:260). |
| modify | src/ui/tools.rs | Glyph enum: add Swatch (track-colour icon) and Rows (view-preset/overview icon) variants -- named Swatch, not Palette, to avoid colliding with the pervasive `use crate::theme::Palette;` (tools.rs:18); add to Glyph::ALL (verified list at tools.rs:229), name()/from_name() (tools.rs:314,404), draw_glyph() (tools.rs:792) match arms (guarded by every_glyph_paints_a_picture, tools.rs:1703). |
| modify | src/ui/app/mod.rs | append `mod tools_timeline_pro;` and one line each into WINDOW_DRAWERS (find_ui::window), ACT_HANDLERS (tools_timeline_pro::act), TOOL_TABLES (tools_timeline_pro::TOOLS) under the pre-seeded `// ---- ws:pro-timeline ----` marker lines from wave 0b. |

## Model changes

- No Track field is added by this workstream. Track.color: Option<[u8;3]> is consumed as landed by wave-0's registries-schema-hooks (per audited cross-check of that plan's own files[] entry for src/model/track.rs) -- this plan's earlier draft incorrectly claimed the field was unclaimed and added a second, differently-typed `color: u8`; that has been removed (see changelog).
- No new Project mutator for rename/recolour/reorder is added here. trim-model (wave 1) already builds Project::rename_track / set_track_color / move_track in src/model/ops/tracks.rs 'for single ownership' anticipating this exact pro-timeline need (per audited cross-check) -- this workstream's header UI calls those via the existing track.set / track.move MCP tools, not new ones.
- Clip::has_effects (model.rs:2578, verified) and Track/Bus.volume:Animated (audio-dsp-automation) already exist by the time this worktree branches.

## Engine changes

- No new engine/model fn. video_dirty_spans' existing wave-1 id-order guard (playback.rs, extended by player-rate-loop) is the mechanism a track reorder relies on for a full cache clear -- this workstream only adds a consumer-side pin test for it, it does not add the reorder fn itself (trim-model owns Project::move_track).

## UI changes

- Track header: colour swatch button (Swatch glyph) writing the existing Option<[u8;3]> field via trim-model's set_track_color, Rename/Colour context-menu rows, drag grip for reorder via move_track
- Timeline toolbar: TimelineView preset combo (uses new Rows glyph)
- Inline overview minimap strip above the ruler (Settings.overview toggle)
- Sequence tab strip above the ruler (Main / <sequence name>)
- Ruler: pacing (boring) tint bands, realtime-safety red tint segment under the existing prerender bar (paint only, no new queuing action)
- Clip lanes: dupe-detection colour bars (gated on `detailed`)
- Track header row: thin automation line for Track/Bus volume
- Clip context menu: Un-nest (Sequence clips only), Match Frame, Reveal in Library
- Ctrl+F Find window (non-blocking egui::Window) with a results list and Enter-to-jump

## New types and functions

- `pub struct TimelineView { pub name: String, pub waves: bool, pub thumbs: bool, pub keys: bool, pub clip_text: bool, pub row_h: f32 }` — src/settings.rs: Named view preset; Settings.timeline_views: Vec<TimelineView>, #[serde(default)] seeded with 2 builtins (Detailed, Compact) in Default::default().
- `pub fn find(p: &Project, q: &str) -> Vec<Hit>  // struct Hit{kind:HitKind,id:Id,t:f64,text:String}` — src/ui/find_ui.rs: Case-insensitive substring search across clip names, marker names/notes, subtitle cues, sequence names.
- `pub fn window(ctx: &egui::Context, state: &mut FindState, p: &Project) -> Option<Jump>  // struct Jump{t:f64,pane:Option<Pane>,select:Option<Id>}` — src/ui/find_ui.rs: Non-blocking egui::Window; Enter/click a hit returns a Jump the caller applies (seek+select+surface).
- `pub fn paint_dupes(pp: &egui::Painter, p: &Project, rows: &RowGeom, state: &TimelineState, pal: &Palette)` — src/ui/timeline/paint.rs: Colour-bar clips sharing (asset, src_in..src_out) with a shared hue per group; gated on `detailed`.
- `pub fn paint_pacing(pp: &egui::Painter, p: &Project, ruler: egui::Rect, state: &TimelineState, thr: (f64,f64), pal: &Palette)` — src/ui/timeline/paint.rs: Tint ruler bands under clips with duration > thr.1 or < thr.0.
- `pub fn paint_realtime_bar(pp: &egui::Painter, p: &Project, ruler: egui::Rect, state: &TimelineState, prerender: &[(f64,f64,bool)], pal: &Palette)` — src/ui/timeline/paint.rs: Merge Clip::has_effects() spans on video tracks; red tint where effects present and not covered by a `ready` prerender segment, skip effect-free seconds entirely. Read-only paint; does not request or queue any render.
- `pub fn paint_automation(pp: &egui::Painter, track: &Track, buses: &[Bus], row: egui::Rect, state: &TimelineState, pal: &Palette)` — src/ui/timeline/paint.rs: Draw the Track/Bus volume Animated curve as a line across the header row. Display-only: no drag gesture is wired here (see ponytail_notes).

## Actions and hotkeys

| Action id | Label | Chord | Note |
|---|---|---|---|
| find | Find… | Ctrl+F | free today (Ctrl+Shift+F is ExportFrame, verified hotkeys.rs:126); opens find_ui window |
| toggle_overview | Toggle Overview Strip |  | unbound; menu/palette only, per skeleton keymap |
| rename_track | Rename Track |  | unbound; header double-click/context-menu only; dispatches to trim-model's Project::rename_track via track.rename tool, not a new op |

## New glyphs

- Swatch -- filled colour swatch, used on the track header colour button (named Swatch, not Palette, to avoid colliding with crate::theme::Palette, used as `pal: &Palette` throughout the same paint.rs/header.rs files)
- Rows -- three stacked bars of differing width, used for the view-preset toolbar combo and the overview-toggle button

## Persisted fields

**Settings:**

- timeline_views: Vec<TimelineView>  #[serde(default)] -- seeded with 2 builtins in Default::default()
- boring_thr: (f32,f32)  #[serde(default = "default_boring_thr")] = (1.5, 20.0)  // (short_s, long_s)
- overview: bool  #[serde(default)] = false

**Project (.sedit):**

- (none)

## MCP tools (required — every capability must be scriptable)

| Tool | Kind | Args | Description | Maps to |
|---|---|---|---|---|
| timeline.find | read | query:string:true:substring, case-insensitive | Search clip names, marker names/notes, subtitle cues, sequence names. | find_ui::find(project,&query) |
| timeline.view_preset | mutate | name:string:false:set (omit=get current); waves:boolean:false; thumbs:boolean:false; keys:boolean:false; clip_text:boolean:false | Get or set the active timeline view preset (row heights/element toggles). | Settings.timeline_views + TimelineState active index |
| timeline.dupes | read |  | Groups of clip ids sharing (asset, src_in..src_out) -- duplicate-source detection. | paint::paint_dupes' grouping fn, exposed read-only |
| timeline.pacing | read | long_s:number:false; short_s:number:false | Clip ids/spans outside the boring-detector thresholds (too long / too short). | paint::paint_pacing's classification fn |
| timeline.overview | ui | enabled:boolean:false:omit=toggle | Show/hide the inline overview minimap strip. | Settings.overview via ui.action(ToggleOverview) |

**Luau:** editor.tool('timeline.find', {query}), 'timeline.view_preset' (get/set by name), 'timeline.dupes'/'timeline.pacing' (read, spans for scripted QC passes), 'timeline.overview' (ui, toggle). Track rename/recolour/reorder go through trim-model's existing 'track.set' / 'track.move' tools -- not duplicated here. Render Selection stays export-deliver's tool. No new @on hook -- these are request/response tool calls, not event sources.

## Tests

| Test | File | Asserts |
|---|---|---|
| move_track_forces_full_clear | src/playback.rs tests (append, consumer-side pin) | video_dirty_spans(old,new) returns None when two video Track ids swap position via trim-model's Project::move_track -- pins the existing wave-1 guard as the reorder feature's contract; this workstream does not re-test move_track's own swap logic, that lives with trim-model |
| header_rename_commits_on_enter | src/ui/timeline/tests.rs | Act::RenameTrack applied via the harness calls trim-model's rename_track and pushes exactly one undo |
| header_color_swatch_cycles_label_colors | src/ui/timeline/tests.rs | clicking the swatch N times cycles the EXISTING Track.color: Option<[u8;3]> field through None + 8 LABEL_COLORS via trim-model's set_track_color, matching Asset/Clip label convention |
| view_preset_toggles_paint_calls | src/ui/timeline/tests.rs | switching TimelineView.waves/thumbs/keys off suppresses the corresponding paint call (spy counters) without changing clip geometry |
| overview_strip_paints_only_when_enabled | src/ui/timeline/tests.rs | Settings.overview=false paints nothing extra; true paints a fit-to-window strip and clicking it seeks+recentres the main state |
| sequence_tab_strip_shows_main_and_open | src/ui/timeline/tests.rs | tab strip has 1 entry when Project.editing is None, 2 (Main + name) when Some; clicking Main triggers the existing OpenParentSequence path |
| clip_menu_shows_unnest_only_for_sequence_clips | src/ui/timeline/tests.rs | clip_menu(is_sequence=true) includes an Un-nest row dispatching Action::Unnest; false omits it |
| clip_menu_always_offers_match_frame_and_reveal | src/ui/timeline/tests.rs | every clip_menu() call includes Match Frame and Reveal in Library rows dispatching the existing Actions |
| dupes_group_by_asset_and_src_range | src/ui/timeline/tests.rs | paint_dupes' grouping fn returns groups only for clips sharing (asset, src_in, src_out); singletons excluded |
| pacing_bands_match_thresholds | src/ui/timeline/tests.rs | clips shorter than thr.0 or longer than thr.1 are flagged; clips inside the range are not |
| realtime_bar_skips_effect_free_seconds | src/ui/timeline/tests.rs | paint_realtime_bar produces no span where has_effects()==false across all active clips at that second; produces a span where any clip has_effects()==true |
| find_matches_clips_markers_cues_sequences | src/ui/find_ui.rs #[cfg(test)] | find() returns hits of every HitKind for a project seeded with one of each, case-insensitive |
| find_jump_seeks_selects_and_surfaces | src/ui/timeline/tests.rs or app tests | choosing a Hit seeks the playhead to its time, selects its id (clip/marker), and surfaces the owning pane |
| asymmetric_multi_roller_trim_moves_only_shift_clicked_seams | src/ui/timeline/tests.rs | Shift-click adds a second seam to the roller set; dragging trims both cuts by the same delta while an untouched third seam on the same track stays put |
| every_glyph_paints_a_picture | src/ui/tools.rs (existing test, extended automatically) | Swatch and Rows glyphs draw without panicking -- no new test file, existing test iterates Glyph::ALL |
| no_duplicate_defaults | src/hotkeys.rs (existing test, extended automatically) | Ctrl+F / new unbound actions introduce no chord collision |
| every_edit_op_has_a_tool | src/ui/app/tools_registry_tests.rs (existing structural test) | no duplicate tool names introduced: track.rename/track.set/track.move remain trim-model's sole entries, pro-timeline adds no track.reorder/track.rename/track.set_color row |
| ui_action_covers_every_action | src/ui/app/tools_registry_tests.rs (existing structural test) | Find, ToggleOverview, RenameTrack all resolve through ui.action and appear in the palette row builder |
| headless_1000_clips_stays_fast | src/ui/timeline/tests.rs (existing, extended) | adding paint_dupes/paint_pacing/paint_realtime_bar/paint_automation to the show() call keeps the 1000-clip frame under 10 ms |

## Verification checklist

- [ ] cargo test (all new tests above green, existing 657+ unchanged/still passing)
- [ ] cargo run -- --selftest (idle step stays green with overview/automation panes idle)
- [ ] scripts/size.ps1 -Note pro-timeline shows delta <= 90 KB or PR body carries a `size:` line
- [ ] screenshot: header colour swatch + rename in progress, view-preset combo, overview strip, sequence tab strip, realtime red tint on an effect-laden clip, dupe bars on two clips sharing source
- [ ] manual: drag a track header past its same-kind neighbour -> order swaps via trim-model's move_track, next preview frame reflects new z-order/mix order
- [ ] manual: Ctrl+F, type a marker name, Enter -> playhead jumps and marker is selected
- [ ] manual: right-click a Sequence clip -> Un-nest restores its content and removes the Sequence clip
- [ ] review: diff against trim-model's landed src/model/ops/tracks.rs to confirm no duplicate rename_track/set_track_color/move_track/track.rename/track.set_color/track.reorder tool was (re)introduced

## Acceptance criteria

- [ ] cargo test green including new tests above; count grows by >=19, no existing test renamed/removed
- [ ] Ctrl+F opens Find window; typing jumps selection+playhead+surfaces the hit's pane; Esc closes
- [ ] Dragging a track header swap-reorders same-kind tracks via trim-model's Project::move_track; playback cache does a full clear on release (existing wave-1 id-order guard fires, not reproduced here)
- [ ] Header context menu Rename renames a track inline via trim-model's Project::rename_track; header colour swatch picks 1 of 8 LABEL_COLORS or None via trim-model's Project::set_track_color and the wave-0 Track.color: Option<[u8;3]> field -- this workstream adds no field and no new mutating op
- [ ] Timeline toolbar combo switches TimelineView presets (waves/thumbs/keys/clip text) live, no repaint when idle
- [ ] Settings.overview=true paints a fit-to-window minimap strip above the ruler that scrubs on click/drag
- [ ] Sequence tab strip shows Main + open sequence name when Project.editing is Some; clicking Main calls close_sequence via existing OpenParentSequence path
- [ ] Clip context menu on a Sequence clip shows Un-nest; every clip menu shows Match Frame / Reveal in Library dispatching existing Actions
- [ ] Render bar shows a third (red) tint under seconds where any active video clip has_effects()==true and is not yet prerendered -- paint only; this workstream does not add or queue a Render Selection action
- [ ] Dupe-detection color bars group clips sharing (asset, src range) when detail view is on; boring bands tint ruler for clips outside boring_thr
- [ ] Track/bus volume automation lane paints Animated volume curve on header row (display only; drag gesture out of scope)
- [ ] size delta measured <= 90 KB via scripts/size.ps1
- [ ] every_edit_op_has_a_tool and ui_action_covers_every_action stay green with no duplicate track.reorder/track.rename/track.set_color tool introduced

## Risks

| Risk | Mitigation |
|---|---|
| clip_menu() signature change (new is_sequence param) touches 4 call sites in the same file this workstream exclusively owns in wave 3 -- low risk, but a stale rebase from timeline-trim-gestures (wave 2, same file lineage) could reintroduce the old signature. | Land this edit first in the branch (implementation step 5) and run cargo test immediately after; the compiler catches every stale call site. |
| Header drag-to-reorder grip rect could overlap the existing mute/solo toggle buttons or the resize handle at the row bottom (HANDLE_H). | Place the grip in the currently-unused header row area to the left of the name text (hr.left()..hr.left()+14), test with a screenshot; existing bw/sb/mb rects already reserve the right side. |
| paint_realtime_bar and paint_dupes/paint_pacing add per-frame O(clips) work; on a 1000-clip project this must stay under the existing 10 ms headless budget. | Gate all three on `detailed` (same flag used for waveform/filmstrip painting) and reuse headless_1000_clips_stays_fast as the regression gate -- add the new passes to that test's setup. |
| This workstream's header UI (rename/colour/reorder) depends on trim-model's Project::rename_track/set_track_color/move_track existing with exactly those signatures by the time pro-timeline branches; trim-model was only a transitive dependency before this fix (via timeline-trim-gestures), not a direct one. | Added trim-model to depends_on directly so the worktree is created only after trim-model's PR lands; if signatures differ, header.rs/mod.rs are the only files needing a one-line call-site fix. |

## Suggested implementation order

1. 1. settings.rs: TimelineView struct + Settings fields + Default literals + tools.rs Glyph additions (Swatch, Rows -- compiles standalone, unblocks everything else)
2. 2. timeline/header.rs: rename (context-menu + inline TextEdit), colour swatch (reads/writes the EXISTING Track.color: Option<[u8;3]>), drag-to-reorder grip -> Act::RenameTrack/SetTrackColor/ReorderTrack, each dispatching App::run_tool_undoable against trim-model's track.rename/track.set/track.move tools (no new tool defined)
3. 3. timeline/mod.rs: wire the 3 new Act arms, TimelineView-driven paint branching, sequence-tab strip, overview strip
4. 4. timeline/paint.rs: paint_dupes, paint_pacing, paint_realtime_bar, paint_automation (each independently testable pure fns)
5. 5. timeline/menus.rs: clip_menu is_sequence param + Un-nest/Match Frame/Reveal in Library rows; update 4 call sites
6. 6. timeline/gestures.rs: asymmetric multi-roller trim id-set generalisation
7. 7. find_ui.rs: find() + window()
8. 8. app/timeline_pane.rs: wire settings-driven view/overview/realtime data into TimelineCtx
9. 9. app/tools_timeline_pro.rs + hotkeys.rs + app/mod.rs registry lines: Find/ToggleOverview/RenameTrack actions + MCP ToolDef table (timeline.find/view_preset/dupes/pacing/overview only -- no track.* row)
10. 10. timeline/tests.rs: append all new tests; run full cargo test; scripts/size.ps1 -Note pro-timeline

## Deliberate simplifications (`// ponytail:`)

- Track.color is NOT added by this workstream -- it is wave-0's Option<[u8;3]> field, consumed as-is. The earlier draft of this plan wrongly asserted the field was unclaimed and added a conflicting `color: u8` LABEL_COLORS-index field; corrected per audit (see changelog). Upgrade path if an index-based scheme is ever preferred: rename the shared field in the same PR that changes it, not independently in a downstream workstream.
- Track reorder is adjacent-swap only via drag (no arbitrary multi-position reorder UI) -- covers the common 'move this track up one' case; upgrade path: full drag-to-any-position if requested. The swap itself is trim-model's Project::move_track, not reimplemented here.
- Realtime-bar classification skips effect-free seconds entirely rather than computing a true 3-state everywhere (red/green/neutral) -- matches gap-matrix row 62's own note; it does not narrow prerender's own request range to effect-only spans (left for export-deliver/prerender.rs), and it does not queue any render itself -- Render Selection stays export-deliver's action.
- Sequence tabs support exactly 2 levels (Main + one open sequence) because Project.editing is a single Option<Id>, not a stack -- matches the existing model; deeper nested editing is out of scope (open-parent already exists via Action::OpenParentSequence).
- Boring/dupe/pacing paint passes are pure fns taking &Project, no caching -- fine at the 1000-clip budget already proven by headless_1000_clips_stays_fast for similar per-frame passes; add a cache keyed on project version if that test regresses.
- Volume automation is paint-only: the existing Gesture::Volume hit-zone (timeline.rs:2554) is scoped to the clip body rect, not the header row, so no drag gesture is wired here -- upgrade path: a header-row hit zone if this becomes a request.
- No new Pane variant for Overview (inline strip) or for anything else in this workstream, per the skeleton's frozen 'exactly one new pane (Source)' decision.

## Review trail

- [audit fix applied, blocker x2] Removed pro-timeline's own `Track.color: u8` field addition (src/model/track.rs) that conflicted with wave-0 registries-schema-hooks' `Track.color: Option<[u8;3]>` on the same struct. Verified against current source (model.rs:2766-2782): Track has no color field today, so both additions were genuinely new and would have been a duplicate-field compile error plus an RGB-triplet vs label-index type mismatch. This plan now consumes the wave-0 field as-is; header colour swatch and the header_color_swatch_cycles_label_colors test were reworded accordingly. Removed from: files[] (track.rs entry deleted), model_changes, project_fields (now empty), ponytail_notes, risks.
- [audit fix applied, blocker] Removed pro-timeline's own `Project::reorder_track` fn and its unit test (reorder_track_swaps_same_kind_only); the audited cross-check of trim-model's plan shows it already builds Project::rename_track/set_track_color/move_track in src/model/ops/tracks.rs specifically for this later need. pro-timeline's header UI (rename/colour/drag-reorder) now dispatches to those existing ops instead of re-implementing them. `move_track_forces_full_clear` replaces `track_reorder_forces_full_clear` as the consumer-side pin test (same guard, corrected fn name).
- [audit fix applied, major] Removed the `track.reorder`, `track.rename`, and `track.set_color` MCP tool rows this plan was going to add -- they duplicated trim-model's `track.set` (name+color) and `track.move` (reorder) tools under different names/argument shapes. pro-timeline's Actions (RenameTrack/SetTrackColor/ReorderTrack) now call the existing tools via App::run_tool_undoable rather than defining new ones; mcp_tools list, luau field, tools_timeline_pro.rs description, and implementation_order step 9 updated accordingly.
- [dependency correction, applied] Added `trim-model` directly to depends_on (previously only reachable transitively via timeline-trim-gestures) since this workstream's header UI now calls trim-model's ops by name -- makes the required merge order explicit for worktree scheduling; added a matching risk entry.
- [not applied -- other audit items are out of scope for this plan] The major finding's other duplicates (looks.apply vs gallery.apply, timeline.replace vs timeline.replace_clip, EffectsResponse.hover double-description) belong to color-engine/inspector-gallery/trim-model/source-monitor's plans, not pro-timeline's; nothing in this plan's text referenced those names, so no change was made here.
- [size/line estimate] Reduced size_delta_kb 100->90 and est_new_lines 1300->1250, and softened the acceptance-criteria test-count floor from >=21 to >=19, reflecting the removed reorder_track fn, its test, and the three removed MCP tool rows -- a small non-load-bearing adjustment following from the above fixes, not itself a separately re-verified finding.
