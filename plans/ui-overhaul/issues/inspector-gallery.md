# inspector-gallery: collapsible inspector, effects-stack polish, Color section, and a preset Gallery

**Workstream:** `inspector-gallery` · **Issue:** [#33](https://github.com/KashTheKing/simple-editor/issues/33) · **Wave:** 2 · **Branch/worktree:** `feat/inspector-gallery` → `../simple-editor-wt/inspector-gallery` · **Depends on:** color-engine, forgiveness, registries-schema-hooks, canvas-handles-monitor · **~1400 new lines · Δ exe ≈ +108 KB**

Read [agents.md](../../../agents.md), [goals.md](../../../goals.md), [notes.md](../../../notes.md) and the master plan [README.md](../README.md) (registry protocol, keymap, modifier table) before starting.

## Summary

Wire wave-1's Track/color/forgiveness primitives into where beginners actually look: collapsible primary-first inspector sections with remembered folds, a real Retime button, a new Color section (wheels/curves/vignette, dispatching to color-engine's already-registered auto/match/LUT tools), effects-stack drag-reorder + bulk edit + stable per-effect fold state, speed-ramp presets in Retime, and the Presets pane reborn as gallery.rs (Looks/LUTs/Captions/SpeedRamps/Transitions/Templates) with alt-render hover preview and one-click apply, reusing color-engine's builtin_looks()/apply_look() rather than redefining them. 14 owned/created files; no shared-registry edits beyond this workstream's pre-seeded marker lines; zero MCP tool-name collisions with color-engine after the audit fix.

## Motivation

Serves goals.md's beginner-progressive-disclosure and pro-depth-one-click-away principles, and skeleton principles 'progressive disclosure' + 'one primitive many verbs' + 'MCP parity is a build failure': gap rows 13/14/65/67/77/80/81/82/84/88 and architecture hooks 8 (hover channel producer) and 14 (thumbnail generaliser) are all beginner_value>=4 with size_risk 'none'/'low'. Revised per audit to eliminate cross-workstream duplicate definitions (builtin_looks/apply_look, three MCP tool names) and a nonexistent field (App.hover_preview) that would not compile.

## In scope

- Inspector collapsible sections + fold persistence (row 13)
- Inspector primary-first ordering call-site reorder + real Retime button + Asset-block link (row 14, 88 inspector half)
- Effects stack drag-reorder, bulk edit, fold-by-stable-key, wiring the pre-existing wave-0 EffectsResponse.hover stub (row 65, 60 producer half)
- Speed-ramp curve presets in Retime + inspector (row 67)
- Color section: Primaries wheels + existing Curves/Levels/HueShift/Vignette, dispatching Auto Colour/Colour Match/add-LUT/eyedropper through color-engine's already-registered tools (row 82, 84 UI halves)
- LUT browser UI over color-engine's EffectKind::Lut, applying via color-engine's clip.add_lut tool (row 81 UI half)
- Looks gallery with GPU thumbnails + intensity, reusing color-engine's builtin_looks()/apply_look(), filtered to non-graph presets (row 80 UI half)
- Caption style gallery cards (row 77)
- Gallery pane (Pane::Presets drawer) replacing the size-diet placeholder
- thumbs.rs generalised to a keyed ThumbSource render cache (hook 14)
- 7 new MCP tools for capabilities not already registered by color-engine

## Out of scope

- Video scopes pane / egui::Window (pro-monitor, wave 3)
- Titles pane / Essential Graphics / template exposed params (text-titles, wave 3)
- Essential Sound audio-role UI, LUFS meter, filter chains (audio-dsp-automation, wave 1)
- EffectKind::Lut/Primaries/Qualifier engine bodies, frame_stats, auto_color/color_match/builtin_looks/apply_look implementations AND their MCP tool rows (clip.add_lut, color.auto, color.match) — all color-engine's, consumed only, never re-registered here
- The App.alt_render/AltRenderState channel itself (canvas-handles-monitor builds it); this workstream only adds the AltRequest::Gallery(kind,name) variant and gallery.hover's producer call
- DragPayload::Look/Lut onto the timeline (ponytail-deferred)
- Multicam angle grid, chroma-key engine changes
- Adding Effect.id to the shared struct Effect (model.rs is not owned by this workstream) — a synthesized stable key is used instead (see risks)

## Files

