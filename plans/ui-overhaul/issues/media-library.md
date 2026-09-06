# Media library overhaul: offline detection, relink, subclips, smart bins, columns, perf, sequence import

**Workstream:** `media-library` · **Issue:** [#34](https://github.com/KashTheKing/simple-editor/issues/34) · **Wave:** 2 · **Branch/worktree:** `feat/media-library` → `../simple-editor-wt/media-library` · **Depends on:** forgiveness, size-diet, registries-schema-hooks, split-god-files · **~1470 new lines · Δ exe ≈ +118 KB**

Read [agents.md](../../../agents.md), [goals.md](../../../goals.md), [notes.md](../../../notes.md) and the master plan [README.md](../README.md) (registry protocol, keymap, modifier table) before starting.

## Summary

Wave-2 workstream closing the media-library gap: offline-media detection/badge/Relink/Consolidate, subclips + Smart Bins, sortable custom columns, thumbnail-viewport culling, keyboard nav, empty/hierarchy cleanup, and image-sequence import — all reachable as MCP tools. Builds on wave-0 registries (ToolDef/FRAME_HOOKS/App+Settings marker sections) and forgiveness (confirm::ask, push_undo_labeled, toasts). Verified against the live repo. Reconcile fix applied: src/ui/app/media_sync.rs is registries-schema-hooks' wave-0 file (creates AssetStatus{Ready,Decoding,Offline,ProxyBuilding(u8)} + App::asset_status(&self,id) method stub); media-library now only MODIFIES it, filling the method body and adding job orchestration, instead of re-declaring a conflicting AssetStatus shape/free-fn API. All internal call sites (library.rs badge, mcp_tools, new_types_and_fns) updated to the method form. Prior fixes retained: New Subclip pushes undo before mutating; Consolidate is split into background copy-only + UI-thread repoint+undo; batch_strip's multi-select fallback covers only 'Convert…'; Import Report's 'Locate missing…' follows the deferred-mutation idiom; tools_media.rs is op:modify; media-library is library.rs's exclusive wave-2 owner; media_sync's tick fires the `import` hook.

## Motivation

goals.md: zero-missing-media forgiveness and a usable bin at hundreds of clips. Matches Avid/Premiere/Resolve offline-media handling (badge+relink), Avid/Resolve subclips+bins, Premiere/Avid sortable list columns, and closes CapCut/Premiere's biggest library-perf complaint (thumbnail flooding + blocking scans). Serves gap_matrix rows 8(part)/85-89 and critique hooks 4/12/16/20/21; every capability ships as an MCP tool per the hard requirement.

## In scope

- Offline media detection, badge, Library-preview slate, Relink…, Import Report 'Locate missing…' (with live issue-list/count update)
- Subclips (Asset.range/parent + add_subclip, undo pushed before the mutation) and Smart Bins
- Library list custom columns + click-to-sort
- Import CTA / destructive-op demotion / empty state in the Library pane
- Thumbnail viewport culling (verified via a concrete test counter) + non-blocking-friendly scan reuse
- Keyboard navigation in the library
- Image-sequence import (bake to one asset)
- Consolidate media (two-phase: background copy, UI-thread repoint+undo)
- Portable (rel_path) relocation fallback
- media.* + library.* MCP tools for every capability above
- fire_hook("import", ...) on sequence-import job completion (this workstream's owned @on event)

## Out of scope

- Convert…/Compress… dialog UI becoming multi-target (owned by files.rs/windows.rs); Compress stays single-target here
- Source monitor / three-point marks (source-monitor workstream)
- Main Preview-pane offline slate (canvas-handles-monitor/pro-monitor own preview.rs)
- Content-hash-based relink, live frame-accurate sequence decode, any new crate dependency
- A background thread ever holding &mut Project
- Defining or reshaping the AssetStatus enum or App::asset_status's signature — that is registries-schema-hooks' (wave 0) file/type; this workstream only fills the method body and consumes the type

## Files

| Op | Path | What |
|---|---|---|
| modify | src/model/asset.rs | Precondition check first (grep for these names — wave 0 schema-first may have already added them under different names; adapt, don't duplicate). If absent: add #[serde(default)] range: Option<(f64,f64)>, parent: Option<Id>, rel_path: Option<String>. Update the local asset() test-fixture literals wherever Asset{..} is spelled out in full (library.rs:2419, model tests) to include the new fields. |
| modify | src/model/project.rs | Precondition check first. Add #[serde(default)] smart_bins: Vec<SmartBin> and struct SmartBin{name:String, search:String, kind_filter:u8, label_filter:u8, unused_only:bool}, mirroring LibraryState's own filter fields (library.rs:54-59). |
| modify | src/model/ops/assets.rs | add_subclip(parent_id,range)->Option<Id> (pushes directly, bypassing add_asset's by-path dedup at model.rs:3094-3102); consolidate_assets_copy(dir:&Path, assets:&[(Id,String)])->Vec<(Id,PathBuf,Result<(),String>)> — background-safe, file I/O only, no Project access (spawn_job's FnOnce+Send+'static bound). apply_consolidate(project:&mut Project, results:&[(Id,PathBuf,Result<(),String>)])->usize is the foreground half: repoints Asset.path for every Ok result; called from media_sync::tick() on the UI thread inside one push_undo_labeled. relocate_with_rel_path(project,dir) tries a.rel_path under dir before the flat filename fallback; insert_asset_clips (model.rs:3339-3368) gains: when asset.range is Some((s,e)), dur=(e-s).max(MIN_CLIP) and every created Clip gets c.src_in=s. |
| modify | src/ui/library.rs | Offline badge (Glyph::Warning) in row()/tile() badge slots driven by self.asset_status(id) — the App method wave 0 (registries-schema-hooks) declares and this workstream's media_sync.rs fills in; culling fix in row_art (allocate the rect before calling file_art, gate on ui.is_rect_visible(rect)) and asset_tile/file_tile (estimate rect from ui.cursor() before file_art, skip decode when not visible, padded by one row height); LibOp::AssetPath(Id,String), LibOp::SmartBinSave(SmartBin)/SmartBinDelete(usize); LibraryResponse += relink:Vec<Id>, consolidate:bool, new_subclip:Vec<Id>, apply_smart_bin:Option<usize>; asset_menu (1858-1906) gains 'Relink…' (offline only), 'New subclip'; toolbar (882-937) gains 'Consolidate…' button and a 'Columns ▾' menu writing settings.library_columns; smart_bins(&mut self,&mut egui::Ui) sidebar block inserted at the top of imported() (1664); keyboard nav (Up/Down move state.selected, Enter/Space -> add_to_timeline, Delete -> resp.remove) added in browser(); empty-state hint when project.assets.is_empty() in browser(); batch_strip (1150-1186): ONLY 'Convert…' falls back to the quick per-id resp.convert path when sel_ids.len()>1; 'Compress…' unchanged (single-target via self.settings.crf, app.rs:2600-2601). COORDINATION: media-library is library.rs's exclusive wave-2 owner; source-monitor's edit lands as a same-day follow-up PR rebased onto this workstream's merged changes. |
| modify | src/ui/import_ui.rs | Add a 'Locate missing…' button before 'Use this project' (60-70) when r.missing_media>0, following the file's existing deferred-mutation idiom: declare `let mut want_locate = false;` beside `accepted`/`close`, set true on click (no mutation inside the closure — r borrows state.report for the whole body). After .show(ctx, ...) returns, if want_locate: rfd::pick_folder() then engine::import::relocate_report(state.report.as_mut().unwrap(), &dir) — rewrites report.issues/missing_media in place so the table and count update without a separate refresh. |
| modify | src/ui/app/media_sync.rs | RECONCILE FIX (was op:create; now modify — this file is created by registries-schema-hooks wave 0, which already declares `pub(crate) enum AssetStatus { Ready, Decoding, Offline, ProxyBuilding(u8) }` and the stub `impl App { fn asset_status(&self, asset: Id) -> AssetStatus }` always returning Ready). This workstream fills App::asset_status's real body: maintain a HashSet<Id> offline-tracking field on App (ws:media-library section, not serialized) recomputed via Path::exists per asset inside the existing proxy tick (sync_proxies, moved here in wave 0a); asset_status returns Offline for ids in that set, else defers to the existing proxy/decode state for Decoding/ProxyBuilding(pct)/Ready. Do NOT redeclare the enum or add a free-standing `status(app,id)` function — every call site in this workstream uses the method form `app.asset_status(id)` / `self.asset_status(id)`. Also adds: start_import_sequence(app:&mut App, first_frame:&Path)->Result<(),String> spawns a background job (engine::export::spawn_job) that only bakes the video and returns its path — add_asset happens in tick() on completion. start_consolidate(app:&mut App)->Result<(),String> computes the out-of-tree (id,path) list on the UI thread, spawns a background job running ONLY ops::assets::consolidate_assets_copy; result stashed behind an Arc<Mutex<..>> tick() polls. start_relink(app:&mut App, ids:&[Id], dir:&Path) runs synchronously, applies via push_undo_labeled BEFORE mutating + the id->path repoints. tick(app:&mut App, ctx:&egui::Context) is the FRAME_HOOK: recomputes the offline HashSet, polls the sequence-import job (probe finished output, add_asset, toast, fire_hook("import", ...) exactly once) and the consolidate job (on completion, push_undo_labeled('Consolidate media') ONCE then ops::assets::apply_consolidate on the UI thread, toast, after_edit); requests a repaint only while a job is pending or the offline set changed. |
| modify | src/ui/app/library_pane.rs | Wire the new LibraryResponse fields: relink -> open a folder-pick then media_sync::start_relink; consolidate -> media_sync::start_consolidate; new_subclip -> self.push_undo_labeled("New subclip") ONCE BEFORE the loop, THEN ops::assets::add_subclip per id using project.in_point/out_point (default 0..duration), THEN self.after_edit(); apply_smart_bin -> copy the saved SmartBin's fields onto self.library. |
| modify | src/engine/import.rs | See engine_changes. |
| modify | src/media/ffpipe.rs | See engine_changes. |
| modify | src/media/thumbs.rs | Add #[cfg(test)] pub(crate) request_count: std::sync::atomic::AtomicU64 to ThumbCache, incremented once per texture() call under cfg(test) only — gives culling_skips_thumbnail_requests_for_off_screen_rows a concrete counter (ThumbCache is a concrete struct with no trait seam to substitute a mock into). |
| modify | src/ui/tools.rs | Add Glyph::Warning (filled triangle + exclamation dot, like existing Flag/Bookmark shapes at 908-925/1393-1404) and Glyph::Chain (two overlapping stadium/ellipse links) to the enum, ALL (229-308), name()/from_name() (282-406), draw_glyph() match (792+). Both must paint > 0 tessellated vertices (every_glyph_paints_a_picture, ~1698). |
| modify | src/hotkeys.rs | Add RelinkMedia => "relink_media", "Relink Media…", None; ConsolidateMedia => "consolidate_media", "Consolidate Media…", None; NewSubclip => "new_subclip", "New Subclip from Marks", None; (unbound, palette/menu-only, `// ---- ws:media-library ----`). |
| modify | src/settings.rs | Add pub library_columns: Vec<String> (#[serde(default)], default vec!["kind".into(),"duration".into()]) to the ws:media-library section. |
| modify | src/ui/app/tools_media.rs | File already created empty by wave 0a (split-god-files); this wave fills its content. pub const TOOLS: &[ToolDef] with the 8 rows in mcp_tools, each run delegating to media_sync/ops::assets fns; Job-kind tools return ToolOutcome::Job(progress, output_path); media.consolidate's run performs both halves synchronously under one push_undo_labeled since a scripted call has no frame loop. |
| modify | src/ui/app/mod.rs | Fill the pre-seeded ws:media-library line in FRAME_HOOKS with media_sync::tick, in TOOL_TABLES with tools_media::TOOLS, and in ACT_HANDLERS with a fn dispatching Action::RelinkMedia/ConsolidateMedia/NewSubclip to the media_sync/library_pane helpers above. |

## Model changes

- Asset += range: Option<(f64,f64)>, parent: Option<Id>, rel_path: Option<String> (all #[serde(default)], precondition-checked against wave 0 first).
- Project += smart_bins: Vec<SmartBin>; new SmartBin struct (name + the 4 LibraryState filter fields).
- insert_asset_clips honors Asset.range (src_in offset + clamped duration) so a subclip places only its window.
- consolidate_assets split into consolidate_assets_copy (background-safe, file I/O only) and apply_consolidate (&mut Project, UI-thread repoint) instead of one &mut self method — not callable from spawn_job's Send+'static closure otherwise.
- AssetStatus enum and App::asset_status's signature are NOT owned here — they belong to registries-schema-hooks (wave 0); this workstream fills the method body only (reconcile fix).

## Engine changes

- engine/import.rs: ImageSequence{dir,pattern(glob),ext} + detect_sequence(path)->Option<ImageSequence> scanning siblings for a numbered run sharing stem-prefix/ext (>=3, gaps tolerated via glob pattern_type).
- engine/import.rs: relink_by_duration(dir, name_stem, want_secs)->Option<PathBuf> extends the existing by-name relink() with a duration-within-1-frame fallback for renamed files.
- engine/import.rs: relocate_report(report: &mut ImportReport, dir: &Path) -> usize — re-resolves project.assets against dir, then fixes the matching Warning/Skipped Issue in report.issues and decrements report.missing_media so the visible list and count both update.
- media/ffpipe.rs: bake_sequence(seq:&ImageSequence, fps:f64, out:&Path)->Result<(),String> runs ffmpeg -framerate fps -pattern_type glob -i seq.pattern -c:v libx264 -pix_fmt yuv420p -crf 16 out, blocking (called inside spawn_job).

## UI changes

- Warning badge on offline rows/tiles (driven by self.asset_status(id)); hatched offline slate in the Library's own preview box (asset_preview, 1325-1416).
- Asset menu: Relink… (offline only), New subclip; toolbar: Consolidate… button, Columns ▾ menu.
- Smart Bins sidebar section above the folder tree (imported(), 1664).
- Empty-state hint (dashed rect + prompt) when the library has no assets.
- Keyboard nav: arrows/Enter/Space/Delete over the current visible order.
- List rows show configured extra columns (Fps/Size/Label/Tags/Proxy) after the existing Kind/Duration.
- Import Report gains a 'Locate missing…' button that visibly updates the issue table and missing-file count in place.

## New types and functions

- `pub range: Option<(f64,f64)>, pub parent: Option<Id>, pub rel_path: Option<String>  // on Asset` — src/model/asset.rs: Subclip window/parent + portable relocation hint (serde default; precondition-checked against wave 0).
- `pub struct SmartBin { pub name: String, pub search: String, pub kind_filter: u8, pub label_filter: u8, pub unused_only: bool }  // + Project.smart_bins: Vec<SmartBin>` — src/model/project.rs: Named, persisted library filter.
- `pub fn add_subclip(&mut self, parent_id: Id, range: (f64, f64)) -> Option<Id>` — src/model/ops/assets.rs: New Asset row referencing parent's path with its own id/range/parent; bypasses add_asset's by-path dedup.
- `pub fn consolidate_assets_copy(dir: &std::path::Path, assets: &[(Id, String)]) -> Vec<(Id, std::path::PathBuf, Result<(), String>)>` — src/model/ops/assets.rs: Pure file-copy over an explicit (id,path) list, no Project access, satisfying spawn_job's FnOnce+Send+'static bound.
- `pub fn apply_consolidate(project: &mut Project, results: &[(Id, std::path::PathBuf, Result<(), String>)]) -> usize` — src/model/ops/assets.rs: Foreground half: repoints Asset.path for every Ok copy result. Called from media_sync::tick() on the UI thread inside one push_undo_labeled.
- `pub fn insert_asset_clips(&mut self, asset_id: Id, at: f64, video_track: Option<usize>) -> Vec<Id>  // modified` — src/model/ops/assets.rs: Honors asset.range: dur=(e-s).max(MIN_CLIP), c.src_in=s.
- `pub struct ImageSequence { pub dir: PathBuf, pub pattern: String, pub ext: String }
pub fn detect_sequence(path: &std::path::Path) -> Option<ImageSequence>` — src/engine/import.rs: Sibling-scan sequence detection (glob pattern, gap-tolerant).
- `pub fn relink_by_duration(dir: &std::path::Path, name: &str, want_secs: f64, fps: f64) -> Option<PathBuf>` — src/engine/import.rs: Duration-within-1-frame fallback match inside a user-picked folder.
- `pub fn relocate_report(report: &mut ImportReport, dir: &std::path::Path) -> usize` — src/engine/import.rs: Re-resolves report.project against dir AND rewrites report.issues/missing_media so 'Locate missing…' visibly reflects the fix.
- `pub fn bake_sequence(seq: &ImageSequence, fps: f64, out: &std::path::Path) -> Result<(), String>` — src/media/ffpipe.rs: ffmpeg glob-pattern-type mux to one mp4.
- `impl App { fn asset_status(&self, asset: Id) -> AssetStatus }  // BODY FILLED HERE — enum + stub signature are registries-schema-hooks' (wave 0); this workstream never redeclares AssetStatus or adds a free fn` — src/ui/app/media_sync.rs: Single source of truth for the library badge, preview slate, inspector link and export preflight — reconcile fix: eliminates the duplicate AssetStatus/status(app,id) definition media-library's prior draft introduced.
- `pub fn start_import_sequence(app: &mut App, first_frame: &std::path::Path) -> Result<(), String>
pub fn start_consolidate(app: &mut App) -> Result<(), String>
pub fn start_relink(app: &mut App, ids: &[Id], dir: &std::path::Path)
pub fn tick(app: &mut App, ctx: &egui::Context)  // FRAME_HOOK` — src/ui/app/media_sync.rs: Background-job orchestration (copy-only jobs) + the polled hook that applies results, recomputes the offline set feeding asset_status, pushes undo on the UI thread, and fires app.fire_hook("import", ...) once per completed sequence-import job.
- `#[cfg(test)] pub(crate) request_count: std::sync::atomic::AtomicU64  // on ThumbCache` — src/media/thumbs.rs: Concrete, test-only call counter for texture(), giving the culling test a real mechanism instead of an unspecified 'stub cache'.

## Actions and hotkeys

| Action id | Label | Chord | Note |
|---|---|---|---|
| relink_media | Relink Media… | None | Unbound; asset menu + palette. Runs on the current selection (or every offline asset if none). |
| consolidate_media | Consolidate Media… | None | Unbound; Library toolbar button + palette. Confirmed via confirm::ask (forgiveness) before copying. |
| new_subclip | New Subclip from Marks | None | Unbound; asset menu + palette. Uses Project.in_point/out_point; push_undo_labeled fires before add_subclip runs, not after. |

## New glyphs

- Warning: filled triangle outline + a short vertical bar and dot inside (same painter-shape technique as Glyph::Flag/Bookmark) — library offline badge, export preflight, asset_menu.
- Chain: two overlapping ellipse/stadium rings (link) — subclip-parent indicator in the tree, and available for source-monitor/trim-model's own link-lock rows later.

## Persisted fields

**Settings:**

- library_columns: Vec<String> (#[serde(default)], default ["kind","duration"])

**Project (.sedit):**

- smart_bins: Vec<SmartBin> (#[serde(default)])
- Asset.range: Option<(f64,f64)>, Asset.parent: Option<Id>, Asset.rel_path: Option<String> (#[serde(default)] each, precondition-checked against wave 0)

## MCP tools (required — every capability must be scriptable)

| Tool | Kind | Args | Description | Maps to |
|---|---|---|---|---|
| media.status | read | id:integer:false:omit for every asset | AssetStatus per asset id (Ready\|Decoding\|Offline\|ProxyBuilding) from the same tick that drives the library badge. | App::asset_status (method; wave-0 enum, media-library-filled body) |
| media.relink | mutate | ids:array:true:asset ids; dir:string:true:folder to search | Relink offline assets under ids by filename then duration-within-1-frame inside dir (+1 level of subfolders); returns {relinked:[id], still_missing:[id]}. | media_sync::start_relink (push_undo before the repoint, as in the UI path) |
| media.consolidate | mutate | dir:string:false:defaults to the project's own folder | Copy every asset outside dir into it and repoint paths synchronously (copy phase, then one push_undo_labeled repoint), since a script call has no frame loop to poll a job against. | ops::assets::consolidate_assets_copy + apply_consolidate, run back-to-back |
| media.smart_bin | mutate | op:string:true:save\|apply\|list\|delete; name:string:false:; id:integer:false:for apply/delete by index | Manage Project.smart_bins from the current LibraryState filter (save), or apply one to library.select's implicit filter. | Project.smart_bins + LibraryState filter fields |
| media.import_sequence | job | path:string:true:one frame of the sequence; fps:number:false:defaults to project fps | Detect the numbered run `path` belongs to, bake it to one video asset in the background; returns a job handle. Fires the `import` @on Luau event once on completion. | media_sync::start_import_sequence |
| library.select | ui | ids:array:false:; paths:array:false: | Set the library selection (mirrors selection.get/set for the timeline from registries-schema-hooks). | LibraryState.sel_ids/sel_paths |
| library.columns | mutate | columns:array:false:omit to just read; each one of kind\|duration\|fps\|size\|label\|tags\|proxy | Get/set Settings.library_columns. | Settings.library_columns |
| media.batch_convert | job | ids:array:true:; ext:string:true:; width:integer:false:; height:integer:false:; scaler:string:false: | Convert every id with the same options (the options dialog itself stays single-target; this is the scripted batch path). | engine::convert::start_convert, once per id, same ConvertOptions |

**Luau:** Every new capability is a ToolDef, so editor.tool("media.status"\|"media.relink"\|"media.consolidate"\|"media.smart_bin"\|"media.import_sequence"\|"library.select"\|"library.columns"\|"media.batch_convert", args) works with no extra Luau surface. This workstream owns and fires exactly one @on hook event, `import`, once per completed sequence-import job (media_sync::tick); selection_changed/project_open/project_save/export_done/marker_added belong to other workstreams. A script can also watch project.notes or poll media.status(id) in a loop for offline-aware automation.

## Tests

| Test | File | Asserts |
|---|---|---|
| add_subclip_gets_its_own_id_not_the_parents | src/model/ops/assets.rs | two add_subclip calls on the same parent return two distinct ids, both present in project.assets, neither equal to the parent id. |
| insert_asset_clips_honours_asset_range | src/model/ops/assets.rs | placing a subclip Asset{range:Some((2.0,5.0)),..} yields a Clip with src_in==2.0 and duration==3.0. |
| consolidate_assets_copy_skips_files_already_under_dir | src/model/ops/assets.rs | consolidate_assets_copy is given only assets whose path is outside dir; one outside dir is copied and its Result is Ok with the new path. |
| apply_consolidate_repoints_only_ok_results | src/model/ops/assets.rs | apply_consolidate repoints Asset.path only for Ok(()) entries, leaves Err entries untouched, returns the count actually repointed. |
| detect_sequence_finds_a_numbered_run_and_ignores_a_lone_still | src/engine/import.rs | 3+ siblings sharing stem/ext yield Some(ImageSequence) with the right glob pattern; a single still or mixed extensions yield None. |
| relink_by_duration_matches_within_one_frame | src/engine/import.rs | a renamed file with duration within 1/fps of the target is found; one outside that tolerance is not. |
| relocate_report_updates_issues_and_missing_count | src/engine/import.rs | given a report with one Warning issue for a missing file that a picked dir resolves, relocate_report removes/downgrades that issue and decrements missing_media by 1. |
| every_glyph_paints_a_picture | src/ui/tools.rs | extended: Glyph::Warning and Glyph::Chain each tessellate to more vertices than a no-op paint. |
| offline_asset_shows_the_warning_badge | src/ui/library.rs | headless ctx.run with one asset whose path does not exist paints a Warning glyph shape in the row. |
| culling_skips_thumbnail_requests_for_off_screen_rows | src/ui/library.rs | a library of 200 assets inside a short-viewport ScrollArea only bumps ThumbCache's #[cfg(test)] request_count for rows whose estimated rect intersects the clip rect. |
| keyboard_nav_moves_selection_and_deletes | src/ui/library.rs | ArrowDown advances state.selected through the visible order; Enter/Space populate resp.add_to_timeline; Delete populates resp.remove. |
| smart_bin_save_and_apply_round_trips_filters | src/ui/library.rs | saving the current filter as a SmartBin then clearing state and applying the bin restores search/kind_filter/label_filter/unused_only exactly. |
| batch_strip_multi_select_convert_uses_quick_path_compress_stays_single | src/ui/library.rs | with 2+ sel_ids, 'Convert…' populates resp.convert for every id; 'Compress…' still only sets resp.compress = sel_ids.first(). |
| new_subclip_undo_restores_pre_subclip_project | src/ui/app/library_pane.rs | dispatching new_subclip then Ctrl+Z leaves project.assets exactly as it was before the subclip was added. |
| asset_status_reflects_offline_set | src/ui/app/media_sync.rs | App::asset_status returns Offline for an id whose file path does not exist after one tick, and Ready once the path exists again — pins the reconcile fix's single-owner method body. |
| every_edit_op_has_a_tool | src/ui/app/tools_registry_tests.rs | (existing structural test) add_subclip/consolidate_assets_copy/apply_consolidate appear in OP_TOOLS mapped to media.smart_bin/media.consolidate, or in OP_INTERNAL with a reason. |
| ui_action_covers_every_action | src/hotkeys.rs or tools_registry_tests.rs | (existing structural test) RelinkMedia/ConsolidateMedia/NewSubclip round-trip through ui.action's resolver and get a palette row despite being unbound. |
| assert_no_idle_repaint_library_pane | src/ui/app/library_pane.rs | 30 headless frames over a populated, non-searching Library pane with no pending media_sync jobs request zero repaints. |
| sequence_import_completion_fires_import_hook_once | src/ui/app/media_sync.rs | tick() polling a finished sequence-import job calls app.fire_hook("import", ..) exactly once, alongside the existing add_asset + toast. |

## Verification checklist

- [ ] cargo test (crate-wide) green, including the 19 tests above and the existing 657+.
- [ ] cargo clippy clean on touched files.
- [ ] scripts/size.ps1 -Note media-library run before/after; delta reported in the PR body against the ~118 KB budget.
- [ ] --selftest idle step covers opening the Library pane with a populated project and confirms zero repaints after settle.
- [ ] Manual/screenshot check: empty library, 200+ asset library (scroll + zoom), one offline asset (badge+slate+Relink), a saved Smart Bin, a Consolidate… run (undo restores pre-copy paths in one Ctrl+Z), a New Subclip followed by Ctrl+Z, an image-sequence import.
- [ ] MCP: tools/list includes all 8 new tools with correct arg schemas; media.status/media.relink/media.consolidate/media.smart_bin/library.columns exercised via a script, including that media.consolidate's undo is a single step.
- [ ] Grep confirms src/ui/app/media_sync.rs declares no second `enum AssetStatus` and no free-standing `fn status(app: &App, id: Id)` — only the wave-0 enum/method plus this workstream's start_*/tick fns.

## Acceptance criteria

- [ ] Library row/tile shows a warning glyph for any asset whose file is missing (via self.asset_status(id)); Library's own preview shows a hatched offline slate instead of a black/frozen frame for the selected offline asset.
- [ ] Asset context menu on an offline asset offers Relink… (folder picker → match by filename then duration within 1 frame across the picked folder + one level of subfolders) and Import Report's 'Locate missing…' updates the visible issue list and missing_media count in place.
- [ ] Consolidate Media… copies every asset outside the project folder into it (background thread, file I/O only), then on the UI thread in one push_undo_labeled step repoints every copied Asset.path and toasts on completion.
- [ ] New Subclip pushes one undo snapshot BEFORE creating the subclip rows, creates a second Asset row with the same path, its own id, range/parent set, nested under the parent in the tree; Ctrl+Z after New Subclip restores the pre-subclip project.
- [ ] Smart Bins: Save current filter as a named bin persists {search,kind_filter,label_filter,unused_only} on Project; clicking a bin row applies it to LibraryState.
- [ ] List view shows a Columns▾ picker (Kind/Duration/Fps/Size/Label/Tags/Proxy) persisted in Settings.library_columns; clicking Kind or Duration's header still drives the existing sort combo.
- [ ] Scrolling a 500+ asset library only requests thumbnails for rows whose estimated rect is visible, verified by a #[cfg(test)] call-counter on ThumbCache.
- [ ] Arrow keys move the library selection, Enter/Space opens/adds it, Delete removes it (guarded by the same confirm/undo path as the Remove button).
- [ ] Dropping or importing one frame of a numbered sequence (>=3 siblings, same stem/ext) bakes an mp4 in the background and adds it as one asset.
- [ ] Multi-select 'Convert…' falls back to the batch resp.convert path; multi-select 'Compress…' stays single-target.
- [ ] cargo test passes crate-wide; every_glyph_paints_a_picture, every_edit_op_has_a_tool, ui_action_covers_every_action and asset_status_reflects_offline_set stay green; scripts/size.ps1 delta is within budget or annotated.
- [ ] Completing a sequence-import job fires the `import` @on Luau hook exactly once, with the new asset's id and path in the payload.
- [ ] src/ui/app/media_sync.rs contains exactly one AssetStatus enum and exactly one asset_status accessor (the wave-0 App method), never a duplicate or free-function alternative.

## Risks

| Risk | Mitigation |
|---|---|
| Wave 0's registries-schema-hooks may have already added similarly-named Asset/Project fields under different names, causing a duplicate/conflicting schema. | Grep model/asset.rs + model/project.rs for range/parent/rel_path/smart_bins as step 1 of implementation_order; adapt to whatever landed instead of adding a second field. |
| add_subclip via the normal add_asset path (dedup-by-path) would collapse every subclip onto its parent's id. | add_subclip pushes to project.assets directly, bypassing add_asset; a unit test asserts two subclips of one parent get distinct ids. |
| Relink-by-duration false-positives (two unrelated files with the same length) silently reattach the wrong footage. | Duration match is offered inside the user-confirmed Relink… folder pick only, and is one push_undo_labeled step pushed BEFORE the repoint mutation. |
| Sequence bake / consolidate jobs can run long; the frame-count-only toast gives no progress bar. | Toast states frame/asset count up front; ponytail-cut for v1; upgrade path is wiring ffmpeg's -progress pipe like engine::convert::run_convert already does. |
| FRAME_HOOKS/ACT_HANDLERS/TOOL_TABLES marker-section lines drift if this worktree forks before wave 0b's exact pre-seeded text lands. | Rebase onto the default branch tip immediately before touching app/mod.rs and edit only the literal ws:media-library line; if missing, stop and sync with registries-schema-hooks. |
| Thumbnail-culling rect estimate can drift for variable-height rows (long tag lines wrap), causing a visible thumbnail to be skipped. | Pad the estimate by one row height; a test scrolls and asserts, via ThumbCache's #[cfg(test)] request_count, that every row inside the viewport got a decode request after two frames. |
| A background thread calling a &mut Project method has no legal path in this codebase. | consolidate_assets split into consolidate_assets_copy (background, no Project access) and apply_consolidate (&mut Project, called from tick() on the UI thread under one push_undo_labeled). |
| Import Report's 'Locate missing…' button sits inside a closure where a live immutable borrow of state.report already spans the window body. | Follow the file's own accepted/close idiom: set want_locate inside the closure, run pick_folder + relocate_report after show() returns (borrow released); relocate_report takes &mut ImportReport and rewrites issues/missing_media itself. |
| Two wave-2 workstreams (media-library, source-monitor) both edit src/ui/library.rs with no declared owner. | media-library is named library.rs's exclusive wave-2 owner; source-monitor's edit lands as a same-day follow-up PR rebased onto media-library's merged changes. |
| RECONCILE: registries-schema-hooks (wave 0, revised) and media-library (wave 2, prior revision) both independently created src/ui/app/media_sync.rs with different AssetStatus shapes (Ready/Decoding/Offline/ProxyBuilding(u8) vs. no-payload ProxyBuilding) and different call conventions (App::asset_status method vs. a free fn media_sync::status/asset_status) — a duplicate-definition class of bug identical to the Track.color/AudioRole cases, only visible once both revised plans are combined. | media-library's media_sync.rs entry changed from op:create to op:modify; it fills the body of wave-0's App::asset_status(&self,id)->AssetStatus stub (keeping ProxyBuilding(u8)) instead of redeclaring the enum or adding a free function. All call sites (library.rs badge, mcp_tools maps_to, new_types_and_fns) now read `self.asset_status(id)` / `App::asset_status`. A new test (asset_status_reflects_offline_set) and a verification grep pin that no second enum/free-fn is added. |

## Suggested implementation order

1. Rebase onto the default branch tip (wave 0 + forgiveness merged); grep model/asset.rs + model/project.rs for range/parent/rel_path/smart_bins, and grep src/ui/app/media_sync.rs for the wave-0 AssetStatus enum + App::asset_status stub — confirm the exact shape before writing a single line.
2. model/ops/assets.rs: add_subclip, consolidate_assets_copy, apply_consolidate, insert_asset_clips range-awareness + unit tests.
3. engine/import.rs + media/ffpipe.rs: detect_sequence, bake_sequence, relink_by_duration, relocate_report + unit tests.
4. media/thumbs.rs: add the #[cfg(test)] request_count counter on ThumbCache.
5. src/ui/app/media_sync.rs: fill App::asset_status's body (offline HashSet via Path::exists in the proxy tick), tick() polling both job halves + firing fire_hook("import",..), start_import_sequence/start_consolidate/start_relink + App field additions — do not touch the enum or the method signature.
6. src/ui/tools.rs: Glyph::Warning/Chain + every_glyph_paints_a_picture green.
7. src/hotkeys.rs + src/settings.rs: 3 actions + library_columns field.
8. src/ui/library.rs: culling fix, offline badge via self.asset_status(id), asset_menu additions, toolbar (Consolidate/Columns), smart_bins() sidebar, keyboard nav, empty state, batch_strip Convert-only fallback + tests.
9. src/ui/import_ui.rs: Locate missing… using the want_locate-after-show() idiom.
10. src/ui/app/library_pane.rs (new_subclip: push_undo before mutate) + app/mod.rs registry lines: wire responses, register FRAME_HOOKS/ACT_HANDLERS/TOOL_TABLES.
11. src/ui/app/tools_media.rs: 8 ToolDef rows + server_end_to_end/tool_names_unique tests.
12. Full test pass, --selftest idle step, scripts/size.ps1 -Note media-library, screenshots, PR.

## Deliberate simplifications (`// ponytail:`)

- Image sequences are baked to one intermediate mp4 at import instead of a live frame-pattern VideoSource — zero decoder/playback changes. Upgrade path: a SequenceSource if live re-conform ever matters.
- New Subclip reuses Project.in_point/out_point as the range source (no source-monitor dependency). Upgrade path: prefer source-monitor's own src_in/src_out once present.
- Thumbnail culling estimates each row/tile's rect from ui.cursor() before layout rather than rewriting the ScrollArea to show_rows — cheap, occasionally over-requests by one row at the fold.
- Columns are a fixed string vocabulary rendered as extra inline cells, not a resizable table (egui_extras is dropped in size-diet).
- Relink matches by filename then duration-within-1-frame, never content hashing; always asks (folder picker) and is one push_undo_labeled step before the repoint mutation.
- Consolidate's background job does file copies only; the id->path repoint + single undo push happen in the next tick() on the UI thread — one extra Vec round-trip, not new architecture.
- AssetStatus's ProxyBuilding progress payload (u8) is threaded through from wave 0's stub but not populated with real percentages yet — asset_status returns ProxyBuilding(0) as a placeholder until the proxy pipeline reports real progress; upgrade path is wiring the actual proxy-build percentage once that pipeline exposes one.

## Review trail

- reconcile: (blocker) src/ui/app/media_sync.rs's file entry changed from op:'create' to op:'modify' — this file is registries-schema-hooks' (wave 0) creation, which already declares `enum AssetStatus{Ready,Decoding,Offline,ProxyBuilding(u8)}` and the stub method `App::asset_status(&self,id)->AssetStatus`. Media-library's prior draft independently redeclared a conflicting no-payload AssetStatus and a free function `media_sync::status(app,id)`/`media_sync::asset_status` — the same duplicate-definition class of bug already caught for Track.color/AudioRole, visible only once both revised plans are combined. Verified: this is a genuine cross-plan conflict (both plans' own inline text quote incompatible shapes), so the fix is applied, not rejected.
- reconcile: adopted registries-schema-hooks' shape as canonical (ProxyBuilding(u8) kept for future real progress; App::asset_status(&self,id) kept as the method form since wave 0 actually ships it). Updated every media-library call site to match: library.rs's badge now reads self.asset_status(id) (was media_sync::asset_status, a free-fn typo the prior draft itself introduced inconsistently against its own new_types_and_fns which said media_sync::status); mcp_tools['media.status'].maps_to now reads 'App::asset_status'; new_types_and_fns' media_sync.rs entry no longer declares the enum, only documents filling the method body plus the job-orchestration fns (start_import_sequence/start_consolidate/start_relink/tick).
- reconcile: added a placeholder-progress ponytail_note for ProxyBuilding(u8) (returns ProxyBuilding(0) until the proxy pipeline reports real percentages), a new test asset_status_reflects_offline_set pinning the single-owner method body, a verification line grepping for zero duplicate enum/free-fn definitions, a matching acceptance_criteria row, and a risks[] entry documenting the conflict and its resolution. size_delta_kb held at 118 (net wash: removed a duplicate enum, added a small offline-set field) and est_new_lines trimmed slightly (1490->1470) to reflect not re-authoring the enum.
- Everything else (model/engine changes, glyphs, remaining UI changes, settings/project fields, remaining mcp_tools, tests, ponytail_notes, implementation_order, prior audit-fix history for tools_media.rs op/library.rs ownership/import hook ownership) preserved unchanged from the prior revision — no other residual finding named this workstream.
