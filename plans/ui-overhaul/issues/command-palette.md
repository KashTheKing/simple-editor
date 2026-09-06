# Command Palette, Hotkeys Overhaul, Keymap Presets, Script Hooks

**Workstream:** `command-palette` · **Issue:** [#20](https://github.com/KashTheKing/simple-editor/issues/20) · **Wave:** 1 · **Branch/worktree:** `feat/command-palette` → `../simple-editor-wt/command-palette` · **Depends on:** size-diet · **~1310 new lines · Δ exe ≈ +104 KB**

Read [agents.md](../../../agents.md), [goals.md](../../../goals.md), [notes.md](../../../notes.md) and the master plan [README.md](../README.md) (registry protocol, keymap, modifier table) before starting.

## Summary

Unify every command surface: Ctrl+K palette over Action::ALL + Pane::ALL + arg-free ToolDefs + scripts + workspaces (allocation-free fuzzy scorer, rebuilt on query change only); App::enabled(a)->Result<(),&'static str> so disabled hotkeys/menu/palette rows toast a reason instead of silently no-oping; F1 cheat-sheet grouped by section; Settings>Hotkeys gets search, grouping, a painted QWERTY map, Reassign/Keep on conflict, and Premiere/Resolve/Avid keymap presets (diff tables, Settings-snapshot + toast); conflict_all() also sees out-of-table consumers (bare S, Ctrl+Y, Backspace, Esc); per-script `-- @name/@desc/@icon/@hotkey/@on/@budget_ms` headers plus a real fire_hook(event,payload) with a 250ms budget and re-entrancy guard; UI-scale slider; palette TextEdit cursor blink off so idle CPU stays 0.

## Motivation

goals.md discoverability/idle-CPU/forgiveness principles; matrix rows 0,15,16,17,18; critique hooks 'unified command registry', 'disabled-action toasts', 'honest conflict detection'; judges' infra-design winner's command-registry-palette workstream, reconciled with skeleton's frozen Command-stays-local decision.

## In scope

- Ctrl+K command palette (Action+Pane+arg-free Tool+Script+Workspace rows)
- F1 cheat-sheet overlay grouped by section
- Settings>Hotkeys rewrite: search, grouping, QWERTY map, conflict Reassign/Keep, keymap presets
- App::enabled predicate + toast-the-reason wiring
- conflict_all() incl. out-of-table reserved chords
- Per-script header metadata + real fire_hook implementation
- UI scale slider
- 9 new MCP tools + Luau parity for all of the above

## Out of scope

- Find/Ctrl+F (owned by pro-timeline, wave 3)
- WORKSPACES registry population beyond the wave-0b single stub entry (layout-modes-onboarding, wave 2)
- Settings-level Undo *button* wiring (Toast::action lands with forgiveness, wave 1 concurrent; this ws only toasts plainly)
- Per-viewport hotkey routing for pop-outs (layout-modes-onboarding, wave 2)
- Any Action/Gesture from other wave-1 workstreams (trim, playback, audio, color) — only 4 rows of my own
- Any actions!/hotkeys.rs row addition by layout-modes-onboarding for ToggleLayoutMode or ShowWelcome — those two variants are owned solely by this ws (audit-fixed duplicate-declaration risk); layout-modes-onboarding's ACT_HANDLERS must consume, never redeclare, them

## Files

| Op | Path | What |
|---|---|---|
| modify | src/hotkeys.rs | append CommandPalette Ctrl+K / CheatSheet F1 / ToggleLayoutMode Ctrl+Shift+G / ShowWelcome rows under `// ---- ws:command-palette ----`; add Hotkeys.extra: Vec<(String,KeyboardShortcut)> + accessors; add Claim enum, RESERVED table, conflict_all(); add group(Action)->&'static str |
| create | src/keymaps.rs | PRESETS: &[(&str,&[(&str,&str)])] for Simple Editor/Premiere/Resolve/Avid over pre-existing action ids only; apply(name,&mut Hotkeys) |
| modify | src/scripting.rs | add ScriptMeta{path,name,desc,icon,hotkey,on,budget} + meta(path) header-comment parser; add run_hook(...) sharing a new private setup_vm(budget) helper with run() |
| modify | src/settings.rs | append ui_scale:f32=1.0, keymap_preset:String, palette_recent:Vec<String> to Settings + Default under ws marker |
| modify | src/ui/tools.rs | Glyph::Keyboard, Glyph::Search variants + ALL/name/from_name/draw_glyph arms under ws marker |
| create | src/ui/app/enabled.rs | create App::enabled(a)->Result<(),&'static str> with real per-Action predicates + disabled reasons — 0b's registries-schema-hooks owns_files list never commits to this path/module (it lists mod.rs, mcp_exec.rs, tools_args.rs, tools_ui.rs, tools_registry_tests.rs, tools_*.rs rows-only, but no enabled.rs), so this ws creates the file directly and wires its own `mod enabled;` line into src/ui/app/mod.rs's ws:command-palette section rather than assuming a prior stub; default arm stays Ok(()) for actions this ws doesn't know about |
| create | src/ui/palette.rs | Command{Action,Pane,Tool,Script,Workspace}, Row, fuzzy_score, rows(app), PaletteState, show(ctx,app)->Option<Command>, ':' arg-form mode |
| create | src/ui/cheatsheet.rs | show(ctx,&Hotkeys,&mut bool): F1 overlay grouped by hotkeys::group(), palette entry-point row |
| create | src/ui/app/palette_ctl.rs | ACT_HANDLERS act(); WINDOW_DRAWERS windows(); FRAME_HOOKS tick() (1Hz script-meta refresh + script-hotkey poll); App::fire_hook real impl; App::script_metas() cache |
| create | src/ui/app/tools_commands.rs | ToolDef rows: ui.palette, hotkeys.get/set/preset, ui.zoom_factor, scripts.list/run, settings.get/set |
| modify | src/ui/app/menus.rs | Help menu: Command Palette, Keyboard Shortcuts (F1), Show welcome again (stub Action, wave-2 consumes it); Scripts menu: per-script icon/hotkey/desc from ScriptMeta. Audit-fixed hand-off: this ws is the sole wave-1 owner of menus.rs section-based edits; forgiveness's separate one-line recent-file confirm_discard_then rewrite is NOT folded in here (would create an undeclared depends_on for a single line) — it lands as forgiveness's own same-day follow-up commit against this ws's already-merged menus.rs, per the registry-protocol hand-off note in touches_shared, not as a cargo-level dependency |
| modify | src/ui/settings_ui/hotkeys.rs | rewrite hotkeys_tab: search TextEdit, group headers, painted QWERTY grid, Reassign/Keep conflict row, keymap-preset combo, UI-scale slider |
| modify | src/ui/app/mod.rs | append `mod palette_ctl;` and `mod enabled;` (this ws owns creating enabled.rs — see its file entry) to mod list; ACT_HANDLERS += palette_ctl::act; WINDOW_DRAWERS += palette_ctl::windows; FRAME_HOOKS += palette_ctl::tick; TOOL_TABLES += &tools_commands::TOOLS; App struct += palette: palette::PaletteState, script_meta_cache: (Instant,Vec<scripting::ScriptMeta>), hook_running: bool, disabled_hooks: Vec<PathBuf> |

## UI changes

- Ctrl+K palette window (Order::Foreground, text_cursor.blink=false while open, wants_keyboard_input silences other hotkeys same as today)
- F1 cheat-sheet egui::Window/Area, non-blocking
- Settings>Hotkeys tab: search box, group headers, QWERTY rect_filled grid colour-coded bound/modifier/free/reserved, hover lists actions, Reassign/Keep inline row, keymap-preset combo, UI-scale slider
- Help menu: Command Palette / Keyboard Shortcuts (F1) / Show welcome again rows
- Scripts menu rows show glyph + hotkey text + desc tooltip from ScriptMeta

## New types and functions

- `pub enum Command { Action(Action), Pane(Pane), Tool(&'static str), Script(PathBuf), Workspace(&'static str) } pub struct Row { pub cmd: Command, pub label: String, pub shortcut: String, pub glyph: Option<Glyph>, pub enabled: bool, pub reason: Option<&'static str> }` — src/ui/palette.rs: single row model over every command surface, built on query change only
- `pub fn fuzzy_score(query: &str, text: &str) -> Option<u32>` — src/ui/palette.rs: allocation-free case-insensitive subsequence scorer with word-start bonus
- `pub fn rows(app: &App, query: &str) -> Vec<Row>  pub struct PaletteState { pub open: bool, pub query: String, pub sel: usize, pub arg_form: Option<(&'static str, Vec<(String,String)>)> }  pub fn show(ctx: &egui::Context, app: &mut App) -> Option<Command>` — src/ui/palette.rs: row builder, palette state, and the window; ':' prefix builds an arg form from mcp::tools::find(name).map(\|t\| t.args) — post-0b ToolDef is a named-field struct, not the current 3-tuple, so `.2` tuple indexing (valid against today's `TOOLS: &[(&str,&str,&[&str])]` at src/mcp/tools.rs:10) would not compile once registries-schema-hooks lands the ToolDef{name,desc,args,kind,run} conversion this ws depends on
- `pub fn show(ctx: &egui::Context, hotkeys: &Hotkeys, open: &mut bool)` — src/ui/cheatsheet.rs: F1 overlay grouped by hotkeys::group(a)
- `pub enum Claim { Action(Action), Fixed(&'static str) }  pub const RESERVED: &[(&'static str, Modifiers, Key)]  pub fn conflict_all(&self, ks: KeyboardShortcut) -> Option<Claim>  pub fn group(a: Action) -> &'static str` — src/hotkeys.rs: conflict/free-key view honest about bare-S snap toggle, Ctrl+Y redo alias, Backspace delete alias, Esc-fullscreen (Shift+S/AddShape grandfathered, already in the table)
- `pub const PRESETS: &[(&str, &[(&str, &str)])]  pub fn apply(name: &str, hotkeys: &mut Hotkeys) -> Result<(), String>` — src/keymaps.rs: Premiere/Resolve/Avid diffs over pre-existing action ids only (e.g. Premiere: split->Ctrl+K, command_palette->Ctrl+Shift+P)
- `pub struct ScriptMeta { pub path: PathBuf, pub name: String, pub desc: String, pub icon: Option<&'static str>, pub hotkey: Option<String>, pub on: Vec<String>, pub budget: Duration }  pub fn meta(path: &Path) -> ScriptMeta  pub fn run_hook(src: &str, chunk_name: &str, event: &Value, budget: Duration, call: &mut dyn FnMut(&str,&Value)->Result<Value,String>, logs: &mut Vec<String>) -> Result<(), String>` — src/scripting.rs: header-comment metadata without starting the VM; hook runner sets an `editor.event` global via a shared private setup_vm()
- `impl App { pub(crate) fn enabled(&self, a: Action) -> Result<(), &'static str> }` — src/ui/app/enabled.rs: real predicates (export/no ffmpeg or empty timeline, undo/redo empty stack, split/delete no selection, paste empty clipboard, ...); default Ok(()) for unknown actions; file created by this ws (see files[] note on 0b not owning this path)
- `pub(crate) fn act(app: &mut App, a: Action) -> bool  pub(crate) fn windows(app: &mut App, ctx: &egui::Context)  pub(crate) fn tick(app: &mut App, ctx: &egui::Context)  impl App { pub(crate) fn fire_hook(&mut self, event: &'static str, payload: Value)  pub(crate) fn script_metas(&mut self) -> &[scripting::ScriptMeta] }` — src/ui/app/palette_ctl.rs: registry entries + real fire_hook (re-entrancy guard via hook_running, per-script budget, disables an overrunning script for the session with one toast) + 1Hz-cached script metadata

## Actions and hotkeys

| Action id | Label | Chord | Note |
|---|---|---|---|
| command_palette | Command Palette | Ctrl+K | free; Premiere preset rebinds split->Ctrl+K, this->Ctrl+Shift+P |
| cheat_sheet | Keyboard Shortcuts overlay | F1 | free; second entry point into the palette |
| toggle_layout_mode | Layout Mode: Dynamic / Granular | Ctrl+Shift+G | row owned exclusively here per audit fix (blocker): layout-modes-onboarding (wave 2) must NOT re-declare this actions! variant, only wire its ACT_HANDLERS arm to consume it — a second declaration is a duplicate-identifier compile error. Arm itself is inert (falls through) until wave 2 lands. |
| show_welcome | Show Welcome Again |  | row owned exclusively here per audit fix (blocker): layout-modes-onboarding (wave 2) must NOT re-declare this actions! variant, only consume it in its ACT_HANDLERS/WINDOW_DRAWERS arm. Help menu row pre-added here; onboarding window lands with layout-modes-onboarding (wave 2). |

## New glyphs

- Keyboard
- Search

## Persisted fields

**Settings:**

- ui_scale: f32 (default 1.0, applied via ctx.set_zoom_factor)
- keymap_preset: String (default "Simple Editor")
- palette_recent: Vec<String> (command ids, capped 20, recent-first when query empty)

**Project (.sedit):**

- (none)

## MCP tools (required — every capability must be scriptable)

| Tool | Kind | Args | Description | Maps to |
|---|---|---|---|---|
| ui.palette | read | query:string:false:filter text | List palette rows (Actions/Panes/arg-free Tools/Scripts/Workspaces), optionally fuzzy-filtered | palette::rows |
| hotkeys.get | read |  | Every action id, label, chord text, section | Hotkeys + hotkeys::group |
| hotkeys.set | ui | action:string:true:Action id \| chord:string:true:e.g. Ctrl+Shift+B, empty unbinds | Rebind one action | Hotkeys::set + Settings.hotkeys save |
| hotkeys.preset | ui | name:string:true:Simple Editor\|Premiere\|Resolve\|Avid | Apply a keymap preset | keymaps::apply |
| ui.zoom_factor | ui | value:number:false:0.5-2.5, omit to read | Get/set the UI zoom factor | ctx.set_zoom_factor + Settings.ui_scale |
| scripts.list | read |  | Luau scripts with @name/@desc/@hotkey/@on metadata | scripting::list + meta |
| scripts.run | mutate | name:string:true:file stem \| event:object:false:test-run an @on hook's payload | Run a script by name | scripting::run via run_tool_undoable (one undo if project JSON changed) |
| settings.get | read | key:string:false:omit for all whitelisted keys | Read whitelisted scalar settings | Settings scalar whitelist |
| settings.set | ui | key:string:true: \| value:string:true:stringified scalar | Set one whitelisted scalar setting | Settings scalar whitelist, save() |

**Luau:** editor.tool/editor.tools/editor.log unchanged. Scripts gain optional leading `-- @name/@desc/@icon/@hotkey/@on <event>/@budget_ms <ms>` comment lines, parsed by scripting::meta without starting the VM. fire_hook runs every script whose @on list contains the fired event, passing payload as a new `editor.event` global (via run_hook's own setup_vm, not touching run()'s call sites); default hook budget 250ms (vs 5s for a manual run), re-entrancy guarded per App, an overrunning hook is disabled for the session with one toast. editor.tools() gains the 9 new rows automatically via TOOL_TABLES.

## Tests

| Test | File | Asserts |
|---|---|---|
| fuzzy_score_prefers_word_starts | src/ui/palette.rs | word-start/consecutive-char bonuses order rows correctly; non-matching returns None (renamed from fuzzy_score_prefers_word_starts_and_is_alloc_free — no allocation-counting instrumentation exists to test the alloc-free half at runtime; that property is a code-review-time invariant, noted in ponytail_notes) |
| palette_lists_every_action_pane_and_arg_free_tool | src/ui/palette.rs | rows(app,"") contains one row per Action::ALL, Pane::ALL, and every ToolDef with args.is_empty() |
| palette_enter_pushes_one_command | src/ui/palette.rs | headless: type query, Enter -> exactly one pending_actions/run_tool_undoable call, window closes |
| assert_no_idle_repaint_palette_closed_and_open | src/ui/palette.rs | 30 headless frames, no input: zero extra repaint requests in both states (blink disabled while open) |
| reserved_chords_are_free | src/hotkeys.rs | no Action default equals a RESERVED chord, except AddShape's grandfathered Shift+S (documented exception) |
| conflict_all_sees_reserved_and_actions | src/hotkeys.rs | bare S/Ctrl+Y/Backspace/Escape resolve to Claim::Fixed; a bound action's chord resolves to Claim::Action |
| keymap_presets_resolve_and_have_no_duplicate_chords | src/keymaps.rs | every action id in PRESETS exists in Action::ALL; no duplicate chords within one preset |
| script_meta_parses_header_comments | src/scripting.rs | @name/@desc/@icon/@hotkey/@on/@budget_ms parsed; a script with no header gets filename + empty defaults |
| fire_hook_once_reentrant_guarded_and_budget_limited | src/ui/app/palette_ctl.rs | matching @on script runs once per fire_hook call; a script that calls fire_hook again does not recurse; an over-budget script is disabled for the session with exactly one toast |
| enabled_reports_reason_for_known_disabled_actions | src/ui/app/enabled.rs | ExportVideo on empty timeline / no ffmpeg, Undo/Redo on empty stacks, Split/Delete with no selection each return Err(reason); an unrecognised Action returns Ok(()) |
| settings_scalar_whitelist_round_trips_and_rejects_unknown_key | src/ui/app/tools_commands.rs | settings.set on a whitelisted key then settings.get round-trips; a non-whitelisted key returns Err and settings.json is unchanged |
| hotkeys_tab_show_headless_no_change | src/ui/settings_ui/hotkeys.rs | redraw with no input (search box, QWERTY grid, preset combo, scale slider all present) reports changed=false |
| no_duplicate_action_declarations_across_wave1_and_wave2 | src/hotkeys.rs | compile-time proxy: this ws's actions! block is the only declaration site for ToggleLayoutMode and ShowWelcome; a grep-based test over the repo's hotkeys.rs at merge time fails if a second `ToggleLayoutMode` or `ShowWelcome` variant line appears (guards the audit-fixed blocker until layout-modes-onboarding actually lands) |

## Verification checklist

- [ ] cargo test (657 existing + ~13 new, all green, no renamed/removed test besides the deliberate fuzzy_score rename)
- [ ] cargo run -- --selftest (idle-repaint step green with palette open and closed)
- [ ] cargo run -- <video> --screenshot x.ppm; view PNG for: palette with a query, cheat-sheet overlay, Settings>Hotkeys tab with QWERTY map + Premiere preset applied
- [ ] scripts/size.ps1 -Note command-palette; delta <= +104 KB or PR body carries a `size:` line
- [ ] MCP: curl tools/list shows the 9 new tools; editor.tool("hotkeys.preset",{name="Premiere"}) then hotkeys.get shows split on Ctrl+K
- [ ] manual: Ctrl+K -> type -> Enter runs top row; F1 -> click palette row -> opens palette; rebind a key to an already-bound chord -> Reassign/Keep row appears
- [ ] before merging: confirm layout-modes-onboarding's (wave 2) hotkeys.rs diff does not redeclare ToggleLayoutMode/ShowWelcome (audit-fixed blocker) — a merge-time grep, not a unit test, since the two PRs land in different waves

## Acceptance criteria

- [ ] Ctrl+K opens the palette over Action::ALL + Pane::ALL('Show X') + arg-free ToolDefs + Script rows + layout::WORKSPACES; typing filters via fuzzy_score (no per-frame realloc); Enter pushes exactly one Command to pending_actions/run_tool_undoable and closes.
- [ ] F1 opens a cheat-sheet overlay grouped by hotkeys::group(); it surfaces a 'Command Palette (Ctrl+K)' row as the second entry point.
- [ ] Settings > Hotkeys: search box, section-grouped rows, painted QWERTY grid (bound/modifier-bound/free/reserved colours), inline Reassign/Keep row on conflict, keymap-preset combo (Simple Editor/Premiere/Resolve/Avid) that snapshots+toasts, and a UI-scale slider calling ctx.set_zoom_factor.
- [ ] Any Action denied by App::enabled(a) shows its Err reason as a toast from menu, palette AND hotkey dispatch alike (single code path).
- [ ] Scripts menu shows each script's @icon/@hotkey/@desc; pressing a script's own hotkey runs it without opening the menu.
- [ ] ui.palette, hotkeys.get/set/preset, ui.zoom_factor, scripts.list/run, settings.get/set all appear in tools/list, editor.tools() and the palette's ':' mode.
- [ ] fire_hook runs matching @on scripts once per event, budget-limited (250 ms default), re-entrancy guarded, disables an overrunning script for the session with one toast.
- [ ] cargo test all-green (existing 657 + ~13 new), cargo run -- --selftest green incl. idle-repaint step with palette closed AND open, scripts/size.ps1 delta <= +104 KB.
- [ ] ToggleLayoutMode and ShowWelcome are declared exactly once, here, in src/hotkeys.rs — verified by grep before wave-2's layout-modes-onboarding PR merges (audit-fixed blocker).

## Risks

| Risk | Mitigation |
|---|---|
| Ctrl+K is Premiere's Add Edit muscle memory | Premiere keymap preset rebinds split->Ctrl+K and palette->Ctrl+Shift+P; F1 stays a second palette entry point in every preset |
| This ws creates src/ui/app/enabled.rs directly because 0b's registries-schema-hooks owns_files list never commits to owning that file/path (verified: it lists mod.rs/mcp_exec.rs/tools_args.rs/tools_ui.rs/tools_registry_tests.rs/tools_*.rs-rows-only, nothing named enabled.rs); if 0b's real PR lands a differently-shaped stub there first, this ws's create conflicts on landing | Confirm the actual wave-0b PR's file list before starting this file; if enabled.rs (or an equivalent module) already exists by then, switch this entry's op to modify and reconcile signatures — default arm stays Ok(()) for unrecognised actions either way, so no behaviour is lost by the reconciliation |
| Palette TextEdit blinks its cursor, costing idle-CPU while open | set ctx.style_mut(\|s\| s.visuals.text_cursor.blink=false) while palette.open; pinned by assert_no_idle_repaint_palette_closed_and_open |
| Pop-out viewports poll hotkeys on the root ctx only (app.rs update loop) — Ctrl+K/F1 are inert in a popped-out Preview/Timeline | documented ponytail; per-viewport on_viewport hook lands with layout-modes-onboarding (wave 2), not blocking this ws |
| Hand-written hotkeys::group() can drift as other wave-1/2/3 workstreams add Actions to their own marker sections | `_ => "Other"` catch-all keeps it non-breaking; no compile-time enforcement, acceptable for a display-only grouping |
| scripting::run_hook duplicates run()'s Lua-VM setup | both share a new private setup_vm(budget) helper inside scripting.rs (file is solely owned here, zero cross-file risk from touching mcp_exec.rs's run_script call site) |
| Audit-flagged blocker: layout-modes-onboarding (wave 2) independently proposed actions! rows for ToggleLayoutMode/ShowWelcome, which would be a duplicate-enum-variant compile error against this ws's rows once both land | Resolved by making this ws the sole declaration site (see actions_and_hotkeys notes + scope_out); layout-modes-onboarding must only add its ACT_HANDLERS consumer arm. Guarded by a merge-time grep check since the two PRs are in different waves and can't share a compile-time test today. |
| Audit-flagged conflict: command-palette declares menus.rs as its exclusive wave-1 file, but forgiveness (concurrent wave-1) also has a one-line edit there (recent-file confirm_discard_then) | Not folded into this ws's diff (would fabricate an undeclared dependency on forgiveness's confirm::ask for one line); instead this ws lands its section first, forgiveness's one-line rewrite follows as a same-day, explicitly-named follow-up commit against the merged file per the registry-protocol hand-off — no code change required here beyond documenting the sequencing |

