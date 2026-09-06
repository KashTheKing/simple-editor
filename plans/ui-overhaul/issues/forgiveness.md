# Forgiveness: toasts, non-blocking confirms, autosave/recovery, cache clear, history restore

**Workstream:** `forgiveness` · **Issue:** [#21](https://github.com/KashTheKing/simple-editor/issues/21) · **Wave:** 1 · **Branch/worktree:** `feat/forgiveness` → `../simple-editor-wt/forgiveness` · **Depends on:** size-diet · **~960 new lines · Δ exe ≈ +76 KB**

Read [agents.md](../../../agents.md), [goals.md](../../../goals.md), [notes.md](../../../notes.md) and the master plan [README.md](../README.md) (registry protocol, keymap, modifier table) before starting.

## Summary

Forgiveness plumbing for wave 1, now REVISED twice (5 prior findings + 4 new audit findings): extend Toast (kind/Undo-action/progress) reusing the 122 existing call sites; replace all 6 rfd::MessageDialog Yes/No sites with a non-blocking confirm::ask queue+window; confirm_discard becomes confirm_discard_then and this workstream lands it plus 7 of its 8 call sites (files.rs x2, actions.rs, drops.rs, windows.rs, app/mod.rs's close-handler) — the 8th, menus.rs's recent-file click, is deliberately left to command-palette's own same-wave menus.rs diff (that file's declared exclusive wave-1 owner) instead of forgiveness reaching into it; debounced off-thread autosave with 20 rolling backups firing the shared fire_hook for project_open/project_save; panic hook + non-blocking recovery offer; Settings.load() keeps its `-> Self` signature (transcribe.rs:46 is a real second caller) and quarantines corrupt JSON via a new private load_inner(), with load_reporting() as the only new public surface for App::new's toast; ConfirmAction drops the RemoveUnused variant (instant+Undo-toast per library.rs's own plan, never confirmed); panes.rs wires library/subtitles response flags to toast_undo; History panel gains click-to-restore; Clear Caches reuses existing cache-clear machinery; the OP_TOOLS structural test that could never fail is dropped for a direct, documented OP_INTERNAL entry. 8 MCP tools cover every capability added.

## Motivation

Serves goals.md's forgiveness/never-block-popups priority and the skeleton's principle "Popups never block" (14 rfd sites -> confirm::ask or undoable+Undo toast) plus critique hooks: panic hook + emergency save, Settings integrity guard, undo/rollback for Settings-level destruction, History label vocabulary, and gap-matrix rows 10-12/105-106 (autosave+recovery, richer toasts, non-blocking confirms, cache clear, history restore) versus Premiere/Resolve/CapCut's autosave+crash-recovery UX.

## In scope

- Toast kind/Undo-action/progress extension
- Non-blocking confirm queue+window replacing all 6 rfd Yes/No sites, including 7 of confirm_discard's 8 call sites (menus.rs's site is command-palette's own diff, see risks)
- Debounced off-thread autosave + rolling backups + recovery offer, firing fire_hook('project_open'/'project_save')
- Panic hook writing crash.log + newest snapshot
- Settings.load() integrity guard (quarantine corrupt file) without breaking its existing signature or its second caller in transcribe.rs
- Single-slot Settings Undo for destructive Settings ops
- History panel click-to-restore
- Clear Caches action + Performance-tab button + size readout
- .sedit.lock warn-only check on project open
- MCP tools for every capability above

## Out of scope

- Command palette / cheat sheet, and the menus.rs recent-file confirm_discard_then call site (command-palette workstream, same file it already exclusively owns this wave)
- First-run onboarding wizard and home screen (layout-modes-onboarding, wave 2)
- Export queue / job progress toasts (export-deliver, wave 2)
- Trim/model primitives, snapping, playback rate, and any actions.rs edits beyond the Delete/RippleDelete arm (other wave-1 workstreams — see risks for the actions.rs coordination note)
- True multi-instance file locking (only a warn-only toast is in scope)
- Any new Glyph icons (existing UndoArrow/text buttons reused)
- fire_hook('selection_changed') (layout-modes-onboarding), fire_hook('import') (media-library), fire_hook('marker_added') (trim-model/audio-analysis) — this workstream owns only project_open/project_save

## Files

