# Export & Deliver

**Workstream:** `export-deliver` · **Issue:** [#28](https://github.com/KashTheKing/simple-editor/issues/28) · **Wave:** 2 · **Branch/worktree:** `feat/export-deliver` → `../simple-editor-wt/export-deliver` · **Depends on:** forgiveness · **~985 new lines · Δ exe ≈ +78 KB**

Read [agents.md](../../../agents.md), [goals.md](../../../goals.md), [notes.md](../../../notes.md) and the master plan [README.md](../README.md) (registry protocol, keymap, modifier table) before starting.

## Summary

Turns the single-shot, single-slot Export window into full delivery: 4 platform preset tiles (YouTube 1080p/4K, Shorts/Reels/TikTok, Instagram) over a collapsed Advanced grid, an Export In/Out Range checkbox wired to the project's own in/out points, Quick Export (Ctrl+M) reusing last options, a VecDeque render queue with ETA (painted in the existing windows.rs progress body) and per-job source-overwrite refusal, letterbox-aware scaling, a loudnorm checkbox that backfills correctly for upgrading users via per-field serde defaults, non-blocking preflight (offline assets/ffmpeg), Render Selection for movie-mode pre-render, and a render-then-optional-ffmpeg-filter Bake pipeline (Render-in-place, Stabilize/deshake, Denoise/afftdn, Slow-mo/minterpolate) that swaps clips onto a fresh asset while keeping the original. Markers gain CSV/YouTube-chapters export and CSV import. All of it lands as 11 typed MCP tools (Read/Mutate/Job/Ui) reachable from Luau, one new Glyph, and structural-test coverage — no model schema change, no new dependency, ~985 new lines / ~78 KB.

## Motivation

Closes gap-matrix rows 62 (render bar realtime-safety), 68 (denoise half), 70 (loudnorm), 91 (stabilize), 92 (slow-mo export half), 102-104 (presets/ETA/queue+range), and critique items 'render in place / bake' and 'export preflight for offline media' — the single biggest beginner-vs-pro gap against CapCut (one-tap platform export) and Premiere/Resolve (queue, range, bake-in-place) while staying inside zero-new-deps by reusing ffmpeg filters already on the machine (deshake/afftdn/minterpolate) instead of any NN or new crate.

## In scope

- PLATFORM_PRESETS/ExportPreset table (4 tiles: YouTube 1080p/4K, Shorts/Reels/TikTok 9:16, Instagram 1:1) + preset tiles UI above a collapsed Advanced grid in the Export window
- Quick Export (Ctrl+M) reusing last options with a first-preset fallback
- ExportOptions.range (in/out or selection) with a dedicated Export window checkbox bound to project.in_point/out_point, and letterbox-aware scaling for aspect-mismatched presets
- Loudness normalization checkbox (fixed -14 LUFS target), correct on both fresh installs and settings.json upgrades
- VecDeque render queue with per-job source-overwrite refusal, ETA/elapsed in the progress window (windows.rs)
- Export preflight (offline assets, ffmpeg missing) via confirm::ask, non-blocking
- Render Selection (explicit-range movie-mode pre-render) + render.range tool
- Bake: Render-in-place, Stabilize (deshake), Denoise (afftdn), Slow-mo (setpts+minterpolate) — each renders the selection, optionally runs one ffmpeg filter, then swaps the clip(s) onto a new asset, original kept, one undo
- Markers export (CSV / YouTube chapters) and import (CSV)
- prerender realtime-safety data (segments_with_heavy) for wave-3 pro-timeline to paint
- 11 MCP tools + Luau exposure + structural-test coverage; Glyph::Queue; 4 actions (1 bound: Ctrl+M)

## Out of scope

- Direct upload / 'Open upload page' browser shell-out (no TLS, no browser automation this workstream)
- Two-pass stabilization (vidstabdetect/vidstabtransform) — single-pass deshake only
- Per-preset fps capping
- User-configurable loudnorm target (fixed -14 LUFS/-1dBTP)
- Parallel/concurrent export or bake jobs — strictly serial through the one Progress slot
- Baking a clip whose mask/graph references another clip/track — refused, not supported
- A new dockable 'Deliver' workspace pane (layout-modes-onboarding's Workspaces cover this)
- Combining a range export with the lossless -c copy fast path (range forces re-encode)

## Files

| Op | Path | What |
|---|---|---|
| modify | src/engine/export.rs | ExportOptions.range/loudnorm/letterbox, ExportPreset+default_export_presets, scale_vf helper, Progress.started/eta, run_export honors range+loudnorm+letterbox, mix_to_wav gains start offset |
| modify | src/engine/convert.rs | ConvertOptions.vf_extra/af_extra, BakeFilter enum, start_bake_filter() |
| modify | src/engine/prerender.rs | segments_with_heavy() realtime-safety data |
| modify | src/ui/markers_ui.rs | MarkerFmt enum, export_markers(), import_markers_csv(), toolbar 'Export…'/'Import…' buttons next to Copy as list |
| modify | src/ui/frame_ui.rs | no functional change this workstream — kept in owns_files only for the shared SCALERS re-export coupling with export_ui.rs |
| modify | src/ui/export_ui.rs | platform preset tiles row (calls apply_preset), Advanced section collapsed under the tiles, 'Export In/Out Range' checkbox bound to project.in_point/out_point populating ExportOptions.range (disables the lossless checkbox when checked), show() returns Option<(ExportChoice,bool queue)> with an 'Add to Queue' button beside 'Export…', loudnorm checkbox; the range-checkbox path is the ONLY UI entry point acceptance-criteria #5 relies on |
| create | src/ui/app/tools_export.rs | TOOLS: &[ToolDef] (11 rows), BakeJob/BakeStage/BakeKind, isolated_project(), swap_clip_asset(), act_quick_export/act_render_selection/act_bake_selection/act_export_markers, frame_tick() draining export_queue + bake_jobs |
| modify | src/ui/app/files.rs | act_export/start_export_choice extended with preflight (confirm::ask) + queue push + letterbox/range wiring; refuses_source() factored out and reused at pop-time; act_export_lossless disabled while a range is set; finish_export pops the next queued job and calls fire_hook("export_done", ..); the moved export_opts() literal (app.rs:1461) gets range:None, loudnorm:self.settings.loudnorm, letterbox:false added explicitly |
| modify | src/ui/app/gpu.rs | new request_prerender_range(a,b) alongside the existing in/out-point request_prerender() |
| modify | src/ui/app/mcp_exec.rs | export.quick/export.bake/export.stabilize/export.slowmo/export.denoise registered as ToolKind::Job through the generalized job list; export.queue/markers.import as Mutate; export.presets/export.status/markers.export as Read; render.range as Ui; the moved ExportOptions literal (app.rs:4853, export.video job) and ConvertOptions literal (app.rs:4878, media.convert job) both get range/loudnorm/letterbox and vf_extra/af_extra explicitly set to None/false so they keep today's behaviour byte-for-byte |
| modify | src/ui/app/windows.rs | exclusive wave 2 for this workstream: the existing inline export/convert progress window body (god_file_split's app.rs:5779-6042 windows() extraction) gains an ETA line (Progress::eta) and a 'N more queued' line under the status text; Cancel also clears export_queue |
| modify | src/settings.rs | ws:export-deliver section: export_presets: Vec<export::ExportPreset> with #[serde(default = "default_export_presets")] (not just the struct Default impl, which serde never calls for a missing individual field); last_export: Option<ExportPresetRef>; loudnorm: bool with a fn default_loudnorm() -> bool { true } and #[serde(default = "default_loudnorm")] so both fresh installs AND existing settings.json files without these keys backfill correctly |
| modify | src/hotkeys.rs | ws:export-deliver section: QuickExport Ctrl+M, RenderSelection/BakeSelection/ExportMarkers unbound |
| modify | src/ui/tools.rs | Glyph::Queue added to the enum, ALL, name()/from_name(), draw_glyph() in the ws:export-deliver sections (stacked document icon with a small clock corner) |
| modify | src/ui/app/mod.rs | ws:export-deliver lines: mod tools_export; App struct fields export_queue: VecDeque<export_ui::ExportChoice>, bake_jobs: Vec<tools_export::BakeJob>; TOOL_TABLES += tools_export::TOOLS; ACT_HANDLERS/FRAME_HOOKS += tools_export::{act_dispatch, frame_tick}; WINDOW_DRAWERS untouched here — the progress window itself lives in the pre-existing windows.rs body, not a new drawer entry |

## Model changes

- None — no Project/Clip schema fields added. Range export reuses the existing Project.in_point/out_point (model.rs:2970-2971) and Render Selection/bake reuse existing public Project::clip_mut/add_asset/all_clips/duration. Bake's clip-isolation and asset-swap are implemented as free functions in tools_export.rs against today's public API, not as new Project ops, so no model/ops file is touched and no OP_TOOLS row is needed.

## Engine changes

- src/engine/export.rs: ExportOptions gains `range: Option<(f64,f64)>`, `loudnorm: bool`, `letterbox: bool`; new `ExportPreset` struct + `default_export_presets()` (4 tiles); new pure `scale_vf((w,h) src, (w,h) out, scaler, letterbox) -> String` factored out of run_export's inline vf-building, adds `scale=W:H:force_original_aspect_ratio=decrease,pad=W:H:(W-iw)/2:(H-ih)/2:color=black` when letterbox && aspect differs, else the existing plain `scale=W:H:flags=`; run_export uses `opts.range.unwrap_or((0.0, project.duration()))` for `dur`/`n`/per-frame `t`, and passes the range start into mix_to_wav; mix_to_wav gains a `start: f64` parameter added to `done/SAMPLE_RATE`; `-af loudnorm=...` appended to the ffmpeg command when `opts.loudnorm && wav.is_some()`; `Progress` gains a private `started: Instant` (set in `Progress::new`) and `pub fn eta(&self) -> Option<Duration>` (elapsed * (1/fraction - 1), None below 1% progress).
- src/engine/convert.rs: `ConvertOptions` gains `vf_extra: Option<String>` and `af_extra: Option<String>`, folded into the existing scale/vf chain and a new `-af` arg in `run_convert`; new `pub enum BakeFilter { Stabilize, Denoise, SlowMo(f64) }` and `pub fn start_bake_filter(src, out, filter, encoder, crf, preset) -> Arc<Progress>` building a `ConvertOptions` with `vf_extra: Some("deshake")` (Stabilize), `af_extra: Some("afftdn")` (Denoise), or `vf_extra: Some(format!("setpts={:.4}*PTS,minterpolate=fps={}:mi_mode=mci", 1.0/factor, target_fps))` (SlowMo) — reuses run_convert's temp-file/-progress-pipe/codec_args plumbing verbatim, no new ffmpeg-invocation code path.
- src/engine/prerender.rs: new `pub fn segments_with_heavy(&self, project: &Project) -> Vec<(f64,f64,bool,bool)>` (from,to,ready,heavy) alongside the existing `segments()`, where heavy = any overlapping clip's `Clip::has_effects()` or `clip.graph.is_some()`; this only produces the realtime-safety data — painting it on the ruler is wave-3 pro-timeline's job, not ours.

## UI changes

- export_ui.rs: 4 platform preset tiles (icon+name, click applies size/crf/loudnorm to state) painted above a collapsed 'Advanced' section holding today's Resolution/Scaler/Encoder/Quality/Preset grid; loudnorm checkbox next to 'Use project background'; a new 'Export In/Out Range' checkbox bound to project.in_point/out_point (disables the lossless-cut checkbox while checked); 'Add to Queue' button beside 'Export…'.
- markers_ui.rs toolbar: 'Export…' (format combo Csv\|YouTube chapters, save dialog) and 'Import…' (open dialog) next to the existing 'Copy as list'.
- Export/convert progress window (windows.rs, the pre-existing inline body): adds an 'ETA 0:32' line under the status text and, when export_queue is non-empty, a '2 more queued' line; Cancel clears the queue too.
- Clip context menu: 'Render in Place (bake to new asset)' entry alongside existing Duplicate/Replace-style entries; a small Stabilize/Denoise/Slow Motion submenu triggering the same bake path with a different BakeKind.
- Ruler context menu: 'Render Selection' entry calling request_prerender_range over the current in/out or clip-selection span.
- Command palette / Help gets the 4 new actions for free once ui.action resolves Action::ALL (command-palette's existing infra, no extra wiring here).

## New types and functions

- `pub struct ExportPreset { pub name: String, pub ext: String, pub width: u32, pub height: u32, pub crf: u32, pub loudnorm: bool }
pub fn default_export_presets() -> Vec<ExportPreset>` — src/engine/export.rs: The 4 platform tiles, stored (not const) so Settings.export_presets is user-editable; also used as the serde field-default fn for Settings.export_presets.
- `ExportOptions { .., pub range: Option<(f64,f64)>, pub loudnorm: bool, pub letterbox: bool }` — src/engine/export.rs: Range = in/out or selection export (set from export_ui.rs's new checkbox, bound to project.in_point/out_point); loudnorm = -af filter toggle; letterbox = fit+pad vs plain stretch (only true from a preset tile, keeps existing custom-resize behaviour byte-identical).
- `fn scale_vf(src: (u32,u32), out: (u32,u32), scaler: &str, letterbox: bool) -> String` — src/engine/export.rs: Factors the inline vf-building out of run_export; adds scale+pad when letterbox && aspect differs, else the existing plain scale.
- `impl Progress { started: Instant (private, set in new()); pub fn eta(&self) -> Option<Duration> }` — src/engine/export.rs: ETA = elapsed * (1/fraction - 1); None below 1% progress.
- `ConvertOptions { .., pub vf_extra: Option<String>, pub af_extra: Option<String> }
pub enum BakeFilter { Stabilize, Denoise, SlowMo(f64) }
pub fn start_bake_filter(src: PathBuf, out: PathBuf, filter: BakeFilter, encoder: String, crf: u32, preset: String) -> Arc<Progress>` — src/engine/convert.rs: Reuses run_convert's ffmpeg/progress plumbing for the 3 filter bakes instead of a new invocation path.
- `pub fn segments_with_heavy(&self, project: &Project) -> Vec<(f64,f64,bool,bool)>` — src/engine/prerender.rs: (from,to,ready,heavy) — heavy = overlapping clip has effects or a graph; feeds wave-3 pro-timeline's render-bar paint.
- `pub enum MarkerFmt { Csv, YoutubeChapters }
pub fn export_markers(project: &Project, fps: f64, fmt: MarkerFmt) -> String
pub fn import_markers_csv(project: &mut Project, csv: &str) -> usize` — src/ui/markers_ui.rs: Text serialization over the existing markers_in_timeline()/add_marker(); returns the count added.
- `pub fn show(ctx, state, project, settings, encoders, exporting) -> Option<(ExportChoice, bool)>` — src/ui/export_ui.rs: bool = true means 'Add to Queue' was clicked instead of 'Export…'; preset tiles set state.preset/custom/loudnorm before either button; a new range-checkbox reads/writes project.in_point/out_point into ExportOptions.range and disables the lossless checkbox while checked.
- `pub(crate) enum BakeKind { Render, Stabilize, Denoise, SlowMo(f64) }
struct BakeJob { stage: BakeStage, clip_ids: Vec<Id>, tmp: PathBuf, out: PathBuf, kind: BakeKind, label: &'static str }
enum BakeStage { Render(Arc<Progress>), Filter(Arc<Progress>) }
fn isolated_project(base: &Project, ids: &[Id]) -> Option<Project>
fn swap_clip_asset(project: &mut Project, clip_id: Id, new_asset: Id)
pub(crate) fn frame_tick(app: &mut App, ctx: &egui::Context)` — src/ui/app/tools_export.rs: Two-stage (render, optional ffmpeg-filter) bake pipeline plus queue-drain, polled once per frame from FRAME_HOOKS; isolated_project keeps only the given clips (zero-shifted), swap_clip_asset is a local, container-agnostic stand-in for trim-model's future replace_clip.
- `pub(crate) fn request_prerender_range(&mut self, a: f64, b: f64)` — src/ui/app/gpu.rs: Render Selection / render.range: request an explicit span without touching in_point/out_point.
- `fn refuses_source(project: &Project, out: &Path) -> bool` — src/ui/app/files.rs: Factored out of start_export_choice's existing canonicalize-and-compare check so the queue can re-check it at pop-time, not only at enqueue-time.
- `fn default_loudnorm() -> bool { true }` — src/settings.rs: Per-field serde default fn so an existing settings.json with no `loudnorm` key backfills true, not bool's own false (serde's struct-level #[serde(default)] only fills a MISSING field with that field's type default, never the custom impl Default for Settings).

## Actions and hotkeys

| Action id | Label | Chord | Note |
|---|---|---|---|
| quick_export | Quick Export | Ctrl+M | Reuses last export options (preset or custom); falls back to the first platform preset with no prior export. Free per skeleton keymap (bare M is AddMarker, ctrl=false). |
| render_selection | Render Selection (pre-render) |  | Unbound; ruler context menu + palette. Calls App::request_prerender_range(sel_start, sel_end). |
| bake_selection | Render in Place (bake to new asset) |  | Unbound; clip context menu + palette. BakeKind::Render over the current clip selection. |
| export_markers | Export Markers… |  | Unbound; Markers pane toolbar button next to 'Copy as list'. |

## New glyphs

- Queue

## Persisted fields

**Settings:**

- export_presets: Vec<export::ExportPreset> — #[serde(default = "default_export_presets")] so a settings.json missing this key backfills the 4 tiles (not an empty Vec); also seeded by default_export_presets() for fresh installs via impl Default
- last_export: Option<ExportPresetRef> where ExportPresetRef is Preset(String) \| Custom{ext,width,height} — drives Quick Export's 'reuse last options'; container-level #[serde(default)] correctly yields None here since Option's type-default already matches the intended default
- loudnorm: bool — #[serde(default = "default_loudnorm")] (fn returning true) so both fresh installs and settings.json files upgrading from before this PR default to true, not bool's type-default false

**Project (.sedit):**

- (none)

## MCP tools (required — every capability must be scriptable)

| Tool | Kind | Args | Description | Maps to |
|---|---|---|---|---|
| export.presets | read | none | List platform export presets (name, ext, size, crf, loudnorm). | Settings.export_presets |
| export.quick | job | preset:string:false:name, default last used; path:string:false:default alongside source | Export with the last-used (or given) preset/options; blocks until the file is written. | tools_export::act_quick_export -> App::start_export_choice |
| export.queue | mutate | preset:string:false:; path:string:true:; range_in:number:false:; range_out:number:false: | Append an export job to the render queue; returns immediately with queue position. | App.export_queue.push_back |
| export.status | read | none | Current export progress (fraction, status, ETA) and queue length. | self.export + self.export_queue |
| export.bake | job | clip_ids:array:true: | Render the selected clip(s) in place (effects flattened) and swap them onto the new asset; original kept. | tools_export::act_bake_selection(BakeKind::Render) |
| export.stabilize | job | clip_ids:array:true: | Bake with ffmpeg deshake, then swap. | BakeKind::Stabilize -> start_bake_filter |
| export.slowmo | job | clip_ids:array:true:; factor:number:false:default 0.5 (0.5=half speed) | Bake with setpts+minterpolate optical-flow slow motion, then swap. | BakeKind::SlowMo(factor) -> start_bake_filter |
| export.denoise | job | clip_ids:array:true: | Bake with ffmpeg afftdn spectral audio denoise, then swap. | BakeKind::Denoise -> start_bake_filter |
| markers.export | read | path:string:true:; format:string:false:csv\|youtube_chapters default csv | Write every marker in timeline order to a CSV or YouTube-chapters text file. | markers_ui::export_markers |
| markers.import | mutate | path:string:true: | Add project markers from a CSV file (time,name[,note,label]). | markers_ui::import_markers_csv -> Project::add_marker |
| render.range | ui | a:number:true:; b:number:true: | Pre-render [a,b) into the movie-mode cache without touching in/out points. | App::request_prerender_range |

**Luau:** "editor.tool(\"export.presets\"\|\"export.quick\"\|\"export.queue\"\|\"export.status\"\|\"export.bake\"\|\"export.stabilize\"\|\"export.slowmo\"\|\"export.denoise\"\|\"markers.export\"\|\"markers.import\"\|\"render.range\", args) — all 11 flow through the existing tool-dispatch bridge, no new Luau surface. Job-kind tools poll the same way as today's export.video/media.convert (script yields on the McpJob until Progress.is_done()). export_done fires via App::fire_hook(\"export_done\", {path, ok}) after every export (queue job or Quick Export), usable from a script's `-- @on export_done` header once command-palette's real fire_hook lands."

## Tests

| Test | File | Asserts |
|---|---|---|
| range_export_frame_count | src/engine/export.rs | opts.range=(1.0,3.0) at 30fps renders exactly 60 frames (n from the range, not project.duration()). |
| letterbox_pads_mismatched_aspect_preset | src/engine/export.rs | scale_vf((1920,1080),(1080,1920),_,letterbox=true) contains force_original_aspect_ratio=decrease and pad=1080:1920; letterbox=false or matching aspect returns the old plain scale= string. |
| loudnorm_appends_af_filter_only_with_audio | src/engine/export.rs | run_export's built command includes -af loudnorm=... when opts.loudnorm && audio present; omitted for a silent/video-only project. |
| eta_is_none_at_start_and_decreasing | src/engine/export.rs | Progress::eta() is None at fraction 0, Some(d1)>Some(d2) as fraction rises for a fixed elapsed clock. |
| bake_filters_build_expected_args | src/engine/convert.rs | start_bake_filter(Stabilize) -> vf contains deshake; Denoise -> af contains afftdn; SlowMo(0.5) -> vf contains setpts=2.0000*PTS and minterpolate; gated real-ffmpeg run via gen_media, skipped if missing. |
| segments_with_heavy_flags_effect_spans | src/engine/prerender.rs | a clip with an added effect makes its covering second heavy=true; a plain cut is heavy=false. |
| markers_csv_round_trip | src/ui/markers_ui.rs | export_markers(Csv) then import_markers_csv on a fresh project reproduces the same marker times/names; YoutubeChapters lines start with an HH:MM:SS timestamp. |
| range_checkbox_sets_opts_range_and_disables_lossless | src/ui/export_ui.rs | checking the new Export In/Out Range box with project.in_point/out_point set populates the returned ExportOptions.range and greys out the lossless-cut checkbox; unchecked reproduces range: None. |
| isolated_project_zero_shifts_selection_only | src/ui/app/tools_export.rs | isolated_project keeps only the given clip ids, earliest start becomes 0, resulting duration equals the selection's span, other tracks are dropped. |
| queue_drains_in_order_and_refuses_source_overwrite | src/ui/app/tools_export.rs | two queued ExportChoice values run sequentially (second starts only once the first's Progress.is_done()); a queued choice whose path canonicalizes to a project source asset is dropped with a toast at pop-time, never started. |
| quick_export_falls_back_to_first_preset | src/ui/app/tools_export.rs | act_quick_export with Settings.last_export == None starts an export using default_export_presets()[0] and toasts which preset was used. |
| settings_backfill_on_upgrade | src/settings.rs | deserializing a settings.json JSON string that has no `export_presets` or `loudnorm` keys yields export_presets == default_export_presets() and loudnorm == true, not [] / false. |
| moved_option_literals_compile_with_new_fields | src/ui/app/files.rs | export_opts() (moved from app.rs:1461) sets range:None, loudnorm:self.settings.loudnorm, letterbox:false — a plain custom export after this PR produces the identical ffmpeg command line it did before (byte comparison). |
| every_edit_op_has_a_tool | src/ui/app/tools_registry_tests.rs | this workstream adds no new `pub fn ...(&mut self` Project op (bake/swap are free fns, not Project methods), so the scan finds nothing new to require a row for. |
| ui_action_covers_every_action | src/ui/app/tools_registry_tests.rs | QuickExport/RenderSelection/BakeSelection/ExportMarkers resolve through ui.action's resolver and appear as palette rows. |
| assert_no_idle_repaint_export_idle | src/ui/app/tests.rs | with self.export None and export_queue/bake_jobs empty, tools_export::frame_tick requests no repaint over 30 headless frames. |
| every_glyph_paints_a_picture | src/ui/tools.rs | extended to cover Glyph::Queue — it paints a non-empty picture like every other glyph. |
| show_headless_with_preset_tiles | src/ui/export_ui.rs | export_ui::show renders the 4 tiles + collapsed Advanced grid + range checkbox without panicking across 2 headless frames; still returns None with no interaction (extends the existing show_headless test). |

## Verification checklist

- [ ] cargo test -p simple-editor engine::export:: engine::convert:: engine::prerender:: ui::export_ui:: ui::markers_ui:: ui::app::tools_export:: settings:: -- new + existing tests green
- [ ] cargo test every_glyph_paints_a_picture every_edit_op_has_a_tool ui_action_covers_every_action tool_names_unique_and_namespaced server_end_to_end mutate_rows_roll_back_on_error
- [ ] cargo run -- --selftest passes including its idle-repaint step
- [ ] scripts/size.ps1 -Note export-deliver; delta <= 96 KB or PR body carries a `size:` line
- [ ] manual: Export window shows 4 preset tiles above a collapsed Advanced section; Shorts preset on a 16:9 project -> ffprobe confirms 1080x1920 with visible pillarbox bars, not a stretch
- [ ] manual: check the range checkbox with in/out points set on the timeline -> exported file duration matches (out-in) within one frame; lossless-cut checkbox is greyed out while it's checked
- [ ] manual: queue two exports to different paths; second starts only after the first's window closes; ETA line shown while running
- [ ] manual: Ctrl+M with no prior export uses the first preset and toasts which one
- [ ] manual: select a clip with a Blur effect -> Bake Selection -> a new asset named for the clip appears in the library, the clip now points at it, Undo restores the original asset
- [ ] manual: Stabilize / Slow Motion (factor 0.5) / Denoise each complete and the resulting clip plays back the filtered picture/audio
- [ ] manual: markers.export to CSV, edit a time, markers.import -> marker moves to the edited time
- [ ] manual: with ffmpeg missing, Export/Quick Export/Bake buttons stay disabled via the existing ffmpeg_missing() toast path
- [ ] manual: rename or delete `loudnorm`/`export_presets` keys from an existing settings.json, relaunch -> loudnorm shows checked and all 4 preset tiles are present (upgrade-path backfill, not just fresh install)

## Acceptance criteria

- [ ] Export window shows 4 platform preset tiles (YouTube 1080p, YouTube 4K, Shorts/Reels/TikTok 9:16, Instagram 1:1) above a collapsed Advanced section; picking one sets size/crf/loudnorm and pillarboxes/letterboxes instead of stretching when the project aspect differs.
- [ ] Ctrl+M (Quick Export) re-runs the last export's options without opening the window; with no prior export it falls back to the first platform preset and toasts which one, never a silent no-op.
- [ ] Two or more exports queued back-to-back run strictly one at a time; the progress window (windows.rs) shows elapsed + ETA and 'N more queued', with a Cancel that also clears the queue.
- [ ] A queued or Quick-Exported job whose path resolves to one of the project's own source assets is refused with the same toast the Export window already gives, checked again at pop-time not just enqueue-time.
- [ ] Export window has an 'Export In/Out Range' checkbox bound to project.in_point/out_point that populates ExportOptions.range; the lossless-cut checkbox is disabled while it's on. A file exported with a range set has duration matching (out-in) within one frame.
- [ ] Loudness checkbox (default on, and defaulting to on for both fresh installs AND existing settings.json files with no loudnorm key) adds `-af loudnorm=I=-14:TP=-1:LRA=11` only when the export has audio; off reproduces byte-identical ffmpeg args to before this change.
- [ ] Render Selection / render.range pre-renders exactly the given [a,b) into the movie-mode cache without disturbing an in-flight in/out-point request.
- [ ] Bake Selection renders the selected clip(s) alone (other tracks/clips stripped, earliest start zeroed), swaps them onto the rendered asset in one undo step, and keeps the original asset in the library.
- [ ] Stabilize / Slow Motion / Denoise each run render-then-filter (deshake / setpts+minterpolate / afftdn) and swap the same way as plain Bake.
- [ ] markers.export writes CSV or YouTube-chapters text at a given path; markers.import adds project markers from a CSV at the recorded times.
- [ ] All 11 new MCP tools appear in tools/list and editor.tools() with valid schemas, are callable from Luau via editor.tool, and every Mutate one rolls back the project JSON on a bad-args call.
- [ ] every_glyph_paints_a_picture, every_edit_op_has_a_tool, ui_action_covers_every_action, tool_names_unique_and_namespaced, and assert_no_idle_repaint all stay green.
- [ ] An existing settings.json missing export_presets/loudnorm keys loads export_presets = the 4 platform tiles and loudnorm = true, not empty/false.
- [ ] The 5 pre-existing ExportOptions/ConvertOptions struct literals outside export_ui.rs (app.rs:1461, 4853, 3329, 4362, 4878 — moving to files.rs/mcp_exec.rs per god_file_split) compile with the new fields explicitly defaulted (range: None, loudnorm/letterbox/vf_extra/af_extra as appropriate).
- [ ] scripts/size.ps1 delta for this PR is <= 96 KB (76 KB estimate + margin) or the PR body carries a `size:` line.

## Risks

| Risk | Mitigation |
|---|---|
| Isolating a clip for bake drops cross-track interactions (a mask or node-graph Combine input from another track/clip) — the baked frame can silently diverge from the visible composite. | Detect a graph/mask referencing another clip id and refuse the bake with a clear toast instead of baking something different; note the limitation in the success toast otherwise. |
| swap_clip_asset's local field-set can drift from trim-model's eventual Project::replace_clip semantics (e.g. link-pair handling) once that lands mid-project. | One call site (tools_export.rs); swap to Project::replace_clip in a follow-up PR the moment trim-model merges, with a ponytail comment pointing at it now. |
| export_queue (UI) and mcp_jobs (MCP export.quick/bake) can both try to start a job the same frame, racing on self.export. | Both gate on self.export.is_none() before starting; frame_tick (queue drain) runs before poll_mcp in App::update, so at most one wins per frame — pin the order with a test. |
| The letterbox/pad scale_vf path changes pixels for any export whose out_size aspect already differed from the project (previously a plain stretch). | ExportOptions.letterbox defaults false; only the preset-tile path sets it true, so the existing manual custom-W×H flow (and its tests) keeps today's plain-stretch behaviour byte-for-byte. |
| Denoise/Stabilize/SlowMo run two sequential background jobs per bake on a large range; no cancel is wired for stage 2 specifically. | Progress.cancel already kills whichever ffmpeg child is active; expose the same Cancel button on the bake toast/job window that export/convert already use — no new cancel plumbing needed. |
| confirm::ask's exact signature (forgiveness, wave 1) isn't verifiable in today's tree — preflight is written against the skeleton's documented shape only. | Read src/ui/confirm.rs at worktree-branch time before wiring the preflight call; only the call site changes if the signature differs, not the decision to preflight. |
| Settings' container-level #[serde(default)] only fills a wholly-missing FIELD with that field's own type default — Vec::new()/false — never the custom impl Default for Settings; without a per-field serde default fn, every pre-existing user's settings.json upgrades to export_presets=[] and loudnorm=false, silently disabling both headline features for every existing install. | export_presets and loudnorm now carry #[serde(default = "default_export_presets")] / #[serde(default = "default_loudnorm")] respectively (see files[]/new_types_and_fns), plus a dedicated serde round-trip test (implementation_order step 15) deserializing a settings.json string that omits both keys and asserting the backfilled values. |
| src/ui/app/windows.rs was not listed in export-deliver's owns_files/touches_shared or in the skeleton's registry_protocol shared-file list, yet the ETA/queue-count feature requires editing its pre-existing inline export/convert progress-window body — an undeclared edit to shared UI code outside the marker-section protocol. | Added `src/ui/app/windows.rs (exclusive wave 2)` to owns_files; the skeleton's registry_protocol point 6 shared-file list should be amended in the same PR that lands wave 0b/registries-schema-hooks to keep the cross-workstream bookkeeping honest (flagged as a skeleton correction, not silently worked around). |

## Suggested implementation order

1. 1. engine/export.rs: ExportPreset/default_export_presets, ExportOptions.range/loudnorm/letterbox, scale_vf, mix_to_wav offset, run_export wiring, Progress.started/eta + tests
2. 2. engine/convert.rs: ConvertOptions.vf_extra/af_extra, BakeFilter, start_bake_filter + tests
3. 3. engine/prerender.rs: segments_with_heavy + test
4. 4. ui/markers_ui.rs: MarkerFmt/export_markers/import_markers_csv + round-trip test, toolbar buttons
5. 5. ui/export_ui.rs: preset tiles, Advanced collapse, loudnorm checkbox, Export In/Out Range checkbox wired to project.in_point/out_point (disables lossless), show() signature change, headless test update
6. 6. settings.rs: export_presets/last_export/loudnorm fields with per-field #[serde(default = ...)] fns + struct Default
7. 7. hotkeys.rs: 4 action rows
8. 8. ui/tools.rs: Glyph::Queue in all 4 sections + glyph test
9. 9. ui/app/tools_export.rs (new): BakeJob/BakeKind/isolated_project/swap_clip_asset, act_* fns, frame_tick, TOOLS table + unit tests
10. 10. ui/app/files.rs: queue+preflight+letterbox+range wiring in act_export/start_export_choice/finish_export; update the moved export_opts() literal (was app.rs:1461) with the 3 new fields
11. 11. ui/app/gpu.rs: request_prerender_range
12. 12. ui/app/mcp_exec.rs: job-list registration for the 5 Job-kind tools; update the two moved struct literals (was app.rs:4853, 4878) with the new fields
13. 13. ui/app/windows.rs: ETA + queue-length line on the existing progress window body
14. 14. ui/app/mod.rs: registry lines + App struct fields
15. 15. serde round-trip test: deserialize a settings.json string missing export_presets/loudnorm keys, assert export_presets == default_export_presets() and loudnorm == true
16. 16. cargo test full pass, scripts/size.ps1 -Note export-deliver, cargo run -- --selftest, manual checklist

## Deliberate simplifications (`// ponytail:`)

- swap_clip_asset bypasses Project::replace_container_media's container-only guard (model.rs:4445-4448) with a local, container-agnostic field-set — delete it and call trim-model's future Project::replace_clip once wave-1 lands; kept local specifically to avoid a cross-workstream dependency this wave.
- Bake isolates a clip by cloning the whole project and stripping every other track/clip, not a dedicated 'render just this layer' compositor mode — correct today, wasteful on huge projects; fine at current sizes, upgrade path is a scoped single-clip render path in Compositor if it ever shows up in a profile.
- Stabilize uses single-pass ffmpeg `deshake`, not two-pass vidstabdetect/vidstabtransform — lower quality, zero extra pipeline stages, matches the skeleton's own wording ('stabilise (deshake)').
- Loudness is one fixed target (-14 LUFS / -1 dBTP) behind a single checkbox — no per-preset or user-tunable target; add a Settings field only if asked.
- Platform presets are 4 seeded Vec entries in Settings.export_presets (so they're already user-editable/removable) rather than a separate read-only const table plus a merge-on-load step; the field now carries its own serde default fn so upgraders backfill the same 4 tiles a fresh install gets.
- export.quick and the render queue share the single self.export slot — true parallel encodes are out of scope; ffmpeg/CPU-bound work stays serial by design.
- windows.rs's export/convert progress body stays one big inline function extended with 2 more lines (ETA, queue count) rather than being carved into a registry-driven drawer — matches how the block already works today; a WINDOW_DRAWER wrapper is only worth it if a second workstream needs to inject into the same window.

## Review trail

- Finding 1 (serde upgrade bug, CONFIRMED — settings.rs:144 has only the struct-level #[serde(default)]; no per-field annotations exist, verified by grep): added `fn default_export_presets()`-as-serde-default and `fn default_loudnorm() -> bool { true }` with per-field #[serde(default = ...)] on export_presets and loudnorm in settings_fields/files[]/new_types_and_fns; added a settings_backfill_on_upgrade test, a manual upgrade-path verification step, and a matching risk entry and acceptance criterion. last_export needed no fix — Option<T>'s container-level default (None) already matches the intended behaviour.
- Finding 2 (undeclared windows.rs ownership, CONFIRMED — export-deliver's files[] already edits windows.rs for the ETA/queue-count line but it was absent from owns_files/touches_shared and from the skeleton's registry_protocol shared-file list): added 'src/ui/app/windows.rs (exclusive wave 2)' to files[] annotation and flagged in risks that the skeleton's registry_protocol point 6 needs the same addition upstream. Did not invent a new WINDOW_DRAWER indirection for this — a ponytail note explains why editing the existing inline body is the right-sized fix.
- Finding 3 (missing UI entry point for range export, CONFIRMED — grepped export_ui.rs for in_point/out_point: zero hits, so the acceptance criterion and manual test were unreachable through the actual window): added an explicit 'Export In/Out Range' checkbox to export_ui.rs's show(), bound to project.in_point/out_point, disabling the lossless checkbox when set. Updated ui_changes, new_types_and_fns' show() signature note, files[]'s export_ui.rs entry, implementation_order step 5, added a range_checkbox_sets_opts_range_and_disables_lossless test, and reworded the affected acceptance criterion and manual verification step. Chose the UI-checkbox fix over descoping, since the plan's own scope_in/acceptance_criteria already promised in-window range export.
- Finding 4 (5 undeclared struct-literal call sites, CONFIRMED via grep — app.rs:1461/4853 for ExportOptions, app.rs:3329/4362/4878 for ConvertOptions): called out explicitly in files[]'s files.rs and mcp_exec.rs 'what' text and added corresponding lines to implementation_order (steps 10 and 12); added a moved_option_literals_compile_with_new_fields test and an acceptance criterion so the byte-identical-behaviour requirement for the untouched literals (3329/4362, outside this PR's touched flows) plus the touched ones (1461/4853/4878) is checked, not just implied.
- No findings were rejected — all 4 verified against source and applied.