| Op | Path | What |
|---|---|---|
| create | src/ui/color_ui.rs | Color section: Primaries wheels (Painter circle+drag like curves.rs) + existing Curves/Levels/HueShift/Vignette controls in fixed order, each effect added lazily to clip.effects on first touch. Auto Colour / Colour Match / add-LUT / eyedropper buttons call color-engine's already-registered ToolDef rows (color.auto, color.match, clip.add_lut) via App::run_tool_undoable — this file does NOT declare new ToolDef rows for these three names (duplicate-tool audit fix). Eyedropper armed via a thread-local hand-off polled in App::poll_panels, matching the PENDING_FONT/EDIT_MASK idiom (inspector.rs:137-147). |
| create | src/ui/gallery.rs | GalleryTab{Looks,Luts,Captions,SpeedRamps,Transitions,Templates}, GalleryState, GalleryResponse{apply,hover,place,save}, show(). Looks tab lists Settings.effect_presets filtered to `!is_graph()` only (verified src/settings.rs:94) plus color-engine's builtin_looks() (reused, not redefined). Card grid per tab reusing effects_ui::effect_card sizing (CARD=96x54); Luts/Captions/SpeedRamps render via thumbs.rs, Transitions reuse transitions_ui::paint_preview + add_transitions, Templates a name tile (no GPU thumb) + engine::presets::decode_template. Hovering a card >=150ms sets App.alt_render to an AltRequest::Gallery(kind,name) (canvas-handles-monitor's enum, gains this one variant) instead of a nonexistent hover_preview field. 'Save from selection' ports presets_ui.rs's capture_effects/capture_template flow, tagging saved entries so is_graph() stays false for Look saves. gallery.list/gallery.apply(tab=Looks) is the sole tool surface for Looks — color-engine must not also register looks.list/looks.apply for the same apply_look() call. |
| create | src/model/ops/effects.rs | New ops file (unclaimed by any other workstream): Project::reorder_effect and Project::bulk_set_effect_params, each with a unit test, seeded into OP_TOOLS. Neither adds a field to struct Effect (model.rs is owned by registries-schema-hooks/split-god-files, not this workstream) — reorder/bulk both operate purely on index within clip.effects. |
| create | src/ui/app/tools_gallery.rs | pub const TOOLS: &[ToolDef] for gallery.list, gallery.apply, gallery.hover, clip.reorder_effect, clip.effects_bulk, subtitles.style_preset, inspector.folds — 7 rows. Does NOT include clip.add_lut/color.auto/color.match (those are color-engine's; duplicate-tool audit fix). |
| modify | src/ui/app/thumbs.rs | (Already exists post split-god-files as the pure-moved build_effect_thumbnails/effect_thumb_*/STOCK/box_blur/effect_thumbs field, app.rs:452-552+3437-3475 today.) Add ThumbSource{Effect,Look(usize),Lut(PathBuf),Caption(usize)} and build_gallery_thumbnails generalising the existing per-key render loop; keep the EffectKind path byte-identical (same key, same early-out) so effects_ui's catalogue is unaffected. |
| modify | src/ui/inspector.rs | Add pub(crate) fn section() (CollapsingState::load_with_default_open + show_header/body, folds: &mut BTreeMap<String,bool>) exported for effects_ui.rs/transitions_ui.rs/color_ui.rs to `use`. Wrap clip_section's ui.strong(...) blocks (re-grep exact lines against merged main post split-god-files) including Effects/Transitions/Sequence/Asset/Nodes/Mask/Shape/Path/Bus/Markers/Labels, plus a new Color block calling color_ui::show. Replace the weak 'Retime… Ctrl+R' text with a button pushing PENDING_ACTION=Action::Retime. Collapse the Asset block — verified on current main at lines 903-947 (ui.strong("Asset") at 907 through the closing brace at 947, including the description TextEdit at 924 and the comma-separated tags TextEdit at 938) — to a status line + 'Open in Library' link; before editing, grep merged main to confirm size-diet (whose own owns_files never lists src/ui/inspector.rs) has not already collapsed it — skip or reconcile instead of double-editing. Decide where the description/tags TextEdits move (Library asset-details box) and do not silently drop them. Reorder calls into (post split-god-files) inspector_audio.rs / inspector_text.rs so audio/text clips show their block first — exact call-site names must be re-grepped against the landed split-god-files PR. |
| modify | src/ui/effects_ui.rs | (exclusive wave 2) Add drag-reorder for the stack alongside the existing up/down buttons (swap logic ~line 583), using a LOCAL drag-and-drop id keyed by stack index/effect identity (see new_types_and_fns EffectDragId) — NOT the shared crate::ui::mod::DragPayload enum, which (verified src/ui/mod.rs:173-184) only carries Asset/Path/Sequence/Template/Effect(EffectKind)/Transition(TransitionKind) and has no slot-identifying variant, ambiguous when a clip has two same-kind effects. Change the fold id from ("fx_stack", i) (line ~374) to ("fx_stack", stable_effect_key(fx)) where stable_effect_key falls back to a synthesized (kind, first-seen creation order) key — struct Effect (verified src/model.rs:1091-1109) has no id field today. Propagate a changed stack-index param to same-kind/same-index siblings when multi-selected, using the absolute-diff-against-orig rule from inspector.rs:1470-1530. Wire the real 150ms-hover value into EffectsResponse.hover — this field is registries-schema-hooks' pre-existing wave-0 stub (that workstream's own scope names it explicitly), not new work invented here; confirm the stub's shape on merged main before wiring rather than assuming it is unclaimed. |
| modify | src/ui/transitions_ui.rs | (exclusive wave 2) BLOCKED on wave 0: verified today src/ui/transitions_ui.rs:305 `pub fn show(...) -> bool` returns a plain bool — there is no TransitionsResponse struct anywhere in the repo (unlike EffectsResponse, which is real). This is a real return-type change touching every call site (app.rs's Pane::Transitions arm, tests), not a trivial stub-field add. Do not edit this file until registries-schema-hooks (wave 0) introduces `struct TransitionsResponse` and migrates show()'s signature + call sites; re-grep merged main to confirm the type exists before adding `.hover` to it. If wave 0 does not land it, this workstream files its own small PR to add TransitionsResponse first, coordinating with whichever workstream currently owns transitions_ui.rs for wave 2. |
| modify | src/ui/retime.rs | Add a speed_ramp_row() of 5-6 painted-polyline buttons (Montage/Hero/Bullet/Jump/Flash) next to the existing 25/50/100/200/400% row (~line 108); clicking calls engine::presets::apply_curve(preset, &mut clip.speed_curve, clip.duration, false) with one undo; 'Edit curve' reveals Pane::Curves via the generic reveal(pane) API — new wiring, not reuse of an established Speed-to-Curves precedent (the only existing reveal(Pane::Curves) calls, src/ui/layout.rs:853-854, are test code). |
| modify | src/engine/presets.rs | Add builtin_speed_ramps/builtin_caption_styles only (see engine_changes) — builtin_looks/apply_look are color-engine's, reused unchanged. |
| modify | src/settings.rs | Add under a `// ---- ws:inspector-gallery ----` section: inspector_folds: BTreeMap<String,bool>, caption_presets: Vec<TextStyle>, gallery_tab: String (remembers the last open Gallery tab), with #[serde(default)] and Default impl entries. |
| modify | src/ui/tools.rs | Add Glyph::Wheel and Glyph::Lut: enum variant, ALL slice entry, name() arm, draw_glyph() arm for each (from_name is derived, no edit needed). |
| modify | src/ui/app/mod.rs | Within the pre-seeded `// ---- ws:inspector-gallery ----` marker only: replace the Pane::Presets PANE_DRAWERS placeholder (library::reuse_ui rows, landed by size-diet in wave 0c) with gallery::show + response handling; add `mod tools_gallery;` and the tools_gallery::TOOLS row (7 tools) in TOOL_TABLES. |
| modify | src/ui/app/tools_registry_tests.rs | Append OP_TOOLS rows for reorder_effect/bulk_set_effect_params (append-only, low-conflict like size_log.csv); append a no_duplicate_tool_names_vs_color_engine assertion that tools_gallery::TOOLS shares no name with color-engine's TOOLS. |

## Model changes

- No new Project or Clip fields; consumes EffectKind::Lut/Primaries/Qualifier + Asset.effects (added upstream by color-engine, wave 1) without redeclaring them.
- Does NOT consume an Effect.id field — verified struct Effect (src/model.rs:1091-1109) has kind/enabled/params/mask/shader/start/len and no id, and no skeleton workstream commits to adding one. This workstream uses a synthesized (kind, creation-order) key computed in effects_ui.rs instead; documented as a ponytail ceiling.
- New file src/model/ops/effects.rs: Project::reorder_effect(clip, from, to) -> bool and Project::bulk_set_effect_params(clip_ids, index, params: &HashMap<String,f64>) -> usize, each mutating clip.effects in place with stable ids (playback-cache invariant preserved: no Track.id/clip.id churn).

## Engine changes

- src/engine/presets.rs: builtin_speed_ramps() -> Vec<CurvePreset> (5-6 normalised curves targeting speed_curve, reuses apply_curve/scaled_keys verbatim) and builtin_caption_styles() -> Vec<TextStyle> (~8 named looks). Does NOT add builtin_looks()/apply_look() — those are color-engine's (its own explicit wave-1 dependency lands both in this same file); verified current main's presets.rs has only builtin_motions()/is_adjustment_template(), confirming both plans previously used first-definition language for the same names.
- No changes to src/engine/effects.rs, gpu.rs or shaders.rs — auto_color/color_match/frame_stats/EffectKind::Lut\|Primaries and builtin_looks()/apply_look() are consumed as landed by color-engine (wave 1) via its already-registered clip.add_lut/color.auto/color.match ToolDef rows, called from this workstream's UI through App::run_tool_undoable rather than re-implemented or re-registered here; verify exact signatures against merged main before wiring, do not trust this plan's guessed shapes.

## UI changes

- inspector.rs: section() wrapper on every clip_section block, primary-first call order, real Retime button, Asset block (verified lines 903-947) collapsed to a link — pending a size-diet ownership check
- effects_ui.rs: local (non-DragPayload) drag-and-drop stack reorder, bulk propagation, fold keyed by a synthesized stable key, wiring registries-schema-hooks' pre-existing hover stub
- transitions_ui.rs: hover field — BLOCKED until wave 0 lands TransitionsResponse and migrates show()'s return type
- retime.rs: speed-ramp preset row, wired to reveal(Pane::Curves) as new wiring
- color_ui.rs (new): Primaries/Curves/Levels/HueShift/Vignette stack; Auto/Match/add-LUT/eyedropper dispatch to color-engine's existing tools, no duplicate ToolDef rows
- gallery.rs (new): 6-tab card catalogue replacing the Presets pane's placeholder, Looks tab filtered by !is_graph() and reusing color-engine's builtin_looks()/apply_look(); hover sets App.alt_render (AltRequest::Gallery), not a nonexistent hover_preview field
- No new hotkeys, no new Pane variant, no egui::Modal

## New types and functions

- `pub fn reorder_effect(&mut self, clip: Id, from: usize, to: usize) -> bool` — src/model/ops/effects.rs: Move clip.effects[from] to index to; false and no mutation on an out-of-range index.
- `pub fn bulk_set_effect_params(&mut self, clip_ids: &[Id], index: usize, params: &std::collections::HashMap<String, f64>) -> usize` — src/model/ops/effects.rs: Set named params on every clip's effect at `index` whose kind matches the first matching clip's; returns count changed; skips clips with a different kind at that index.
- `pub fn builtin_speed_ramps() -> Vec<CurvePreset>` — src/engine/presets.rs: 5-6 normalised curves (Montage/Hero/Bullet/Jump/Flash) fed straight into apply_curve against speed_curve.
- `pub fn builtin_caption_styles() -> Vec<TextStyle>` — src/engine/presets.rs: ~8 named caption looks (boxed/outlined/karaoke/pop/...) settable onto project.subtitle_style.
- `pub(crate) fn section(ui: &mut egui::Ui, id: &str, title: &str, default_open: bool, folds: &mut std::collections::BTreeMap<String, bool>, header: impl FnOnce(&mut egui::Ui), body: impl FnOnce(&mut egui::Ui))` — src/ui/inspector.rs: Shared CollapsingState wrapper (load_with_default_open + show_header/body) reused by effects_ui.rs/transitions_ui.rs/color_ui.rs via `use crate::ui::inspector::section`; fold state mirrored into Settings.inspector_folds.
- `fn stable_effect_key(fx: &Effect, slot_birth_order: usize) -> String` — src/ui/effects_ui.rs: Synthesized fold/drag identity for an effect since struct Effect has no id field: (kind name, first-seen creation order within the clip). Ceiling: identical-kind effects reordered by something other than this UI (e.g. MCP reorder_effect) can lose fold association — acceptable, documented ponytail note, upgrade path is a real Effect.id if a later workstream lands one.
- `pub fn show(ui: &mut egui::Ui, clip: &mut Clip, stats: Option<&FrameStats>, palette: &Palette, g: &mut Gesture) -> ColorResponse` — src/ui/color_ui.rs: Primaries wheels + Curves/Levels/HueShift/Vignette in fixed order, added lazily; ColorResponse{eyedrop_for: Option<usize>, auto: bool, match_ref: Option<Id>} — auto/match/add-lut are dispatched via App::run_tool_undoable against color-engine's tools, not new ToolDef rows.
- `pub enum GalleryTab { Looks, Luts, Captions, SpeedRamps, Transitions, Templates } pub fn show(ui: &mut egui::Ui, ctx: &egui::Context, state: &mut GalleryState, project: &Project, settings: &mut Settings, selection: &[Id]) -> GalleryResponse` — src/ui/gallery.rs: One card grid per tab (Looks filtered to !is_graph(), reusing color-engine's builtin_looks()); GalleryResponse{apply: Option<(GalleryTab,String,f32)>, hover: Option<(GalleryTab,String)>, place: Option<String>, save: Option<String>}.
- `pub(crate) enum ThumbSource { Effect(EffectKind), Look(usize), Lut(std::path::PathBuf), Caption(usize) } impl App { pub(crate) fn build_gallery_thumbnails(&mut self, ctx: &egui::Context) }` — src/ui/app/thumbs.rs: Generalises the existing build_effect_thumbnails loop to render Looks/LUTs/Caption cards once per (source, key), same GL-context-optional fallback as today.
- `fn speed_ramp_row(ui: &mut egui::Ui, presets: &[CurvePreset], clip_dur: f64) -> Option<usize>` — src/ui/retime.rs: Painted-polyline preset buttons; returns the clicked preset's index.

## New glyphs

- Wheel
- Lut

## Persisted fields

**Settings:**

- inspector_folds: BTreeMap<String, bool>
- caption_presets: Vec<TextStyle>
- gallery_tab: String

**Project (.sedit):**

- (none)

## MCP tools (required — every capability must be scriptable)

| Tool | Kind | Args | Description | Maps to |
|---|---|---|---|---|
| gallery.list | read | tab:string:false:Looks\|Luts\|Captions\|SpeedRamps\|Transitions\|Templates, omit=all | Cards available in one or every Gallery tab (name, builtin vs user-saved); Looks excludes node-graph presets (is_graph()==true) and reuses color-engine's builtin_looks() — this is the sole tool surface for Looks, color-engine registers no separate looks.list. | color-engine's builtin_looks() + settings lists per tab, filtered by !is_graph() for Looks |
| gallery.apply | mutate | tab:string:true:, name:string:true:, clip_ids:array:false:default selection, intensity:number:false:Looks only 0..1 default 1 | Apply a Gallery card: Look/LUT/Caption/SpeedRamp/Transition/Template, one undo. For Looks this is the sole apply path — color-engine registers no separate looks.apply. | color-engine's engine::presets::apply_look (reused, not redefined) / clip.add_effect(Lut) / project.subtitle_style / apply_curve / transitions_ui::add_transitions / place_template |
| gallery.hover | ui | tab:string:true:, name:string:false:null clears | Set/clear App.alt_render to an AltRequest::Gallery(kind,name) for a hovered Gallery card (no mutation, no undo). Not App.hover_preview — that field is never declared by any workstream. | canvas-handles-monitor's App.alt_render/AltRenderState, new AltRequest::Gallery variant |
| clip.reorder_effect | mutate | clip_id:integer:true:, from:integer:true:, to:integer:true: | Move an effect to a new stack index. | Project::reorder_effect |
| clip.effects_bulk | mutate | clip_ids:array:true:, index:integer:true:, params:object:true:name->value | Set one stack-index effect's params on every listed clip whose effect at that index shares its kind. | Project::bulk_set_effect_params |
| subtitles.style_preset | mutate | name:string:true:builtin or Settings.caption_presets | Set project.subtitle_style from a named caption style. | builtin_caption_styles()/Settings.caption_presets -> project.subtitle_style |
| inspector.folds | ui | section:string:true:, open:boolean:true: | Set one inspector section's remembered open/closed state. | Settings.inspector_folds (no undo, Settings-level) |

**Luau:** editor.tool('gallery.list'/'gallery.apply'/'gallery.hover'/'clip.reorder_effect'/'clip.effects_bulk'/'subtitles.style_preset'/'inspector.folds', {...}) — 7 ToolDef rows the palette and tools/list use. clip.add_lut/color.auto/color.match remain reachable via editor.tool but are registered by color-engine, not here — no new Luau surface beyond editor.tool, no -- @on hooks needed.

## Tests

| Test | File | Asserts |
|---|---|---|
| sections_default_open_primary_per_kind | src/ui/inspector.rs | A text clip opens with its primary section expanded and others collapsed per ClipKind; a video clip's Color section defaults closed. |
| fold_state_persists_in_settings | src/ui/inspector.rs | Toggling one section's CollapsingState round-trips through Settings.inspector_folds across a fresh egui::Context (simulated restart). |
| retime_button_pushes_action | src/ui/inspector.rs | Clicking the (now real) Retime button sets PENDING_ACTION = Some(Action::Retime). |
| asset_block_not_double_collapsed | src/ui/inspector.rs | Before editing, a merged-main grep for the Asset block's current shape confirms size-diet has not already collapsed lines 903-947; test fails loudly if the block shape differs from what this PR expects. |
| stack_reorder_via_local_drag_one_undo | src/ui/effects_ui.rs | Dropping a stack row via the local (non-DragPayload) drag id reorders clip.effects with exactly one undo call; two same-kind effects reorder independently. |
| bulk_param_propagates_same_kind_index_only | src/ui/effects_ui.rs | Editing a param at stack index i with 3 clips selected (2 sharing kind at i, 1 not) writes only to the 2 matching clips, absolute value not relative delta. |
| fold_state_keyed_by_synthesized_key_survives_reorder | src/ui/effects_ui.rs | Reordering two effects via the UI keeps each one's own collapsed/expanded state attached via stable_effect_key, not the slot index. |
| hover_after_150ms_wires_wave0_stub | src/ui/effects_ui.rs | A card hovered <150ms leaves EffectsResponse.hover = None; >=150ms sets Some(kind) on the pre-existing wave-0 stub field; no repaint requested while nothing is hovered. |
| speed_ramp_scales_to_clip_duration | src/ui/retime.rs | Applying the 'Hero' preset stretches its normalised keys into clip.speed_curve scaled to the clip's actual duration via apply_curve. |
| color_section_adds_effects_lazily_in_fixed_order | src/ui/color_ui.rs | Touching only Vignette does not create Primaries/Curves/Levels/HueShift; touching Primaries first inserts it ahead of a later-added Curves. |
| auto_color_and_match_dispatch_to_color_engine_tools | src/ui/color_ui.rs | Clicking Auto Colour calls App::run_tool_undoable("color.auto", ...) exactly once with one undo; no new ToolDef named color.auto exists in tools_gallery::TOOLS. |
| no_duplicate_tool_names_vs_color_engine | src/ui/app/tools_registry_tests.rs | tools_gallery::TOOLS shares zero names with color-engine's registered TOOLS (specifically: clip.add_lut, color.auto, color.match are absent). |
| gallery_lists_every_builtin_excludes_graphs | src/ui/gallery.rs | Looks/SpeedRamps/Captions tabs list color-engine's builtin_looks() plus this workstream's builtin_speed_ramps()/builtin_caption_styles() plus every Settings-saved entry, EXCEPT Settings.effect_presets entries where is_graph()==true. |
| gallery_apply_is_one_undo_per_tab | src/ui/gallery.rs | Clicking a card in each of the 6 tabs pushes exactly one undo and edits only the intended clip/project field. |
| gallery_hover_sets_alt_request_not_hover_preview | src/ui/gallery.rs | Hovering a card sets App.alt_render to AltRequest::Gallery(tab,name); no App.hover_preview field exists anywhere in the diff. |
| reorder_effect_rejects_out_of_range | src/model/ops/effects.rs | reorder_effect with from/to >= len returns false and leaves clip.effects unchanged. |
| bulk_set_effect_params_skips_kind_mismatch | src/model/ops/effects.rs | bulk_set_effect_params only writes to clip ids whose effect at `index` matches the first matching clip's EffectKind. |
| thumbnails_build_once_per_key | src/ui/app/thumbs.rs | build_gallery_thumbnails is a no-op on a second call with the same (source, key) — mirrors the existing effect_thumbs_key early-out. |
| assert_no_idle_repaint_gallery_and_color | src/ui/gallery.rs | 30 headless frames with no input over an open Gallery pane / Color section request zero repaints. |
| mcp_gallery_and_reorder_round_trip | src/ui/app/tools_gallery.rs | gallery.apply / clip.reorder_effect / clip.effects_bulk via App::run_tool_undoable match the UI path's project mutation and roll back on garbage-typed args. |

## Verification checklist

- [ ] cargo test -p simple-editor (all 20 new + existing inspector/effects_ui/transitions_ui/retime suites) green
- [ ] cargo test every_glyph_paints_a_picture, every_edit_op_has_a_tool, mutate_rows_roll_back_on_error, tool_names_unique_and_namespaced, no_duplicate_tool_names_vs_color_engine pass with the 2 new glyphs + 7 new tools
- [ ] assert_no_idle_repaint passes for Gallery pane and Color section
- [ ] screenshots: text-clip inspector (Text-equivalent section first, others folded), Gallery Looks tab (12 cards, zero node-graph presets), Color section, Retime window with 5 ramp buttons
- [ ] scripts/size.ps1 -Note inspector-gallery reports delta within 108 KB or PR body states size:+N KB — reason
- [ ] manual: hover a Look >=150ms sets App.alt_render (canvas-handles-monitor's AltRequest::Gallery producer half only, no visible change until its consumer lands), click applies with one Undo entry; drag a LUT card onto the timeline is NOT expected to work (ponytail-deferred)
- [ ] MCP: gallery.apply then clip JSON contains the expected effect/style change; clip.reorder_effect then timeline.list order matches; confirm color-engine's tools/list still shows exactly one each of clip.add_lut/color.auto/color.match (not duplicated by this PR)
- [ ] before starting: grep merged main for TransitionsResponse (add it first if absent) and for color-engine's builtin_looks/apply_look + clip.add_lut/color.auto/color.match (wire to them, do not redefine)

## Acceptance criteria

- [ ] Every flat inspector block (Effects, clip-Transitions, Sequence, Asset, Nodes, Mask, Shape, Path, Bus, Markers, Labels, new Color) is wrapped in section() with independently remembered fold state in Settings.inspector_folds; only the primary section for the clip's ClipKind defaults open
- [ ] inspector.rs's 'Retime… Ctrl+R' weak text is a real button pushing Action::Retime; the Asset block (verified lines 903-947 on current main) collapses to one status line + 'Open in Library' link, only after confirming size-diet has not already done so
- [ ] Effects stack: local (non-DragPayload) drag reorder works alongside the existing up/down buttons (one undo either way); multi-select param edits at one stack index propagate to same-kind/same-index siblings via the absolute-diff-against-orig rule (inspector.rs:1470-1530 pattern); fold state is keyed by a synthesized stable_effect_key (no Effect.id assumed) and survives UI-driven reordering
- [ ] Retime window + inspector Speed row offer Montage/Hero/Bullet/Jump/Flash buttons scaling into clip.speed_curve via apply_curve; 'Edit curve' reveals Pane::Curves on Speed via new wiring (no prior Speed-tied precedent exists)
- [ ] New Color section (color_ui.rs) shows Primaries wheels + existing Curves/Levels/HueShift/Vignette in fixed order, effects added lazily on first touch; Auto Colour / Colour Match buttons invoke color-engine's already-registered color.auto/color.match ToolDef rows via App::run_tool_undoable, not new tool rows of the same name
- [ ] Pane::Presets draws gallery.rs (Looks/LUTs/Captions/SpeedRamps/Transitions/Templates); Looks tab excludes any Settings.effect_presets entry with is_graph()==true and reuses color-engine's builtin_looks()/apply_look(); every card click applies with exactly one undo; hovering a card >=150ms sets an AltRequest::Gallery(kind,name) on App.alt_render (canvas-handles-monitor's mechanism), not a nonexistent hover_preview field
- [ ] transitions_ui.rs's hover field is added only after TransitionsResponse exists (verified missing on current main, show() returns plain bool) — this workstream authors it first if wave 0 has not
- [ ] gallery.list/gallery.apply is the sole tool surface for Looks (color-engine registers no separate looks.list/looks.apply once this lands); clip.add_lut/color.auto/color.match are NOT re-registered here — color_ui.rs and gallery.rs call color-engine's existing tools/fns directly, so tool_names_unique_and_namespaced has zero collisions with color-engine
- [ ] Every genuinely new capability (reorder_effect, bulk_set_effect_params, gallery list/apply/hover, subtitles.style_preset, inspector.folds) has exactly one ToolDef in tools_gallery.rs registered in TOOL_TABLES, callable via editor.tool in Luau, with mutate rows rolling back on bad args
- [ ] cargo test green including new structural + assert_no_idle_repaint tests for Gallery and the Color section, plus asset_block_not_double_collapsed, gallery_lists_every_builtin_excludes_graphs, and no_duplicate_tool_names_vs_color_engine regression tests
- [ ] scripts/size.ps1 delta within the 108 KB budget or PR body carries 'size: +N KB — reason'
- [ ] No egui::Modal and no new rfd dialog introduced anywhere in this diff

## Risks

| Risk | Mitigation |
|---|---|
| struct Effect (verified src/model.rs:1091-1109) has no id field, and no skeleton workstream commits to adding one. The original plan treated Effect.id as a given. | Do not depend on Effect.id. Use a synthesized (kind, creation-order) key (stable_effect_key) for fold-state and drag identity instead; document the collision ceiling as a ponytail note. If a later workstream adds a real Effect.id, migrate the key fn in one line. |
| transitions_ui.rs has no TransitionsResponse type today (verified: `pub fn show(...) -> bool` at src/ui/transitions_ui.rs:305) — unlike EffectsResponse, which is real. | Confirm on merged main whether wave 0 introduced TransitionsResponse before writing to transitions_ui.rs; if not landed, this workstream authors that struct + migrates show()'s one call site (app.rs Pane::Transitions arm) as a small preceding commit, coordinating with the wave-2 owner of that file. |
| Effects-stack drag-reorder has no existing slot-identifying drag payload: DragPayload (verified src/ui/mod.rs:173-184) only carries Asset/Path/Sequence/Template/Effect(EffectKind)/Transition(TransitionKind) — ambiguous when a clip has two same-kind effects. | Use a local (non-shared) egui drag id scoped to effects_ui.rs's own stack UI, keyed by stable_effect_key, instead of extending the cross-module DragPayload enum. |
| The skeleton's own god_file_split table assigns the Asset-block collapse to wave 0c (size-diet), but size-diet's owns_files list never includes src/ui/inspector.rs. | Before touching the Asset block, grep merged main to check whether size-diet already collapsed it; if so, skip this sub-item entirely. |
| Unfiltered Looks-tab listing would show saved node-graph presets as 'Looks' cards, since Settings::effect_presets is shared between colour-look effect chains and node-graph presets, distinguished only by EffectPreset::is_graph() (verified src/settings.rs:94). | Filter by `!is_graph()` when populating the Looks tab; enforced by the gallery_lists_every_builtin_excludes_graphs test. |
| AUDIT (major, fixed): the previous draft of this plan re-declared src/engine/presets.rs's builtin_looks()/apply_look() — functions color-engine, this workstream's own explicit dependency, already lands in that same file one wave earlier. Verified current main has neither yet, so both plans' 'add' wording was first-definition language for the same names, which would fail to compile as two definitions. | presets.rs entry now adds only builtin_speed_ramps()/builtin_caption_styles(); gallery.rs calls color-engine's builtin_looks()/apply_look() directly. Grep merged main for both names before writing gallery.rs's Looks tab to confirm color-engine landed as expected. |
| AUDIT (blocker, fixed): the previous draft registered clip.add_lut, color.auto, and color.match as new ToolDef rows in tools_gallery.rs with argument shapes that diverged from color-engine's own rows of the identical names (clip_id vs clip_ids[]) — registries-schema-hooks' tool_names_unique_and_namespaced test would fail the build on all three. | Removed all three from this workstream's mcp_tools/tools_gallery.rs. color_ui.rs calls color-engine's already-registered rows via App::run_tool_undoable instead. Added a no_duplicate_tool_names_vs_color_engine structural-test line to tools_registry_tests.rs. |
| AUDIT (blocker, fixed): gallery.hover's maps_to previously cited 'App.hover_preview', a field no workstream's files/project_fields ever declares — would not compile. The real hover-render pipeline (App.alt_render/AltRenderState) is built by canvas-handles-monitor under a different name/arg shape. | gallery.hover now sets App.alt_render to a new AltRequest::Gallery(kind,name) variant (added in canvas-handles-monitor's enum, referenced here by name only); canvas-handles-monitor added as an explicit dependency. |
| AUDIT (major, fixed): gallery.list/gallery.apply(tab=Looks) previously risked shipping as a near-duplicate of a hypothetical color-engine looks.list/looks.apply pair, and effects_ui.rs's plan text incorrectly claimed EffectsResponse.hover was new work rather than registries-schema-hooks' pre-existing wave-0 stub. | Documented gallery.list/gallery.apply as the sole tool surface for Looks in this plan's mcp_tools descriptions and acceptance_criteria (a reviewer flag if color-engine's landed PR still carries looks.*); corrected effects_ui.rs's files[] wording to say this workstream wires registries-schema-hooks' existing stub rather than adding a new field. |
| Building Looks(~12)+LUTs(user files)+Captions(~8) GPU/rasterizer thumbnails in one pass can stall a frame if a user's lut_dirs folder is large. | Cap the LUT scan at 24 files/dir and throttle new-thumbnail builds to 2/frame while the Gallery tab is visible (ponytail budget). |
| Replacing the size-diet placeholder in app/mod.rs's PANE_DRAWERS (library::reuse_ui rows for Pane::Presets) must land as a one-line swap inside this workstream's pre-seeded marker, not a broader edit. | grep the `// ---- ws:inspector-gallery ----` PANE_DRAWERS line on merged main and confirm it is exactly one array entry before editing. |

## Suggested implementation order

1. 0. Grep merged main for: struct Effect (id field?), TransitionsResponse (exists?), size-diet's handling of the inspector Asset block, and App.alt_render/AltRequest (canvas-handles-monitor landed?). Resolve all four before writing code.
2. 1. Grep merged main for color-engine's EffectKind::Lut/Primaries/Qualifier, auto_color/color_match/frame_stats signatures, builtin_looks()/apply_look(), and its clip.add_lut/color.auto/color.match ToolDef arg shapes, before writing any call site.
3. 2. Re-grep inspector.rs's post split-god-files clip_section and inspector_audio.rs/inspector_text.rs call sites to replace this plan's pre-refactor line numbers (Asset block is verified 903-947 pre-split; re-check post-split).
4. 3. If TransitionsResponse is missing, author it (struct + show() signature migration + the one Pane::Transitions call site) as a small preceding commit.
5. 4. Settings: add inspector_folds/caption_presets/gallery_tab + Default.
6. 5. tools.rs: add Glyph::Wheel/Lut (4-site edit); run every_glyph_paints_a_picture.
7. 6. src/model/ops/effects.rs: reorder_effect + bulk_set_effect_params + unit tests.
8. 7. engine/presets.rs: builtin_speed_ramps/caption_styles + unit tests (skip builtin_looks/apply_look — color-engine's).
9. 8. inspector.rs: add section(), wrap existing blocks, reorder audio/text call order, collapse Asset block (after the size-diet check), real Retime button.
10. 9. effects_ui.rs: stable_effect_key fn, local drag-id reorder, bulk propagation, fold-by-key, wire the wave-0 hover stub + tests.
11. 10. transitions_ui.rs: hover field on the now-confirmed TransitionsResponse + test.
12. 11. retime.rs: speed-ramp row + test.
13. 12. color_ui.rs: wheels/curves/vignette fixed order + dispatch auto/match/add-lut/eyedropper to color-engine's tools + tests; wire into inspector.rs's new Color section.
14. 13. thumbs.rs: ThumbSource + build_gallery_thumbnails, keep EffectKind path byte-identical + regression test.
15. 14. gallery.rs: GalleryState/Response/show, 6 tabs (Looks filtered !is_graph(), reusing color-engine's builtin_looks()), alt_render hover producer, save-from-selection + tests.
16. 15. Wire gallery.rs into app/mod.rs PANE_DRAWERS ws section; wire tools_gallery.rs (7 rows) into TOOL_TABLES + OP_TOOLS rows.
17. 16. tools_gallery.rs: MCP round-trip tests + no_duplicate_tool_names_vs_color_engine.
18. 17. Full cargo test, screenshots (Gallery Looks tab, Color section, text-clip inspector), scripts/size.ps1, se-review.

## Deliberate simplifications (`// ponytail:`)

- Template gallery cards show a name tile, no GPU thumbnail — compositing a placed template needs a synthetic mini-Project render pass. Upgrade path: Player::layers_once over decode_template's clips once that pipeline is convenient.
- Transitions tab reuses transitions_ui::paint_preview + add_transitions instead of inventing a 'transition preset' model type — no new Settings/Project field.
- Drag-from-Gallery-to-timeline (DragPayload::Look/Lut) is deferred; click-to-apply covers the outcome without touching timeline.rs's drop match, which concurrent wave-2/3 workstreams own. Add the DragPayload variant + one drop arm once that file is quiet.
- Eyedropper uses a thread-local hand-off (PENDING_FONT/EDIT_MASK idiom) instead of growing color_ui::show's signature.
- LUT directory scan capped at 24 files per Settings.lut_dirs entry; thumbnail build throttled to 2 new textures/frame while the Gallery tab is visible — global synchronous scan, no watcher. Upgrade path: background scan thread if a user's LUT folder is large.
- stable_effect_key uses (kind, creation-order) instead of a real Effect.id, since no upstream workstream adds one; this can misattribute fold state if effects are reordered by something other than this UI's own drag/buttons (e.g. concurrent MCP reorder_effect calls). Upgrade path: switch the key fn to a real Effect.id in one line if a later PR adds the field to struct Effect.
- gallery.hover reuses canvas-handles-monitor's AltRequest/App.alt_render channel instead of a dedicated hover field — one fewer piece of App state, one more cross-workstream coordination point (this file adds only the Gallery(kind,name) enum variant, not the channel itself).

## Review trail

- Effect.id: removed as an assumed dependency (verified struct Effect at src/model.rs:1091-1109 has no id field, and no skeleton workstream commits to adding one). Replaced fold/drag identity with a synthesized stable_effect_key(kind, creation-order) fn owned entirely within this workstream's effects_ui.rs, avoiding a cross-workstream schema dependency. Documented the collision ceiling as a ponytail note and added an upgrade path.
- transitions_ui.rs hover field: corrected the false premise that TransitionsResponse is 'a pre-existing wave-0 stub field' — verified transitions_ui::show() returns a plain bool today (src/ui/transitions_ui.rs:305), no such struct exists anywhere in the repo. Reclassified as a blocking dependency: this workstream now authors TransitionsResponse itself (small preceding commit) if wave 0 hasn't, before touching the hover field.
- Asset-block collapse: fixed the cited line range to the verified 903-947 (includes the description TextEdit at 924 and tags TextEdit at 938). Added an explicit risk and a new asset_block_not_double_collapsed test/acceptance-criterion since size-diet's own owns_files never lists inspector.rs despite the skeleton assigning it that collapse.
- gallery_lists_every_builtin: added an explicit !is_graph() filter requirement for the Looks tab (verified Settings::effect_presets is shared between colour looks and node-graph presets via EffectPreset::is_graph() at src/settings.rs:94).
- retime.rs 'Edit curve' -> Pane::Curves: reworded to 'new wiring using the generic reveal(pane) API' since the only current reveal(Pane::Curves) calls (src/ui/layout.rs:853-854) are test code with no Speed/retime tie-in.
- File-count cosmetic fix: 14 owned/created files (4 creates + 10 modifies).
- AUDIT FIX (major, duplicate function names): src/engine/presets.rs entry reworded — this workstream no longer defines builtin_looks()/apply_look() (color-engine, its own explicit wave-1 dependency, already lands both in that same file; today's presets.rs verified to have only builtin_motions()/is_adjustment_template()). inspector-gallery now adds ONLY builtin_speed_ramps() and builtin_caption_styles(), and reuses color-engine's builtin_looks()/apply_look() unchanged from gallery.rs's Looks tab. Updated files[], engine_changes, new_types_and_fns (removed the builtin_looks/apply_look duplicate entries) accordingly.
- AUDIT FIX (blocker, duplicate MCP tools): removed clip.add_lut, color.auto, and color.match from this workstream's mcp_tools — all three are color-engine's tools (color-engine registers them one wave earlier over the same underlying fns/params this plan guessed at). color_ui.rs's LUT/Auto/Match buttons now call App::run_tool_undoable("clip.add_lut"/"color.auto"/"color.match", ...) against color-engine's already-registered rows instead of re-declaring ToolDef rows with diverging arg shapes (clip_id vs clip_ids[]). tools_gallery.rs now registers 7 tools, not 10. Added a no_duplicate_tool_names_vs_color_engine test and acceptance criterion; registries-schema-hooks' tool_names_unique_and_namespaced would otherwise fail the build on these three names.
- AUDIT FIX (blocker, nonexistent field): gallery.hover's maps_to corrected from a self-invented 'App.hover_preview' (never declared by any workstream's files/project_fields, would not compile) to canvas-handles-monitor's real App.alt_render/AltRenderState mechanism, via a new AltRequest::Gallery(kind, name) variant added there. Updated ui_changes, mcp_tools, ponytail_notes, and acceptance_criteria; verification's manual hover check now targets alt_render. Added canvas-handles-monitor to depends_on since gallery.rs's producer half now needs its AltRequest enum to exist.
- AUDIT FIX (major, near-duplicate tools / mis-stated stub ownership): gallery.list/gallery.apply(tab=Looks) is documented as the sole canonical tool surface for Looks — color-engine must not separately register looks.list/looks.apply for the same apply_look() call (cross-referenced in this plan's mcp_tools description and acceptance_criteria so a reviewer catches it if color-engine's PR still has them). Separately, corrected effects_ui.rs's files[] wording: EffectsResponse.hover is registries-schema-hooks' pre-existing wave-0 stub (that workstream's own scope explicitly names it), not new work invented by this workstream — inspector-gallery wires the real 150ms-hover value into that stub rather than adding the field.
