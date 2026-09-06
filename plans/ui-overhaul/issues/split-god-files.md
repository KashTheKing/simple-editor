# refactor(wave0a): split app.rs/model.rs/timeline.rs/inspector.rs/settings_ui.rs into child modules (pure move)

**Workstream:** `split-god-files` · **Issue:** [#16](https://github.com/KashTheKing/simple-editor/issues/16) · **Wave:** 0a · **Branch/worktree:** `refactor/split-god-files` → `../simple-editor-wt/split-god-files` · **Depends on:** — · **~160 new lines · Δ exe ≈ +10 KB**

Read [agents.md](../../../agents.md), [goals.md](../../../goals.md), [notes.md](../../../notes.md) and the master plan [README.md](../README.md) (registry protocol, keymap, modifier table) before starting.

## Summary

Zero-behaviour-change file split so waves 1-3 get disjoint ownership: src/ui/app.rs (6816 ln) -> src/ui/app/{mod,files,actions,panes,preview_pane,timeline_pane,library_pane,gpu,thumbs,jobs,menus,drops,lib_preview,mcp_exec,tools_playback,tools_clip,tools_subtitles,tools_media,tools_timeline,tools_helpers,windows,tests}.rs; src/model.rs (6092 ln) -> src/model/{mod,text,asset,effect,graph,shape,path,marker,audio,transition,subtitle,clip,track,project}.rs + src/model/ops/*.rs + io.rs + tests.rs; src/ui/timeline.rs (4348 ln) -> src/ui/timeline/{mod,paint,menus,header,cue_lane,gestures,tests}.rs; src/ui/inspector.rs (2407 ln) audio/text blocks -> inspector_audio.rs/inspector_text.rs; src/ui/settings_ui.rs (980 ln) -> settings_ui/{mod,general,appearance,performance,capture,hotkeys}.rs. All 657 tests keep their names and pass unchanged; no new pub API, no registries (wave 0b), no deletions (wave 0c).

## Motivation

Enables 'concurrency is structural': waves 1-3 (23 workstreams) need disjoint file ownership on app.rs/model.rs/timeline.rs, which today are single files every workstream would collide on. Explicit wave-0a prerequisite; must land before registries-schema-hooks (0b) and size-diet (0c).

## In scope

- Pure code motion of the 5 named files into child-module trees, one impl-block-per-file split
- Splitting timeline::show()'s internal blocks into free fns in paint.rs/menus.rs/header.rs/cue_lane.rs/gestures.rs, called in sequence from mod.rs::show()
- cargo fmt on moved files only (not full-crate reformat)
- ARCHITECTURE.md module-map update to reflect new paths
- Deleting src/ui/app.rs, src/model.rs, src/ui/timeline.rs, src/ui/settings_ui.rs after their contents move

## Out of scope

- TOOL_TABLES/ACT_HANDLERS/FRAME_HOOKS/WINDOW_DRAWERS/PANE_DRAWERS registries and ToolDef type (wave 0b)
- Any schema/serde field additions (Track.locked/ripple etc.) — wave 0b
- Any dependency or dead-code deletion (egui_commonmark, Tool::Zoom, presets_ui.rs) — wave 0c
- Any behaviour change, new Action, new hotkey, new Pane, new MCP tool

## Files

| Op | Path | What |
|---|---|---|
| delete | src/ui/app.rs | Content redistributed into src/ui/app/* below |
| create | src/ui/app/mod.rs | Imports(1-35); ExportKind enum(46), Toast(53-67), McpJob(70-76), LibPreview(78-87), App fields(89-264); free helpers kept here as pub(crate) because they're called from multiple sibling files below: job_window(265), now_secs(310), relocate_assets(379), guarded(397), preview_canvas/clamp_canvas/frame_render_size(421-456), timeline_is_empty(825), Compress+converted_path(830-864); App::new(922-1091); toast/push_undo/after_edit/set_project/seek/import_files(1092-1258); impl eframe::App::update(6044-6392); `mod` decls for every child file |
| create | src/ui/app/files.rs | open_path/open_media/open_project(1192-1231), poll_probes call sites(1259-1701 minus poll_probes body -> jobs.rs), finish_export(1655), url_window/download_dir/start_download(3250-3346), act_import_timeline(3649), url/convert/compress windows(4294-4379); uses app/mod.rs's pub(crate) Compress/converted_path |
| create | src/ui/app/actions.rs | fn act(&mut self, a: Action)(1702-2253), toggle_pane(2254-2262), add_shape/add_text/add_stroke(2949-3108), poll_panels(3109-3228), enter_sequence(3229-3249) |
| create | src/ui/app/panes.rs | draw_pane(2263-2273), draw_pane_inner(2274-2934) minus Preview/Timeline/Library arms (-> preview_pane.rs/timeline_pane.rs/library_pane.rs); refresh_presets(2935-2948); calls app/mod.rs's pub(crate) guarded/preview_canvas/clamp_canvas |
| create | src/ui/app/preview_pane.rs | The Pane::Preview arm body extracted from draw_pane_inner as fn draw(app, ui) |
| create | src/ui/app/timeline_pane.rs | The Pane::Timeline arm body extracted from draw_pane_inner as fn draw(app, ui) |
| create | src/ui/app/library_pane.rs | The Pane::Library arm body extracted from draw_pane_inner as fn draw(app, ui) |
| create | src/ui/app/gpu.rs | sync_gpu(3407-3436); export_frames(3476-3487), serve_gpu_exports(3487-3512), gpu_off(3512-3527), gpu_preview_texture(3527-3557), gpu_frame(3557-3583), render_frame_now(3583-3598), request_prerender(3598-3611), export_frame(3611-3631), source_frame(3632-3648) — all 8 fns previously left unassigned by the 'gpu.rs = 3407-3648 minus thumbnails' clause are now listed explicitly; export_frame calls write_image from thumbs.rs (pub(crate)) |
| create | src/ui/app/thumbs.rs | STOCK/STOCK_W/STOCK_H consts(474-476), effect_thumb_key(457-479), effect_thumb_source(480-501), box_blur(505-552) — corrected from the prior 'effect_thumb_source(480-604)' citation, which wrongly absorbed box_blur/write_image/base64; build_effect_thumbnails(3437-3475) — corrected from the prior '3437-3631' citation, which wrongly absorbed export_frames onward. write_image(553-588) and base64(589-604) move here too as pub(crate) since gpu.rs::export_frame and tools_*.rs both call them |
| create | src/ui/app/jobs.rs | poll_probes(1259-1701 body), import_recording(3721-3747), audio_inputs(3748-3756), blur_capture_options(3757-3773), screenshot_tick(4622-4659), sync_proxies(4660-4709) |
| create | src/ui/app/menus.rs | glyph_for(3774-3780), menu_item(3781-3823), pane_glyph(3824-3828), ensure_bg_texture(3829-3857), view_menu(3858-4020), menu_bar(4021-4232); calls thumbs.rs's pub(crate) box_blur |
| create | src/ui/app/drops.rs | handle_drops(4233-4293) |
| create | src/ui/app/lib_preview.rs | start_lib_preview(4380-4411), lib_preview_frame(4412-4437), draw_lib_preview(4438-4621); also step_time and scrub_time (moved here from app/mod.rs's free-helper pile since these two are lib_preview-only callers) |
| create | src/ui/app/mcp_exec.rs | run_script(4710-4754), sync_mcp(4755-4783), poll_mcp(4784-4809), handle_tool(4810-4838), start_tool_job(4839-4893) |
| create | src/ui/app/tools_helpers.rs | New file (not in the original plan): shared free fns called from 2+ of the tools_*.rs split below — parse_ease(625), anim_of(644), mask_shape(662), mask_slot(670), apply_mask_fields(681), add_mask(403), node_kind(720), color_arg(748), apply_clip_fields(759), arg_str/arg_f64/arg_u64/arg_bool/arg_ids(605-620) — all pub(crate), used by tools_clip.rs, tools_timeline.rs and inspector.rs's clip_menu |
| create | src/ui/app/tools_playback.rs | run_tool arm bodies for the real prefixes playback.*, frame.*, render.*, sequence.*, templates.* (corrected: the original 'tools_project.rs' grouping cited settings./history. prefixes that do not exist in the match — verified by extracting every literal "x.y" string in app.rs:4894-5747, which lists audio/clip/container/export/frame/labels/markers/media/notes/plan/playback/project/render/sequence/shapes/shot/style/subtitles/templates/timeline). reconcile: re-verified against wave-1 player-rate-loop's plan, which lists this same path a second time as op:'create' with a description admitting it 'relocates whatever wave-0a parked' here — that is a duplicate-create bug on player-rate-loop's side only; this plan's op:'create' (the true origin of the file) is unchanged |
| create | src/ui/app/tools_clip.rs | run_tool arm bodies for clip.*, audio.*, shapes.*, style.*, container.* (corrected: real prefixes, not the invented effect./transition.*) |
| create | src/ui/app/tools_subtitles.rs | run_tool arm bodies for subtitles.*, labels.*, markers.*, notes.*, plan.* (corrected: real prefixes, not the invented marker.* singular) |
| create | src/ui/app/tools_media.rs | run_tool arm bodies for media.*, export.*, shot.* (corrected: real prefix is media., not the invented library./asset./import.*) |
| create | src/ui/app/tools_timeline.rs | run_tool arm bodies for timeline.*, project.* (corrected: real prefixes, not the invented track./settings./history.*) |
| create | src/ui/app/windows.rs | name_window(5748-5778), windows(5779-6042) |
| create | src/ui/app/tests.rs | #[cfg(test)] mod tests body verbatim (6394-6816), header changed to `use super::*;` |
| delete | src/model.rs | Content redistributed into src/model/* below |
| create | src/model/mod.rs | TrackKind/ClipKind/Scaler/BackgroundMode/BlendMode/Ease/Keyframe/AnimLink/Animated(18-397); Sequence/Stash/Note/MoodItem/PlanItem/Project type defs(2849-3015); AttrSet(6014-6092); `mod` decls + `pub use` re-exports + `pub mod ops;` |
| create | src/model/text.rs | TextStyle, TextSpan(398-583) |
| create | src/model/asset.rs | AudioStreamInfo, Asset(584-656) |
| create | src/model/effect.rs | EffectKind, ParamSpec, MaskShape, Mask, Effect(654-1140) |
| create | src/model/graph.rs | MathOp/CmpOp/LogicOp/NodeKind/Node/Edge/NodeGraph(1141-1700) |
| create | src/model/shape.rs | ShapeKind, Stroke, ShapeStyle, PathAsset(1701-1921) |
| create | src/model/path.rs | live-links/expressions types(1922-1987) |
| create | src/model/marker.rs | Marker, Label(1988-2043) |
| create | src/model/audio.rs | FilterKind, AudioFilter, Bus(2044-2229) |
| create | src/model/transition.rs | TransitionKind, TransitionEdge, Transition(2230-2332) |
| create | src/model/subtitle.rs | Cue(2333-2353) |
| create | src/model/clip.rs | Clip(2354-2760, all fields already pub) |
| create | src/model/track.rs | Track(2761-2848, all fields already pub) |
| create | src/model/project.rs | `pub struct Project { .. }` field block(2956-3015); impl moves to ops/*.rs + io.rs |
| create | src/model/ops/assets.rs | impl Project 'assets' section (3084-3154) |
| create | src/model/ops/queries.rs | impl Project 'queries'(3155-3277) + 'usage'(4028-4056) |
| create | src/model/ops/tracks.rs | impl Project 'tracks' section (3278-3328) |
| create | src/model/ops/editing.rs | impl Project 'editing'(3329-3615) + 'motion helpers'(4131-4182) |
| create | src/model/ops/transitions.rs | impl Project 'transitions' section (3616-3719) |
| create | src/model/ops/subtitles.rs | impl Project 'subtitles' section (3720-3779) |
| create | src/model/ops/sequences.rs | impl Project 'sequences (nested timelines)' section (3780-3953) |
| create | src/model/ops/planner.rs | impl Project 'planner'(3966-4014) + 'notes'(4015-4027) |
| create | src/model/ops/autocut.rs | impl Project 'auto-cut' section (4057-4083) |
| create | src/model/ops/templates.rs | impl Project 'templates' section (4084-4130) |
| create | src/model/ops/markers.rs | impl Project both 'labels' sections(3954-3965, 4183-4234) + 'markers' section(4235-4330) |
| create | src/model/ops/buses.rs | impl Project 'buses' section (4331-4388) |
| create | src/model/ops/shapes.rs | impl Project 'shapes / adjustment layers' section (4389-4534) |
| create | src/model/ops/paths.rs | impl Project 'reusable paths' section (4535-4669) |
| create | src/model/ops/graph.rs | impl Project 'node graphs' section (4670-4719) |
| create | src/model/ops/attrs.rs | impl Project 'copy / paste attributes' section (4720-4807), plus AttrSet-consuming fns |
| create | src/model/io.rs | impl Project 'persistence' section (to_json/from_json/save), (4808-4900) |
| create | src/model/tests.rs | #[cfg(test)] mod tests body verbatim (4901-6013), header `use super::*;` |
| delete | src/ui/timeline.rs | Content redistributed into src/ui/timeline/* below |
| create | src/ui/timeline/mod.rs | Cap enum(51), TimelineState+Default(114-178), Act enum(348-360, kept HERE not routed through hotkeys::Action — see corrected header.rs signature), paste_clips(194-207), TimelineCtx/TimelineResponse/Drag/Gesture(266-407), pub fn show() hub(1027-2905) calling into paint/menus/header/cue_lane/gestures; keeps scroll/zoom(1060-1171), rows loop skeleton(1172-1805), rubber-band/gutter/pre-render-bar/ruler-ticks/project-markers(1806-1913), playhead/lanes-bg/scrollbars/deferred-apply(2103-2495), edge-auto-scroll(2845-2904) |
| create | src/ui/timeline/paint.rs | row_order/row_top/drop_on_clip/nearest/snap_target/snap_playhead/snap_time/tick_step/tick_label/toggle_button/draw_waveform/has_curve_keys/has_keys/key_range/mini_prop_color/draw_mini_graph/prop_range/hatch/flag/marker_hit/wave_color/label_color/diamond/db_frac/frac_db/gain_db/draw_filmstrip(408-809) |
| create | src/ui/timeline/menus.rs | paste_menu(179-193), label_menu/transition_kind_menu/transition_ease_menu/shared_effect_kinds/clip_menu(810-1026); clip_menu calls tools_helpers.rs's pub(crate) mask/anim/color_arg fns from wave 1's tooling once those land — for wave 0a it keeps calling the app.rs-local versions unchanged |
| create | src/ui/timeline/header.rs | Track-header cell drawing (~1172-1330: name/lock/mute/solo/height controls) as `pub(super) fn draw_header(ui: &mut egui::Ui, bp: &egui::Painter, state: &mut TimelineState, pal: &Palette, font: &egui::FontId, small: &egui::FontId, track: &Track, idx: usize) -> Option<Act>` — corrected return type from the wrong `Option<crate::hotkeys::Action>` (Mute/Solo/AddTrack/RemoveTrack are variants of the timeline-local `Act` enum at timeline.rs:348-360, not of hotkeys::Action, which has no such variants) and corrected params to include the painter/palette/font locals the block actually closes over |
| create | src/ui/timeline/cue_lane.rs | Subtitle lane block (1914-2102) as `pub(super) fn draw(ui: &mut egui::Ui, c: &mut TimelineCtx<'_>, state: &mut TimelineState, painter: &egui::Painter, playhead_x: f32, sub_h: f32, subs_lane: egui::Rect, pal: &Palette, small: &egui::FontId, thin: egui::Stroke, full: egui::Rect, id: egui::Id, mods: egui::Modifiers) -> Option<Act>` — corrected from the original 4-param signature, which omitted sub_h/subs_lane/pal/small/id/thin/full/mods that the block demonstrably uses (verified by reading timeline.rs:1914-1950); use the plan's own wrap-then-cut technique (nest the nested fn inside show() first, let rustc's capture errors enumerate the full param list) before finalizing since more locals may appear later in the 1914-2102 range |
| create | src/ui/timeline/gestures.rs | 'gestures' block (2496-2844) as fn handle(ui, state, c, response) -> TimelineResponse; apply the wrap-then-cut technique here too since this is the plan's own flagged non-mechanical extraction |
| create | src/ui/timeline/tests.rs | mod tests body verbatim (2907-4348), header `use super::*;` |
| modify | src/ui/inspector.rs | Keep show()/transition_section/project_section/rescale_prompt/luau_highlight/link_menu/asset_use_count/take_*; clip_section(602-1577) keeps its 8-param signature (ui, project, selection, playhead, fonts, palette, settings, undo — verified at inspector.rs:602-610) and its non-audio/non-text branches, calling inspector_audio::section()/inspector_text::section() for the audio/text sub-blocks with the corrected signatures below |
| create | src/ui/inspector_audio.rs | `pub(super) fn section(ui: &mut egui::Ui, project: &mut Project, ids: &[Id], playhead: f64, palette: &Palette, undo: &mut dyn FnMut(&Project)) -> bool` — corrected to add `playhead: f64`, which the audio/video branch uses at inspector.rs:1510 (`s.local(playhead)`; branch starts inspector.rs:1379). gain_to_db(47)/db_to_gain(55) move here too. reconcile: re-verified against source (inspector.rs:602-611, 1500-1512) and against wave-1 audio-dsp-automation's plan, which lists this same path a second time as op:'create' with a different fn name (`show`) and different params — that duplicate-create/signature-drift bug belongs to audio-dsp-automation's plan (fix: change its entry to op:'modify', rename `show` back to `section`, conform params to this exact signature so this workstream's inspector.rs call site keeps compiling). This plan's own entry is unchanged, confirmed correct |
| create | src/ui/inspector_text.rs | `pub(super) fn section(ui: &mut egui::Ui, project: &mut Project, ids: &[Id], fonts: &[String], palette: &Palette, undo: &mut dyn FnMut(&Project)) -> bool` — corrected to add `fonts: &[String]`, which the Text branch (starts inspector.rs:949) iterates at inspector.rs:982-986 to populate a font ComboBox. span_draft_at(65)/set_span(91)/text_preset_fields(108) move here |
| delete | src/ui/settings_ui.rs | Content redistributed into src/ui/settings_ui/* below |
| create | src/ui/settings_ui/mod.rs | SettingsUi struct(23-35), Status(36-40), pub fn show(74-131) dispatching to tab fns by name |
| create | src/ui/settings_ui/general.rs | general(132-272), port_field(273-287) |
| create | src/ui/settings_ui/performance.rs | performance(288-370) |
| create | src/ui/settings_ui/capture.rs | capture_tab(371-437), export(438-467) |
| create | src/ui/settings_ui/hotkeys.rs | hotkeys_tab(468-527), capture_key(528-570, renamed from capture to avoid a clash with capture.rs's capture_tab) |
| create | src/ui/settings_ui/appearance.rs | appearance(571-783), icon_row(784-831), color_row(832-end) |
| modify | src/ui/mod.rs | No path changes needed for app/inspector/settings_ui/timeline mod decls; add `pub mod inspector_audio; pub mod inspector_text;` |
| modify | ARCHITECTURE.md | Update 'Module map' to list the new src/ui/app/*, src/model/*, src/ui/timeline/*, src/ui/settings_ui/*, inspector_audio.rs, inspector_text.rs paths in place of the 5 old single-file entries |

## Model changes

- None: every `pub struct` field stays exactly as declared today
- impl Project is split across 16 files under src/model/ops/ plus io.rs; multiple impl blocks for one type across files is valid Rust
- Any private helper fn called cross-section must be promoted to `pub(crate)` at cut time — grep for non-pub `fn` inside impl Project and check callers against the new file boundaries before finalizing cuts

## Engine changes

- None — src/engine/* is untouched by this workstream

## UI changes

- None visible: file-layout change only. Private fields stay visible to defining module's descendants via `mod x;` nesting inside app/mod.rs / timeline/mod.rs — zero visibility changes needed there
- inspector.rs's audio/text extraction and timeline.rs's cue_lane/gestures extraction change function boundaries only (explicit params replace closures); widget IDs, hover/gesture behaviour and screenshot pixels must stay identical
- header.rs/cue_lane.rs signatures corrected per findings: header.rs returns the timeline-local `Act` enum (not hotkeys::Action, which has no Mute/Solo/AddTrack/RemoveTrack variants); cue_lane.rs takes 13 params, not 4, to cover sub_h/subs_lane/pal/small/id/thin/full/mods alongside the originally-listed ui/state/c/painter/playhead_x

## New types and functions

- `pub(super) fn section(ui: &mut egui::Ui, project: &mut Project, ids: &[Id], playhead: f64, palette: &Palette, undo: &mut dyn FnMut(&Project)) -> bool` — src/ui/inspector_audio.rs: Extracted audio block of clip_section; adds playhead (used at inspector.rs:1510) that the original signature omitted. reconcile: this exact fn name/signature is the canonical shape wave-1 audio-dsp-automation must conform to (its own file entry must switch to op:'modify')
- `pub(super) fn section(ui: &mut egui::Ui, project: &mut Project, ids: &[Id], fonts: &[String], palette: &Palette, undo: &mut dyn FnMut(&Project)) -> bool` — src/ui/inspector_text.rs: Extracted text/typography block of clip_section; adds fonts (used at inspector.rs:982-986) that the original signature omitted
- `pub(super) fn draw_header(ui: &mut egui::Ui, bp: &egui::Painter, state: &mut TimelineState, pal: &Palette, font: &egui::FontId, small: &egui::FontId, track: &Track, idx: usize) -> Option<Act>` — src/ui/timeline/header.rs: Track-header cell extracted from show()'s row loop; returns the timeline-local Act enum (Mute/Solo/AddTrack/RemoveTrack), corrected from a wrongly-cited hotkeys::Action return, with the painter/palette/font params the block actually needs
- `pub(super) fn draw(ui: &mut egui::Ui, c: &mut TimelineCtx<'_>, state: &mut TimelineState, painter: &egui::Painter, playhead_x: f32, sub_h: f32, subs_lane: egui::Rect, pal: &Palette, small: &egui::FontId, thin: egui::Stroke, full: egui::Rect, id: egui::Id, mods: egui::Modifiers) -> Option<Act>` — src/ui/timeline/cue_lane.rs: Subtitle-cue lane extracted verbatim from show(); corrected to include every local the block closes over (sub_h/subs_lane/pal/small/id/thin/full/mods) instead of the originally-listed 5 params
- `pub(super) fn handle(ui: &mut egui::Ui, state: &mut TimelineState, c: &mut TimelineCtx<'_>, resp: &mut TimelineResponse, drag: &mut Option<Drag>)` — src/ui/timeline/gestures.rs: The gesture match extracted verbatim from show(); flagged non-mechanical, use wrap-then-cut
- `pub(crate) fn parse_ease/anim_of/mask_shape/mask_slot/apply_mask_fields/add_mask/node_kind/color_arg/apply_clip_fields/arg_str/arg_f64/arg_u64/arg_bool/arg_ids(...)` — src/ui/app/tools_helpers.rs: New file (finding-driven addition): these app.rs free fns are each called from 2+ of the tools_*.rs split files plus timeline's clip_menu, so they need one shared pub(crate) home instead of being silently left in mod.rs's private 'free helpers' bucket

## MCP tools (required — every capability must be scriptable)

| Tool | Kind | Args | Description | Maps to |
|---|---|---|---|---|

**Luau:** No change. editor.tools()/tools/list output must be byte-for-byte identical before and after (existing 64 tools, same names/args/order) — pinned by server_end_to_end and a manual curl diff against main. The corrected tools_*.rs prefix grouping (playback/frame/render/sequence/templates -> tools_playback.rs; clip/audio/shapes/style/container -> tools_clip.rs; subtitles/labels/markers/notes/plan -> tools_subtitles.rs; media/export/shot -> tools_media.rs; timeline/project -> tools_timeline.rs) changes only which file a match arm lives in, never the tool's name, args or behaviour.

## Tests

| Test | File | Asserts |
|---|---|---|
| (all 657 existing test names, unchanged) | src/ui/app/tests.rs, src/model/tests.rs, src/ui/timeline/tests.rs, src/ui/inspector.rs, src/ui/settings_ui/*.rs | Every test keeps its original name and body; cargo test reports the same count (657) and zero failures/renames |
| module_tree_matches_architecture_md | src/ui/app/tests.rs (new, small) | Smoke test: constructing App and calling a representative fn from each new sub-module compiles and runs — catches an accidentally-unwired `mod` declaration |
| tools_list_json_unchanged | manual verification step, not a #[test] | MCP server tools/list JSON diffed byte-for-byte against a copy captured from main before the split |

## Verification checklist

- [ ] cargo build (debug) compiles with zero new warnings introduced by the split
- [ ] cargo test — exactly 657 tests, same names, all pass (diff `cargo test -- --list` output before/after)
- [ ] cargo run -- --selftest passes, including timing/idle assertions unaffected by this workstream
- [ ] cargo run -- --screenshot <path> before and after; ffmpeg psnr filter reports pixel-identical output
- [ ] Start the MCP server before and after; curl tools/list on both; diff the JSON byte-for-byte
- [ ] cargo fmt --check on only the touched files (never bare `cargo fmt` — it reformats the whole crate per notes.md pitfalls)
- [ ] scripts/size.ps1 shows the release binary within a few KB of baseline
- [ ] git diff --stat confirms no file outside the listed owns_files changed
- [ ] grep every non-pub `fn` inside old impl Project blocks and confirm each caller lands in the same target ops/*.rs file post-split, or was promoted to pub(crate)

## Acceptance criteria

- [ ] src/ui/app.rs, src/model.rs, src/ui/timeline.rs, src/ui/settings_ui.rs no longer exist as single files; content lives in the directory trees listed in files
- [ ] cargo test reports exactly 657 passing tests with the same names as before the split
- [ ] cargo run -- --selftest exits 0
- [ ] Screenshot PSNR check and MCP tools/list diff both show zero difference from main
- [ ] scripts/size.ps1 delta is within +/-15 KB of baseline
- [ ] ARCHITECTURE.md module map reflects every new path
- [ ] No file outside this workstream's owns_files list is touched
- [ ] header.rs returns Option<Act> (not hotkeys::Action); cue_lane.rs's draw() takes all 13 real params; inspector_audio/text section() fns take playhead/fonts respectively; tools_*.rs files are keyed on the verified real prefixes (audio/clip/container/export/frame/labels/markers/media/notes/plan/playback/project/render/sequence/shapes/shot/style/subtitles/templates/timeline), not invented ones
- [ ] inspector_audio.rs keeps fn name `section` with the 6-param signature (ui, project, ids, playhead, palette, undo) so wave-1 audio-dsp-automation's modify of this file keeps inspector.rs's call site compiling
- [ ] tools_playback.rs keeps op:'create' here in wave 0a; wave-1 player-rate-loop must treat it as op:'modify', not a second create

## Risks

| Risk | Mitigation |
|---|---|
| timeline::show()'s cue_lane and gestures blocks are inline code inside one giant function, not separate top-level fns — extracting them requires identifying every closed-over local; the original plan's cue_lane signature (5 params) already missed 8 real locals (sub_h/subs_lane/pal/small/id/thin/full/mods), confirmed by reading timeline.rs:1914-1950 | Extract by first wrapping each block in a nested fn taking explicit args INSIDE show() (compiles = correct capture list — the plan's own prescribed technique), then cut-paste that fn to its new file unchanged. Apply this to header.rs and cue_lane.rs too, not just gestures.rs, since both had wrong signatures in the original plan |
| A private (non-pub) helper fn in one impl-Project section is called from a fn in a different section — after the ops/* split this becomes a private-item-not-visible compile error; the same problem exists in app.rs's ~14 free helpers (parse_ease, anim_of, mask_*, node_kind, color_arg, apply_clip_fields, Compress/converted_path, step_time/scrub_time) which are called across 7+ target files but the original plan gave them no explicit home or pub(crate) note | For model.rs: grep every non-pub `fn` inside impl Project and check call sites fall within the same target file; promote to pub(crate) otherwise. For app.rs: the new tools_helpers.rs (see files) collects the shared ones; Compress/converted_path stay pub(crate) in app/mod.rs (used by files.rs); step_time/scrub_time move to lib_preview.rs (their only caller) |
| Removing Pane::Presets or renaming any Action id while 'cleaning up' during the move would silently drop a user's stored hotkey/layout override | This workstream renames zero Action/Pane identifiers; any such rename is out of scope (wave 0c's job) and must be rejected in review |
| cargo fmt with no path argument reformats the entire crate, hiding the real move (documented pitfall) | Run `cargo fmt -- <list of only the new/changed files>` or run cargo fmt then git diff and revert unrelated formatting hunks before commit |
| inspector_audio.rs/inspector_text.rs extraction changes widget ui.id()/egui::Id::new salts if id derivation implicitly used enclosing-fn call-site identity | Audit every ui.id()/Id::new in the extracted blocks for reliance on enclosing-fn identity; pass an explicit salt param if any exists so persisted collapsing-state ids in Settings don't shift |
| The corrected tools_*.rs prefix grouping (5 files, 19 real prefixes) still needs each prefix's exact line ranges pinned before cutting, since only prefix names were re-derived here, not line numbers | Implementer re-runs `grep -n '"[a-z_]*\.[a-z_]*"' src/ui/app.rs` scoped to 4894-5747 and buckets each match's line range into its corrected target file before the cut, mirroring the audit done for this revision |
| reconcile: downstream waves (audio-dsp-automation wave 1, player-rate-loop wave 1) list inspector_audio.rs / tools_playback.rs a second time as op:'create' with drifted signatures/descriptions, instead of op:'modify' against this wave-0a origin | Not this workstream's fix to make (its own entries are already correct and verified against source) — flag in review that any downstream workstream creating a path this plan already creates must use op:'modify' and conform to the signature documented here, specifically inspector_audio.rs's `section(ui, project, ids, playhead, palette, undo)` and tools_playback.rs's ownership of playback./frame./render./sequence./templates. arms |

## Suggested implementation order

1. 1. git checkout -b refactor/split-god-files in a fresh worktree off up-to-date main
2. 2. Split src/model.rs first (types already all-pub) — model/mod.rs + type files, then model/ops/*.rs + io.rs + tests.rs; cargo check after each 2-3 files
3. 3. Split src/ui/timeline.rs — mod.rs + paint.rs + menus.rs first (mechanical), then header.rs and cue_lane.rs using wrap-then-cut to get their real param lists (do not trust the signatures pre-written in this plan verbatim — confirm against the live capture-error list), then gestures.rs, then tests.rs
4. 4. Split src/ui/app.rs — mod.rs (struct + App::new + update) first, then files/actions/panes/preview_pane/timeline_pane/library_pane/gpu/thumbs/jobs/menus/drops/lib_preview in any order, then tools_helpers.rs (cut first, since tools_*.rs and menus.rs/timeline's clip_menu depend on its pub(crate) fns), then mcp_exec.rs + the 5 tools_*.rs files split by the corrected real prefixes (re-grep line ranges per prefix before cutting), then windows.rs, then tests.rs
5. 5. Extract inspector_audio.rs / inspector_text.rs from src/ui/inspector.rs's clip_section with the corrected 6-param signatures
6. 6. Split src/ui/settings_ui.rs into settings_ui/{mod,general,appearance,performance,capture,hotkeys}.rs
7. 7. Update src/ui/mod.rs and ARCHITECTURE.md module map
8. 8. Run the full verification checklist; fix any compile/test fallout before opening the PR
9. 9. PR body includes size: +N KB per agents.md's gate convention

## Deliberate simplifications (`// ponytail:`)

- No new abstractions beyond the finding-driven tools_helpers.rs, which exists only because the alternative (leaving ~14 cross-file helpers in mod.rs as private fns) doesn't compile — smallest fix for a real constraint, not speculative
- Cross-section private-fn promotions default to pub(crate), never blanket pub — keeps the crate's real API surface (MCP tools, Luau) unchanged
- header.rs/cue_lane.rs take explicit params instead of a shared 'context' struct — more params is cheaper than a new type used by 2-3 call sites
- Corrected tools_*.rs grouping is still a guess at line-range boundaries within each file; the implementer must re-grep before cutting (flagged in risks) rather than trust this plan's prose beyond the prefix-to-file mapping
- reconcile: both residual fixes turned out to be entirely the other workstream's problem (audio-dsp-automation's duplicate/incompatible inspector_audio.rs create; player-rate-loop's duplicate tools_playback.rs create) — no speculative defensive changes added here just because a sibling plan drifted; this plan's own entries were re-verified against live source and left as-is

## Review trail

- Finding 1 (wrong-path, tools_*.rs prefix grouping): confirmed by grepping every "x.y" tool-name literal in app.rs:4894-5747 — real prefixes are audio/clip/container/export/frame/labels/markers/media/notes/plan/playback/project/render/sequence/shapes/shot/style/subtitles/templates/timeline (20, not the finding's claimed 18 — export and shot also appear). Renamed and regrouped all 5 tools_*.rs files onto real prefixes: tools_playback.rs (playback/frame/render/sequence/templates), tools_clip.rs (clip/audio/shapes/style/container), tools_subtitles.rs (subtitles/labels/markers/notes/plan), tools_media.rs (media/export/shot), tools_timeline.rs (timeline/project). Dropped the invented settings./history./track./effect./transition./library./asset./import. groupings entirely.
- Finding 2 (wrong-path, thumbs.rs build_effect_thumbnails range): confirmed via grep -n that build_effect_thumbnails runs 3437-3475 (export_frames starts 3476, not part of it). Corrected the citation from '3437-3631' to '3437-3475' and explicitly listed export_frames/serve_gpu_exports/gpu_off/gpu_preview_texture/gpu_frame/render_frame_now/request_prerender/export_frame/source_frame(3476-3648) under gpu.rs's owns instead of a vague 'minus' clause.
- Finding 3 (wrong-path, thumbs.rs effect_thumb_source range): confirmed via grep -n that effect_thumb_source runs 480-501 (box_blur starts at 505). Corrected the citation from '480-604' to '480-501', kept box_blur/STOCK consts in thumbs.rs per the skeleton's own '457-552' citation, and explicitly assigned write_image(553-588) and base64(589-604) to thumbs.rs as pub(crate) (rather than leaving them homeless) since gpu.rs::export_frame and the tools_*.rs files both call them.
- Finding 4 (ownership, app/mod.rs free-helper cross-file calls): confirmed each named helper's cross-module callers by grep (job_window in windows.rs; guarded/preview_canvas/clamp_canvas in panes.rs and gpu.rs; relocate_assets in files.rs and tools files; mask_*/node_kind/color_arg/apply_clip_fields/parse_ease/anim_of in tools files and timeline's clip_menu; step_time/scrub_time only in lib_preview.rs; Compress/converted_path in files.rs and tools files). Added a new tools_helpers.rs file for the shared tools-only group, kept Compress/converted_path pub(crate) in app/mod.rs, and moved step_time/scrub_time into lib_preview.rs since it's their only caller.
- Finding 5 (other, header.rs signature): confirmed timeline.rs:348-360 declares Act::{AddTrack,RemoveTrack,Mute,Solo} as a timeline-local enum with zero relation to hotkeys::Action (grep of hotkeys.rs for those variant names returns nothing). Corrected draw_header's return type to Option<Act> and added the bp/pal/font/small params the header-cell block (timeline.rs:1172-1230) closes over.
- Finding 6 (other, inspector_audio/text signatures): confirmed inspector.rs:602-610 clip_section takes 8 params including playhead and fonts, and that the audio branch (1379+) uses playhead at 1510 while the Text branch (949+) uses fonts at 982-986. Added playhead:f64 to inspector_audio::section and fonts:&[String] to inspector_text::section; kept clip_section's own signature as originally documented since it was already correct at 8 params.
- Finding 7 (other, cue_lane.rs signature): confirmed by reading timeline.rs:1914-1950 that the block uses sub_h, subs_lane, pal, small, id, thin, full, mods beyond the previously-listed ui/state/c/painter/playhead_x. Corrected the signature to 13 explicit params and added an explicit risks-section instruction to use wrap-then-cut for cue_lane.rs and header.rs too.
- No finding was rejected — all 7 were confirmed against source and applied.
- Untouched by any finding, preserved as-is: thesis/principles/decisions/registry_protocol/mcp_parity/size_plan/keymap/modifier_table/waves/other 22 workstreams/skip list from the parent skeleton are not part of this per-workstream plan's schema and are not reproduced here; nothing in this workstream's own scope_in/scope_out, model_changes, engine_changes beyond the corrections above, settings/project/glyph/hotkey/mcp_tools fields (all empty, unaffected), or luau paragraph needed any change.
- reconcile: blocker finding (inspector_audio.rs double-create vs audio-dsp-automation) re-verified against live source — inspector.rs:602-611 clip_section and the playhead usage at inspector.rs:1510 both match this plan's existing inspector_audio.rs entry (fn `section`, 6 params: ui, project, ids, playhead, palette, undo) exactly, unchanged. This plan's own files[] entry was never the problem; the fix's actual repair (rename audio-dsp-automation's `show()` back to `section()`, change its op from 'create' to 'modify', conform its params to this signature) belongs entirely in workstream audio-dsp-automation's plan, not here. No change made to split-god-files.
- reconcile: minor finding (tools_playback.rs double-create vs player-rate-loop) re-verified — this plan creates tools_playback.rs first in wave 0a (op:'create' is correct here, it is the origin file for playback./frame./render./sequence./templates. arm bodies per Finding 1 above). The duplicate-create bug is entirely on wave-1 player-rate-loop's side (its entry must become op:'modify'); no change made to split-god-files' own files[] entry or signature.