| Op | Path | What |
|---|---|---|
| create | src/ui/app/feedback.rs | Toast{msg,kind,at,open_path,action:Option<(String,Action)>,progress:Option<Arc<Progress>>} builder; moves the struct+impl out of app/mod.rs (post-wave0a home of current app.rs:53-67) and the toast-drawing block (current app.rs:6363-6386) into pub(crate) fn draw(app,&egui::Context) registered as a WINDOW_DRAWER. App::toast/toast_with_folder keep their signatures (122 call sites untouched); adds App::toast_undo(msg, Action) and App::push_toast dedupe-by-msg. |
| create | src/ui/confirm.rs | Non-blocking replacement for all 6 rfd::MessageDialog Yes/No sites. ConfirmAction enum (ClearRecent, DeleteTemplate(usize), ClearSubtitles, ReplaceSubtitles(Vec<Cue>), Custom(Box<dyn FnOnce(&mut App)>)) — NO RemoveUnused variant, that path is instant+Undo-toast, never confirmed; thread_local staging queue; ask(title,body,ConfirmAction) and ask_app(title,body,impl FnOnce(&mut App)+'static) wrappers; pub(crate) fn draw(app,ctx) is a WINDOW_DRAWER that renders a small egui::Window per pending item and calls App::resolve_confirm(action) on Yes. |
| create | src/ui/app/autosave.rs | AutosaveState{due:Option<Instant>, last_ok:Option<Instant>}; pub(crate) fn autosave_tick(app:&mut App, ctx:&egui::Context) registered as a FRAME_HOOK — arms `due` lazily when app.dirty && due.is_none(), writes the already-serialized top-of-undo-stack JSON on a spawned thread to %LOCALAPPDATA%\SimpleEditor\autosave\<name-or-untitled>-<n>.sedit (name = project_path file_stem hashed with a short pid suffix to avoid two-instance collisions) keeping the last 20, skipped while project_path is None-and-empty or an export/overwrite is running (App::export.is_some()); calls app.animate_until(ctx, due) so idle CPU stays 0 except at the deadline; clears any autosave dir for the current name on clean save. |
| create | src/ui/app/recovery.rs | pub fn backups() -> Vec<(PathBuf,SystemTime)> scans the autosave dir; pub fn recover_candidate(project_path:Option<&Path>) -> Option<PathBuf> returns the newest autosave newer than the saved file's mtime (or newer than 'now - 1h' if untitled); pub fn install_panic_hook() (called once from main.rs) sets std::panic::set_hook writing %LOCALAPPDATA%\SimpleEditor\crash.log plus the latest snapshot from a static Mutex<Option<String>> that autosave_tick refreshes on every successful write; App-side pub(crate) fn boot(app:&mut App) queues a confirm::ask 'Recover unsaved project?' when recover_candidate() finds one. |
| create | src/ui/app/caches.rs | pub(crate) fn cache_bytes() -> u64 (walks Settings::cache_dir()); pub(crate) fn clear(app:&mut App) — app.player.release_files() (blocks on the existing ack, playback.rs:393-396), app.waveforms.clear(), app.thumbs.clear(), then deletes Settings::cache_dir() contents on a spawned thread; toasts the freed MB via App::toast on completion. |
| modify | src/ui/history_ui.rs | Add a 'Restore' small_button next to 'Delete' on every Editing-category row (386-line file; row rendering at current lines 195-215); change pub fn show(...) -> bool to return pub struct HistoryResponse{pub changed:bool, pub restore:Option<usize>} (Layout rows stay non-restorable per existing comment at lines 8-10). |
| modify | src/settings.rs | Keep `pub fn load() -> Self` signature unchanged (verified: transcribe.rs:46 is a real second caller). Internals become `fn load_inner() -> (Self, Option<String>)` doing the quarantine (on parse failure, rename settings.json -> settings.json.bad best-effort, return (Self::default(), Some(reason))); `load()` becomes `Self::load_inner().0` — transcribe.rs's plain load() gets quarantine-on-corrupt for free, zero-diff for it. Add `pub fn load_reporting() -> (Self, Option<String>) { Self::load_inner() }` used only by App::new so it can toast the reason. Adds within a new `// ---- ws:forgiveness ----` section: `pub lock_warn: bool` (default true) controlling the .sedit.lock toast. |
| modify | src/ui/app/files.rs | open_project (current app.rs:1213-1224, verified via grep): after Project::load succeeds, check/write a sidecar `<path>.lock` next to the .sedit (toast if present, gated by settings.lock_warn), then overwrite with the current pid, then call `app.fire_hook("project_open", json!({"path": path}))` (NEW per audit finding — this workstream owns the project_open/project_save hook events; the other four @on events listed in the mcp_parity skeleton section belong to layout-modes-onboarding/media-library/trim-model, see scope_out). save_project (current app.rs:1377-1389) removes the lock file on success, calls autosave::clear_for(&path), and calls `app.fire_hook("project_save", json!({"path": path}))` (NEW, same reason). App::new calls Settings::load_reporting() instead of load(), toasting the quarantine reason if Some. confirm_discard is rewritten as `pub(crate) fn confirm_discard_then(&mut self, on_yes: impl FnOnce(&mut App) + 'static)`: if !dirty, calls on_yes(self) immediately; else queues confirm::ask_app with save/discard/cancel semantics — non-blocking, no rfd. The two in-file guard-clause callers at 1314/1323 (verified via grep) are refactored to move their remaining function body into the on_yes closure. Both act_overwrite dialogs (1594-1602, 1615-1633) become confirm::ask_app(...) windows. |
| modify | src/ui/app/actions.rs | Delete\|RippleDelete arm (current app.rs:1834-1845): push_undo_labeled(if ripple {'Ripple delete'} else {'Delete'}) instead of push_undo(); after after_edit(), self.toast_undo(format!('{n} clip{s} deleted'), Action::Undo). NewProject arm (app.rs:1711, verified via grep, formerly `if self.confirm_discard() { self.set_project(...) }`) becomes `self.confirm_discard_then(\|app\| app.set_project(Project::new(), None));`. Add Action::ClearCaches => caches::clear(self); Action::RestoreBackup => open the recovery window; Action::UndoSettings => if let Some(s)=self.settings_undo.take() { self.settings=s; self.settings.save(); }. AUDIT NOTE: this file also receives edits from trim-model in wave 1 (new Action variants for trim/edit-point ops); per registry protocol those should route through ACT_HANDLERS, not this legacy match — if trim-model's plan instead edits this same match block, the two PRs must land serially (whichever lands second rebases the ~12-line diff above onto the other); flagged here since actions.rs has no owner named in the wave-1 exclusive-owner list. |
| modify | src/ui/app/drops.rs | The drag-drop project-open at former app.rs:4251 (verified via grep) becomes `self.confirm_discard_then(move \|app\| app.open_project(&dropped0));`. drops.rs is first substantively touched by canvas-handles-monitor in wave 2; this one-line change lands in wave 1 and canvas-handles-monitor rebases onto it. |
| modify | src/ui/app/windows.rs | The import-timeline 'Use this project' accept at former app.rs:6012 (verified via grep) becomes: `if accept { self.confirm_discard_then(move \|app\| { if let Some(r) = app.import_ui.report.take() { app.set_project(r.project, None); app.toast("Imported timeline is now the project"); } app.import_ui.open = false; }); }`. No other workstream owns windows.rs in wave 1. |
| modify | src/ui/app/panes.rs | After `library::show(...)` (former app.rs:2537), `if let Some(n) = resp.removed_unused { app.toast_undo(format!("Removed {n} unused asset(s)"), Action::Undo); }`; after `subtitles_ui::show(...)` (former app.rs:2775), `if resp.cleared_subtitles { app.toast_undo("Cleared subtitles", Action::Undo); }`. |
| modify | src/ui/app/boot.rs | pub(crate) fn run(app:&mut App) called once from the existing `if !self.window_shown {}` first-frame gate in app/mod.rs::update (current app.rs:6045-6049); calls recovery::boot(app) to offer recovery. |
| create | src/ui/app/tools_project.rs | pub const TOOLS: &[ToolDef] — project.autosave (Job\|Mutate), project.recover (Read/Mutate), history.list (Read), history.restore{index} (Mutate), caches.clear (Mutate), caches.size (Read), ui.toast{text,kind?} (Ui), ui.confirm_pending (Read). history.restore/caches.clear/project.recover are added directly to OP_INTERNAL in tools_registry_tests.rs with a documented reason ('lives in src/ui/app/*, not src/model/ops/*, exempt from the model-op scan') rather than a test the scan can never exercise. |
| modify | src/ui/app/mod.rs | Fill the pre-seeded `// ---- ws:forgiveness ----` lines: `mod feedback; mod autosave; mod recovery; mod caches; mod tools_project; mod boot;` and one line each in FRAME_HOOKS, WINDOW_DRAWERS (feedback::draw, confirm::draw), TOOL_TABLES. Fills the App struct's + App::new literal's ws:forgiveness slots with `settings_undo: Option<Settings>`, `confirm_active: Vec<confirm::Pending>`, `pending_close: bool`. The close-handler confirm_discard call in update() (former app.rs:6092, verified via grep) becomes `self.confirm_discard_then(\|app\| { app.pending_close = true; });` with the top of update() re-sending ViewportCommand::Close when pending_close is set. |
| modify | src/ui/library.rs | Delete rfd-based `confirm()` helper (615-632); 'Clear recent' click calls confirm::ask('Clear recent', CLEAR_RECENT, ConfirmAction::ClearRecent). 'Remove unused' click: no confirm gate — `ops.push(LibOp::RemoveUnused)` stays immediate, sets `resp.removed_unused = Some(unused_n)` for panes.rs to toast+Undo. 'Delete template': confirm::ask('Delete template', ..., ConfirmAction::DeleteTemplate(i)). |
| modify | src/ui/subtitles_ui.rs | 'Clear all' (227-233): drop the rfd gate, clear immediately (already inside the undo-snapshotting `once(...)` helper) and set `resp.cleared_subtitles:bool`. Import 'replace?' dialog (828-834): confirm::ask('Import subtitles', ..., ConfirmAction::ReplaceSubtitles(cues)) — the No/append path proceeds inline, no confirmation needed. |
| modify | src/hotkeys.rs | Fill the pre-seeded `// ---- ws:forgiveness ----` rows: `ClearCaches => "clear_caches", "Clear Caches", None;` `RestoreBackup => "restore_backup", "Restore Autosave…", None;` `UndoSettings => "undo_settings", "Undo Settings Change", None;` |
| modify | src/ui/settings_ui/performance.rs | Add a 'Clear Caches' button showing caches::cache_bytes() formatted as MB next to the existing cache_mb slider (current line ~310-312). |
| modify | src/main.rs | Call ui::app::recovery::install_panic_hook() as the first line of main(), before eframe::run_native (current line 59). |

## UI changes

- Toast area gains colored kind + optional 'Undo' button + optional progress bar (visual only, existing bottom-right Area)
- New small non-blocking Confirm window (egui::Window, Foreground order) replacing every native Yes/No dialog this workstream owns (menu Open, drag-drop, import-timeline accept, New Project, window close) — the recent-file-click site is command-palette's own diff against this workstream's confirm_discard_then
- New Recover card/toast offered once at startup when a newer autosave exists
- Settings ▸ Performance gains a 'Clear Caches' button + live cache-size readout next to the existing cache_mb slider
- History panel: each Editing row gains a 'Restore' button beside the existing 'Delete'

## New types and functions

- `pub struct Toast { msg: String, kind: ToastKind, at: Instant, open_path: Option<PathBuf>, action: Option<(String, Action)>, progress: Option<Arc<Progress>> } pub enum ToastKind { Info, Success, Warn, Error }` — src/ui/app/feedback.rs: Extends the current 53-67 struct (msg/at/open_path only) without breaking Toast::new/with_folder; adds .undo(Action) builder method.
- `impl App { fn push_toast(&mut self, t: Toast); fn toast_undo(&mut self, msg: impl Into<String>, undo: Action); }` — src/ui/app/feedback.rs: toast()/toast_with_folder() (current app.rs:1092-1099) become thin wrappers over push_toast; toast_undo is new, used by Delete/RippleDelete/Remove-unused/Clear-subtitles via panes.rs.
- `pub enum ConfirmAction { ClearRecent, DeleteTemplate(usize), ClearSubtitles, ReplaceSubtitles(Vec<crate::model::Cue>), Custom(Box<dyn FnOnce(&mut App)>) } pub struct Pending { title: String, body: String, action: ConfirmAction } pub fn ask(title: impl Into<String>, body: impl Into<String>, action: ConfirmAction); pub fn ask_app(title: impl Into<String>, body: impl Into<String>, on_yes: impl FnOnce(&mut App) + 'static); pub(crate) fn draw(app: &mut App, ctx: &egui::Context);` — src/ui/confirm.rs: Single non-blocking replacement for the 6 rfd::MessageDialog Yes/No sites. No RemoveUnused(usize) variant — that path is instant+Undo-toast, never confirmed.
- `impl App { pub(crate) fn confirm_discard_then(&mut self, on_yes: impl FnOnce(&mut App) + 'static); pub(crate) fn resolve_confirm(&mut self, action: confirm::ConfirmAction); pub(crate) fn settings_snapshot(&mut self, label: &'static str); }` — src/ui/app/files.rs (confirm_discard_then) + src/ui/app/mod.rs (resolve_confirm, settings_snapshot): confirm_discard_then is the ONE new async replacement for the old synchronous `confirm_discard() -> bool`. This workstream implements it and 7 of its 8 former call sites (files.rs x2, actions.rs, drops.rs, windows.rs, app/mod.rs's close-handler); the 8th (menus.rs recent-file click) is command-palette's own diff against this same function, since menus.rs is command-palette's declared exclusive wave-1 file.
- `impl Settings { fn load_inner() -> (Self, Option<String>); pub fn load() -> Self; pub fn load_reporting() -> (Self, Option<String>); }` — src/settings.rs: load() keeps its original `-> Self` signature (transcribe.rs:46 verified as a real second caller) and now quarantines corrupt JSON as a side effect of factoring load_inner() out; load_reporting() is the only new public surface, used solely by App::new.
- `pub struct AutosaveState { due: Option<Instant>, last_ok: Option<Instant> } pub(crate) fn autosave_tick(app: &mut App, ctx: &egui::Context); pub(crate) fn clear_for(project_path: &Path);` — src/ui/app/autosave.rs: FRAME_HOOK; arms lazily on app.dirty, writes off-thread, keeps 20 rolling copies, skipped while untitled/exporting; clear_for deletes this project's autosave copies on a clean save. Callers of open_project/save_project also fire_hook('project_open'/'project_save').
- `pub fn backups() -> Vec<(PathBuf, SystemTime)>; pub fn recover_candidate(project_path: Option<&Path>) -> Option<PathBuf>; pub fn install_panic_hook(); pub(crate) fn boot(app: &mut App);` — src/ui/app/recovery.rs: install_panic_hook (main.rs's only new call) writes crash.log + latest snapshot; boot() queues the non-blocking Recover offer once, called from the existing window_shown first-frame gate.
- `pub(crate) fn cache_bytes() -> u64; pub(crate) fn clear(app: &mut App);` — src/ui/app/caches.rs: release_files -> in-memory clear -> disk delete, in that order; used by Action::ClearCaches and the Performance-tab button.
- `pub struct HistoryResponse { pub changed: bool, pub restore: Option<usize> } pub fn show(ui, state, undo, project) -> HistoryResponse` — src/ui/history_ui.rs: Replaces the current `-> bool` (line 81-86); adds a Restore button per Editing row.

## Actions and hotkeys

| Action id | Label | Chord | Note |
|---|---|---|---|
| clear_caches | Clear Caches |  | Unbound per skeleton keymap; Settings ▸ Performance button + palette row. Runs Player::release_files() + Cmd::ClearDecoders ack, then WaveformCache::clear()+ThumbCache::clear(), then deletes Settings::cache_dir() contents on a worker thread; toasts freed MB. |
| restore_backup | Restore Autosave… |  | Unbound. Opens a small egui::Window (recovery.rs) listing autosave::backups() by mtime; pick one to load via App::set_project (pushes current state as an undo first if dirty). |
| undo_settings | Undo Settings Change |  | Unbound (no keymap row — toast-button only). Pops the single-slot Settings snapshot taken by settings_snapshot(label) and calls settings.save(). AUDIT: not present in the master skeleton keymap table (minor/documentation-only finding); this workstream defines the Action here, docs-refresh should append it to that table — no code change needed since it claims no chord. |

## Persisted fields

**Settings:**

- lock_warn: bool = true (ws:forgiveness section, new)

**Project (.sedit):**

- (none)

## MCP tools (required — every capability must be scriptable)

| Tool | Kind | Args | Description | Maps to |
|---|---|---|---|---|
| project.autosave | mutate | force:bool:false:write an autosave now regardless of the debounce | Force-writes the current project to the autosave folder; returns the path written. | autosave::force_write (new fn beside autosave_tick, same write path) |
| project.recover | mutate | path:string:false:autosave path to load; omitted = the newest candidate | Loads an autosave/backup as the live project (pushes a labeled undo first if the project is dirty). | recovery::recover_candidate + App::set_project |
| history.list | read | limit:number:false:max rows, newest first | Lists undo-stack entries with their lazily-derived labels and categories. | history_ui label derivation over App.undo |
| history.restore | mutate | index:number:true:index into the undo stack (0 = oldest) | Restores the project to that snapshot; refuses Layout-category entries (not restorable). | HistoryResponse.restore handling: push_undo_labeled + Project::from_json + after_edit |
| caches.clear | mutate | none | Releases decoder file handles, clears in-memory waveform/thumb caches, deletes the on-disk cache dir; returns bytes freed. | caches::clear |
| caches.size | read | none | Returns the on-disk cache directory size in bytes. | caches::cache_bytes |
| ui.toast | ui | text:string:true:message; kind:string:false:info\|success\|warn\|error, default info | Shows a toast identical to the app's own (dedupes against the last toast with the same text). | App::push_toast |
| ui.confirm_pending | read | none | Lists titles/bodies of currently-open non-blocking confirm windows, for a script that must wait on user input rather than racing it. | App.confirm_active |

**Luau:** editor.tool('project.autosave'), ('project.recover'), ('history.list'), ('history.restore', {index}), ('caches.clear'), ('caches.size'), ('ui.toast', {text, kind}) all resolve through the same TOOL_TABLES flattening as every other tool — no bespoke Luau binding. This workstream fires the shared fire_hook for 'project_open' and 'project_save' only (audit-assigned); a script's -- @on project_open/@on project_save now has a real call site to fire from. The other four @on events named in the skeleton's mcp_parity narrative (selection_changed, import, export_done, marker_added) are owned by other workstreams (export_done already has a committed site in export-deliver).

## Tests

| Test | File | Asserts |
|---|---|---|
| toast_dedupes_by_message | src/ui/app/feedback.rs | pushing the same msg+kind twice within its lifetime refreshes `at` instead of appending a second toast |
| toast_undo_button_pushes_undo_action | src/ui/app/feedback.rs | a Toast built with .undo(Action::Undo) renders a button that, when clicked, appends Action::Undo to App.pending_actions |
| confirm_never_blocks_frame | src/ui/confirm.rs | ask() and ask_app() return immediately; draw() renders 0 or more windows without waiting on input |
| confirm_resolves_named_action | src/ui/confirm.rs | ConfirmAction::ClearRecent / DeleteTemplate / ClearSubtitles / ReplaceSubtitles each mutate the expected field via App::resolve_confirm when Yes is clicked, and do nothing on No (RemoveUnused is not a ConfirmAction) |
| confirm_discard_then_runs_continuation_without_blocking | src/ui/app/files.rs tests | with dirty=false, confirm_discard_then runs its closure synchronously; with dirty=true, it queues a confirm window and the closure has not run until Yes/No is resolved next frame |
| delete_selected_pushes_undo_toast | src/ui/app/actions_tests.rs (or existing app tests module) | Action::Delete on a non-empty selection labels the undo entry 'Delete', clears selection, and appends a toast whose action is Some((_, Action::Undo)); clicking Undo restores the deleted clips |
| remove_unused_is_instant_and_undoable | src/ui/library.rs tests | clicking Remove Unused with no confirm dialog removes unused assets immediately and the resulting LibraryResponse carries removed_unused == Some(n) for n>0 |
| panes_wires_toast_undo_for_library_and_subtitles | src/ui/app/panes.rs tests | a LibraryResponse.removed_unused = Some(2) and a SubtitlesResponse.cleared_subtitles = true each result in exactly one toast_undo call |
| autosave_arms_once_and_writes_after_debounce | src/ui/app/autosave.rs | after dirty is armed -> due is Some; advancing past due with an injectable clock writes exactly one file to a temp autosave dir and clears due |
| autosave_skips_while_untitled_and_while_exporting | src/ui/app/autosave.rs | autosave_tick is a no-op when project_path is None and dirty is false at start, and when App.export.is_some() |
| open_and_save_fire_project_hooks | src/ui/app/files.rs tests | open_project fires fire_hook('project_open', ..) exactly once on success; save_project fires fire_hook('project_save', ..) exactly once on success (NEW per audit finding 2) |
| recovery_prefers_newer_autosave | src/ui/app/recovery.rs | recover_candidate returns the autosave only when its mtime is strictly newer than the saved project file's mtime; None otherwise |
| corrupt_settings_are_quarantined_not_overwritten | src/settings.rs | writing garbage to settings.json then calling load() (or load_reporting()) renames it to settings.json.bad and returns defaults; load() returns Self, load_reporting() additionally returns Some(reason); a second load() does not re-corrupt anything |
| transcribe_settings_load_unaffected | src/engine/transcribe.rs tests | Settings::load()'s call site in transcribe.rs (verified at line 46) still compiles and returns a plain Settings |
| clear_caches_empties_dir_and_reports_bytes | src/ui/app/caches.rs | seeding Settings::cache_dir() with dummy files, calling clear() removes them and returns the prior total byte count |
| history_restore_pushes_one_labeled_undo | src/ui/history_ui.rs | clicking Restore on an Editing row sets HistoryResponse.restore = Some(i); the app-side handler pushes exactly one new undo entry labeled 'Restore history entry' and the live project equals the restored snapshot |
| history_layout_rows_not_restorable | src/ui/history_ui.rs | no Restore button is rendered for HistoryCategory::Layout rows |
| mutate_rows_roll_back_on_error | src/ui/app/tools_registry_tests.rs | calling history.restore and caches.clear with garbage-typed args leaves project JSON unchanged and pushes no undo |
| assert_no_idle_repaint_with_pending_autosave | src/ui/app/autosave.rs (uses the wave-0 harness helper) | with dirty=true and due armed 5s out, 30 headless frames request zero repaints until within animate_until's window of `due` |
| assert_no_idle_repaint_confirm_window_closed | src/ui/confirm.rs | with no pending confirms, draw() requests no repaint |

## Verification checklist

- [ ] cargo test (whole crate) green, incl. every test above
- [ ] cargo fmt --check / lint:stylua-equivalent clean on touched files
- [ ] grep -rn "rfd::MessageDialog" src/ returns 0 matches
- [ ] grep -rn "egui::Modal" src/ returns 0 matches (never introduced)
- [ ] grep -rn "fn confirm_discard\b" src/ returns 0 matches (only confirm_discard_then remains)
- [ ] manual: delete 3 clips -> toast 'Deleted 3 clips · Undo' -> click Undo -> clips back, selection restored
- [ ] manual: drag-drop a .sedit / accept an imported-timeline report / close the window while the project is dirty -> non-blocking confirm window appears in each case (the recent-file-click case is command-palette's to verify against its own menus.rs diff)
- [ ] manual: open a project, then save it -> a script with -- @on project_open and -- @on project_save each fire exactly once (NEW, audit finding 2)
- [ ] manual: corrupt %APPDATA%\SimpleEditor\settings.json, launch -> app opens with defaults + a toast naming settings.json.bad
- [ ] manual: kill -9 the process mid-edit, relaunch -> non-blocking Recover card appears, accepting it restores the in-progress edit
- [ ] manual: Settings ▸ Performance ▸ Clear Caches shows a byte count and the on-disk cache dir is empty afterward
- [ ] manual: History panel row Restore swaps the project and adds one new History row labeled 'Restore history entry'
- [ ] screenshot: toast with Undo button; confirm window; recovery card — none of them is a blocking OS dialog
- [ ] scripts/size.ps1 -Note forgiveness delta recorded in size_log.csv, within +76 KB of the wave-0 baseline

## Acceptance criteria

- [ ] cargo test passes incl. every test listed below; cargo build --release succeeds
- [ ] 122 existing toast()/toast_with_folder() call sites compile unchanged
- [ ] All 6 rfd::MessageDialog Yes/No sites removed (grep rfd::MessageDialog returns zero matches in app.rs/files.rs, library.rs, subtitles_ui.rs)
- [ ] No egui::Modal added anywhere (grep)
- [ ] Delete/RippleDelete and Remove-unused-assets show an Undo toast that actually restores state on click
- [ ] confirm_discard's 8 former call sites (verified via grep: files.rs x2, actions.rs, drops.rs, windows.rs, app/mod.rs's close-handler owned by this workstream — 6 sites; menus.rs's site is command-palette's own diff against confirm_discard_then; former line 1427 is the definition site itself) all route through confirm_discard_then and none block the frame
- [ ] open_project and save_project each fire the shared fire_hook exactly once, for 'project_open' and 'project_save' respectively (NEW, audit finding 2)
- [ ] Kill -9 mid-edit then relaunch offers Recover via a non-blocking card/toast, never a blocking dialog
- [ ] Corrupt settings.json is renamed to settings.json.bad and defaults load from load()/load_reporting() alike, with App::new's toast naming the reason
- [ ] Clear Caches empties %LOCALAPPDATA%\SimpleEditor\cache and shows a toast with the freed byte count
- [ ] History pane row click restores that snapshot and pushes exactly one new undo entry labeled 'Restore history entry'
- [ ] scripts/size.ps1 delta is within +76 KB of the wave-0 baseline
- [ ] selftest idle step stays green with a pending autosave and with toasts/confirm window open only when a repaint is actually due
- [ ] tool_names_unique_and_namespaced / mutate_rows_roll_back_on_error pass for the new tools_project.rs rows; history.restore/caches.clear/project.recover are listed in OP_INTERNAL with a documented reason

## Risks

| Risk | Mitigation |
|---|---|
| Autosave writing a multi-MB project.to_json() string on the UI thread before handing to the spawned writer thread could hitch a frame on large projects. | to_json() already runs once per edit for the undo stack (push_undo_json), so autosave_tick clones the ALREADY-SERIALIZED top-of-undo-stack JSON instead of re-serializing — zero extra parse cost. |
| Two app instances (or MCP co-editing) on the same .sedit both autosaving and both warning on the lock file can flap or race the lock write. | Lock check/write is warn-only (toast, never blocks open) and always overwrites with the current pid on open; not a real mutex — documented as a known ceiling, not a correctness guarantee. |
| Clear Caches racing WaveformCache/ThumbCache background workers or the Player's decoders still holding file handles on Windows (delete fails silently). | Order is release_files() (blocks on ack, playback.rs:393-396) then in-memory clear() then disk delete; anything still locked is left for next-start cleanup. |
| Converting library.rs's synchronous `confirm()` to a deferred queue changes user-visible timing for Clear Recent / Delete Template. | Those two genuinely need a Yes/No choice so confirm::ask is correct there; Remove Unused is made instant + Undo toast instead of confirmed at all — net UX improvement, not a regression. |
| HistoryResponse's added `restore` field is a breaking signature change to history_ui::show's one call site. | Single call site (app.rs:2836-2840, Pane::History arm) — updated in the same PR. |
| [AUDIT FINDING, CONFIRMED] confirm_discard's 8 call sites (verified by grep at app.rs:1314,1323,1427-def,1711,4053,4251,6012,6092) span files owned by 3 different wave-1 concurrent workstreams: files.rs/actions.rs/drops.rs/windows.rs/app-mod.rs are this workstream's, but line 4053 sits in menus.rs, which command-palette's own wave notes declare its wave-1 exclusive file. The prior revision of this plan edited menus.rs anyway despite flagging the conflict in its own text — an uncoordinated same-wave edit to another workstream's exclusively-owned file. | REMOVED the menus.rs diff from this workstream's files[] entirely. This workstream lands confirm_discard_then and its own 7 call sites; command-palette's own PR (which already touches menus.rs this wave) makes the one-line rewrite of the recent-file click to call confirm_discard_then, since that function will already exist on main by the time either PR needs it (this workstream has no dependency on command-palette, only the reverse for this one line). |
| [AUDIT FINDING, CONFIRMED] src/ui/app/actions.rs receives a same-wave edit from trim-model in addition to this workstream's Delete/RippleDelete-arm edit, with no shared owner named for the file across the two plans. | Documented directly on the actions.rs files[] entry above: per the registry protocol, trim-model's new actions should route through ACT_HANDLERS rather than editing this legacy match; if trim-model's plan instead touches the same match block, whichever PR lands second rebases its diff onto the other (~12 line hunk, low collision risk since the two edits target different match arms). |
| [AUDIT FINDING, CONFIRMED] mcp_parity's skeleton narrative promises 6 Luau @on hook events but only export_done had a committed fire_hook call site anywhere in the plans; project_open/project_save had no owner. | This workstream now owns and implements those two call sites (files.rs open_project/save_project) with a dedicated test (open_and_save_fire_project_hooks). The remaining three events (selection_changed, import, marker_added) are explicitly out of scope here — see scope_out — and must be assigned to layout-modes-onboarding, media-library, and trim-model/audio-analysis respectively, recorded in docs-refresh's final event list. |
| [AUDIT FINDING, MINOR, CONFIRMED but out of scope for this document] UndoSettings (this workstream) and ToggleTranscript (transcript-captions) are unbound Actions absent from the master skeleton's top-level `keymap` array. | No code change needed (neither claims a chord); flagged inline on the undo_settings actions_and_hotkeys entry for docs-refresh to append both rows to the skeleton's keymap table. The keymap array itself is not a field of this per-workstream plan, so it cannot be edited here. |

## Suggested implementation order

1. 1. feedback.rs: extract Toast+draw loop, add kind/action/progress fields, toast_undo/push_toast — keep the 122 call sites green
2. 2. confirm.rs: ConfirmAction enum (no RemoveUnused) + queue + ask/ask_app + draw() WINDOW_DRAWER; App::resolve_confirm
3. 3. files.rs: rewrite confirm_discard as confirm_discard_then (continuation-based, non-blocking); refactor its two in-file guard-clause callers; swap both act_overwrite dialogs to confirm::ask_app; add fire_hook('project_open'/'project_save') calls
4. 4. actions.rs, drops.rs, windows.rs, app/mod.rs (close-handler): update this workstream's remaining 4 confirm_discard call sites to confirm_discard_then — menus.rs's site is explicitly NOT touched here (see risks); leave a one-line note in the PR description for command-palette's reviewer
5. 5. library.rs + subtitles_ui.rs: swap the 4 remaining rfd sites to confirm::ask / instant-undo-toast; delete library::confirm helper
6. 6. panes.rs: wire library's removed_unused and subtitles' cleared_subtitles flags to toast_undo
7. 7. settings.rs: split load() into load_inner()/load()/load_reporting(); App::new switches to load_reporting() and toasts the quarantine reason; lock_warn field
8. 8. autosave.rs + recovery.rs: AutosaveState, autosave_tick FRAME_HOOK, backups()/recover_candidate(), install_panic_hook, boot.rs wired into the existing window_shown gate
9. 9. caches.rs: cache_bytes/clear; Action::ClearCaches; Settings ▸ Performance button
10. 10. history_ui.rs: Restore button + HistoryResponse; app-side restore wiring
11. 11. tools_project.rs: ToolDef rows for every capability above; register in TOOL_TABLES; add history.restore/caches.clear/project.recover to OP_INTERNAL directly
12. 12. tests (see tests[]); run cargo fmt/clippy/tests; scripts/size.ps1

## Deliberate simplifications (`// ponytail:`)

- Undo toast button reuses the EXISTING App.pending_actions + Action::Undo (Ctrl+Z) machinery verbatim — no per-op revert logic, no new undo stack.
- Boot-once logic reuses the EXISTING `if !self.window_shown {}` first-frame gate instead of adding a new App field or a booted-flag FRAME_HOOK.
- Clear Caches has no progress bar — release_files()+clear()+dir delete measured well under a frame on typical cache sizes; add Toast.progress wiring only if a real user reports a multi-second clear.
- ConfirmAction is a closed enum (4 named variants + one Custom escape hatch) rather than a generic dyn-Fn-everywhere design, because library.rs/subtitles_ui.rs cannot see App's private fields — add a variant only when the next cross-module confirm needs one.
- Settings-level Undo is a single slot (one Option<Settings>), not a stack; a second consecutive destructive Settings op silently drops the first Undo offer. Upgrade path: small VecDeque if that's ever reported as surprising.
- autosave file naming keys off (project name or 'untitled') + a short pid suffix — good enough to keep two app instances from clobbering each other's files; true cross-instance coordination is `.sedit.lock`'s job (warn-only), not autosave's.
- settings.rs keeps load() -> Self unchanged (verified transcribe.rs:46 stays untouched) and gets quarantine-on-corrupt for free as a side effect of factoring load_inner() out.
- confirm_discard_then's rollout is now split by file ownership rather than by a single PR touching all 8 sites: this workstream ships 7, command-palette ships the 8th against the same function — avoids the cross-workstream file collision the prior revision shipped anyway.
- history.restore/caches.clear/project.recover go straight into OP_INTERNAL with a one-line reason instead of a test that asserts behavior the real scan (src/model/ops/*.rs only) can never exercise.
- fire_hook('project_open'/'project_save') is the minimal two-line addition needed to make this workstream's slice of the skeleton's @on-event promise real; the other four events are each one line in a different workstream's existing edit, not new files — assign, don't build infrastructure.

## Review trail

- [Round 2 audit] Finding 1 (major, file-ownership collision) CONFIRMED by re-grepping app.rs: all 8 confirm_discard call sites verified at lines 1314,1323,1427(def),1711,4053,4251,6012,6092 — line 4053 is inside the recent-file menu handler, which command-palette's own wave notes declare its exclusive wave-1 file (menus.rs). The prior revision's changelog claimed this conflict was only 'flagged for sign-off' but the files[] diff still edited menus.rs directly. FIXED: removed the menus.rs entry from files[] entirely; this workstream now ships confirm_discard_then plus its own 7 call sites (files.rs x2, actions.rs, drops.rs, windows.rs, app/mod.rs) and leaves the menus.rs rewrite to command-palette's own same-wave diff against the function this workstream defines. Updated acceptance_criteria, verification, scope_in/scope_out, ui_changes, implementation_order step 4, and added a dedicated risks[] entry.
- [Round 2 audit] Finding 1's actions.rs sub-point (major) CONFIRMED as a real cross-plan gap: this workstream's Delete/RippleDelete-arm edit and trim-model's new-action wiring both land in src/ui/app/actions.rs in wave 1 with no owner named. FIXED: added an explicit coordination note directly on the actions.rs files[] entry (route new actions through ACT_HANDLERS per the registry protocol; otherwise whichever PR lands second rebases) and a matching risks[] entry, and named it in scope_out.
- [Round 2 audit] Finding 2 (major, missing fire_hook owners) CONFIRMED by grepping the whole src tree: zero fire_hook call sites exist anywhere in the current source (consistent with it being a wave-0b stub not yet built), and no workstream plan text commits to firing project_open or project_save. FIXED: this workstream now owns and implements those two events — added fire_hook calls to the files.rs open_project/save_project diffs, a new test (open_and_save_fire_project_hooks), a manual verification step, an acceptance_criteria row, a luau field update, and a risks[] entry naming the three remaining events (selection_changed/import/marker_added) as other workstreams' responsibility, listed in scope_out so this plan doesn't silently claim them.
- [Round 2 audit] Finding 3 (major) is the same underlying defect as Finding 1's menus.rs point (both audits independently caught forgiveness's own text flagging-but-shipping the menus.rs edit) — resolved by the same fix as Finding 1; no separate change needed beyond what's already listed.
- [Round 2 audit] Finding 4 (minor, doc-completeness) CONFIRMED: UndoSettings (this workstream) has no row in the master skeleton's top-level keymap array. Since that array is a field of the skeleton document, not of this per-workstream plan, no structural change is possible here — added an inline note on the undo_settings actions_and_hotkeys entry and a risks[] entry directing docs-refresh to append the row (and transcript-captions' ToggleTranscript) to the skeleton's keymap table. No code or chord change, since neither action claims a chord.
- Re-verified via source (grep on HEAD 6b92982): confirm_discard's 8 sites (exact line numbers above), Settings::load()'s two real call sites (app.rs:925, transcribe.rs:46 — confirms the prior revision's transcribe.rs fix was correct and remains unchanged), and zero existing fire_hook implementations (confirms Finding 2's premise). No fix in this round was found to be based on a wrong source citation; all four were applied as described.
- Everything else (Toast extension, confirm.rs core design minus the menus.rs site, autosave/recovery, caches, history restore, all MCP tool rows, all prior-round tests and fixes, size budget) preserved unchanged from the prior revision except est_new_lines +10 (two fire_hook call sites).