## Suggested implementation order

1. src/hotkeys.rs: rows + extra map + group() + Claim/RESERVED/conflict_all + tests
2. src/keymaps.rs: PRESETS + apply() + tests
3. src/scripting.rs: ScriptMeta/meta()/setup_vm/run_hook + tests
4. src/settings.rs: ui_scale/keymap_preset/palette_recent + Default
5. src/ui/tools.rs: Glyph::Keyboard/Search
6. src/ui/app/enabled.rs: create file, real predicates + tests (confirm 0b's actual shape first — see risks)
7. src/ui/palette.rs: Command/Row/fuzzy_score/rows/PaletteState/show + tests
8. src/ui/cheatsheet.rs: show()
9. src/ui/app/palette_ctl.rs: act/windows/tick/fire_hook/script_metas + tests
10. src/ui/app/tools_commands.rs: TOOLS table + tests
11. src/ui/app/mod.rs: wire mod/registries/App fields incl. `mod enabled;`
12. src/ui/app/menus.rs: Help menu + Scripts menu augmentation (no forgiveness fold-in — see risks)
13. src/ui/settings_ui/hotkeys.rs: full tab rewrite
14. cargo test / --selftest / screenshots / scripts/size.ps1
15. post-merge: note the ToggleLayoutMode/ShowWelcome exclusivity for whoever picks up layout-modes-onboarding in wave 2

## Deliberate simplifications (`// ponytail:`)

- group(Action) is a hand-maintained match, not an actions! macro rewrite — smaller diff, `_=>"Other"` covers future actions; fold into the macro only if grouping needs grow.
- PRESETS covers only the ~90 pre-existing action ids (no CapCut, per skeleton); expand once other wave-1 chords (JKL, trims) have actually landed and merged.
- UI-scale slider lives in the Hotkeys tab because it's the only settings_ui file this ws owns; relocate to Appearance/General when that tab's owner accepts a marker section.
- Settings-level Undo for a keymap-preset apply is a plain toast today (no Undo button) — it upgrades automatically once forgiveness's Toast::action lands, same Toast type, no code change needed here.
- Command::Workspace reads layout::WORKSPACES, which is a 1-entry wave-0b stub; the real Edit/Color/Audio/Text/Deliver/Simple list arrives with layout-modes-onboarding (wave 2) for free.
- fuzzy_score's allocation-freedom is a code-review-time property (no .to_string()/Vec alloc in the hot path), not something the test suite instruments or asserts — the test only checks scoring/ordering behaviour.
- The cross-wave hotkeys.rs/menus.rs conflicts are resolved by ordering + a grep-check, not a shared type or a depends_on edge — adding a real dependency between two same-wave concurrent branches for a one-line or one-variant fix would be a heavier fix than the problem, so a documented hand-off is the lazy-correct answer here.

## Review trail

- Audit fix (blocker, applied via scope/action-notes/risks/tests/acceptance/implementation_order): confirmed against this plan's own files[] and actions_and_hotkeys — command-palette already declared ToggleLayoutMode (Ctrl+Shift+G) and ShowWelcome in its hotkeys.rs section, and the audit's fix instructs *layout-modes-onboarding* (a different plan/wave) to drop its duplicate declarations and only consume the variants. Since this plan can't edit another workstream's plan, the correction here is to make the exclusivity explicit and enforceable from this side: strengthened the note text on both actions_and_hotkeys entries, added them to scope_out, added a merge-time grep test (no_duplicate_action_declarations_across_wave1_and_wave2) and a matching verification/acceptance-criteria line, and logged the constraint as a risk with mitigation. No functional code changed — this ws's own rows were already correct.
- Audit fix (major, applied): the menus.rs same-file conflict with forgiveness (both wave 1) was resolved by NOT folding forgiveness's one-line recent-file confirm_discard_then rewrite into this ws's diff — doing so would fabricate an undeclared dependency on forgiveness's src/ui/confirm.rs for a single line, which is a heavier fix than the conflict. Instead: added an explicit note to the menus.rs files[] entry, added a matching risk/mitigation pair, and a ponytail_note explaining why a hand-off note beats a cross-branch dependency here. This ws's own menus.rs scope (Help menu + Scripts menu) is unchanged.
- Audit fix (major, actions.rs/library.rs/panes.rs conflicts): verified against this plan's files[] list (hotkeys.rs, keymaps.rs, scripting.rs, settings.rs, tools.rs, enabled.rs, palette.rs, cheatsheet.rs, palette_ctl.rs, tools_commands.rs, menus.rs, settings_ui/hotkeys.rs, mod.rs) — none of the three conflicting files (src/ui/app/actions.rs, src/ui/library.rs, src/ui/app/panes.rs) are touched by this workstream. No change needed; confirmed out of scope.
- Everything else preserved verbatim: all other scope_in/out lines, all other files/tests/risks/mcp_tools/settings_fields/luau text, depends_on, and implementation_order ordering (only the menus.rs step's inline note and one appended post-merge line were touched). size_delta_kb kept at 104 (no new production code added, only a test and doc strengthening); est_new_lines nudged 1300->1310 for the one added grep-style test.
