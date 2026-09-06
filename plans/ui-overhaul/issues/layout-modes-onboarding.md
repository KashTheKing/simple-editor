# Layout modes, workspaces, pin/glow, and first-run onboarding

**Workstream:** `layout-modes-onboarding` · **Issue:** [#31](https://github.com/KashTheKing/simple-editor/issues/31) · **Wave:** 2 · **Branch/worktree:** `feat/layout-modes-onboarding` → `../simple-editor-wt/layout-modes-onboarding` · **Depends on:** forgiveness, command-palette, snap-engine · **~1265 new lines · Δ exe ≈ +105 KB**

Read [agents.md](../../../agents.md), [goals.md](../../../goals.md), [notes.md](../../../notes.md) and the master plan [README.md](../README.md) (registry protocol, keymap, modifier table) before starting.

## Summary

Progressive Fluidity as layout infra. Adds Settings-driven Dynamic/Granular layout_mode (field pre-added by wave0b), Layout.pinned-aware reveal_auto (Shown|Pinned|Absent), a selection-context FRAME_HOOK mapping SelectionKind to a pane (auto-surface in Dynamic, tab-glow in Granular/pinned) and firing fire_hook("selection_changed", ...) once per change, a WORKSPACES registry (Simple/Edit/Color/Audio/Text/Deliver) driving a menu-bar tab strip + Alt+1..6 + View menu, maximise-pane-under-cursor, pop-out viewport hotkey routing by wiring poll_popout into the on_viewport hook wave-0b pre-places in layout::show, a non-blocking first-run wizard (mode, ffmpeg status, hotkey pointer, template, context-menu opt-in — folding today's guarded contextmenu::install() call into Finish rather than replacing it), and a central-area empty-state + home screen with recent/template cards. Zero new Panes; Pane::Source/pinned field/SelectionKind stub/on_viewport hook/auto_surface+workspace_strip/fire_hook no-op hooks all pre-seeded by wave0b (registries-schema-hooks). ToggleLayoutMode and ShowWelcome hotkey rows are owned by command-palette (already declared there); this workstream only consumes those two Action variants in its ACT_HANDLERS, adding hotkey rows solely for Workspace1..6/MaximizePane/TogglePin/ToggleSource. Settings-only workstream otherwise: no Project/model or engine changes; playback-cache invariants untouched.

## Motivation

goals.md Progressive Fluidity principle + gap rows 1-9 (workspaces, Simple layout, Dynamic/Granular mode, auto-surface, pin, tab glow, first-run, empty states, home screen); architecture hooks "Layout mode + pin + workspace registry" and "Selection-context hook"; skeleton principle "beginner-safe default, pro one gesture deeper" realized as Simple+Dynamic vs Granular+pinned being the same objects; mcp_parity's promised selection_changed @on Luau hook (owned here per audit fix).

## In scope

- Layout::reveal_auto/maximize/unmaximize + Behaviour pin/highlight fields
- WORKSPACES registry + Simple/Audio/Text/Deliver builders (Color reuses colorist_layout)
- Selection-diff FRAME_HOOK -> SelectionKind -> surface() (dynamic) or glow (granular/pinned), and fire_hook("selection_changed", payload) once per change (owns this event per audit fix)
- Workspace tab strip in menu bar + Alt+1..6 + View menu radio
- Maximise-pane-under-cursor (backtick)
- Wiring poll_popout into wave-0b's pre-placed on_viewport hook in layout::show (not adding the hook itself)
- First-run onboarding wizard (mode/ffmpeg/hotkeys/template/context-menu opt-in), folding the existing guarded contextmenu::install() call (settings.context_menu && screenshot.is_none() && !debug_assertions && !is_installed(), app.rs:940-946) into its Finish handler
- Central empty-state Area + Home screen (recent/template cards)
- Adaptive tool-strip reorder by SelectionKind in Dynamic mode
- layout.* + onboarding.reset MCP tools
- Workspace1..6/MaximizePane/TogglePin/ToggleSource hotkeys.rs rows only

## Out of scope

- Per-pane empty-state hints inside timeline/library bodies (owned by snap-engine / media-library)
- Pane::Source content, Color/Scopes/Transcript sections (owned by source-monitor/color-engine/transcript-captions/inspector-gallery)
- layout_mode/onboarded/autosave_secs Settings fields, Pane::ALL slice/stack_unplaced/Pane::Source/Layout.pinned/SelectionKind type, the on_viewport: &mut dyn FnMut(&egui::Context) parameter itself on layout::show, AND the ToggleLayoutMode/ShowWelcome actions! rows (all pre-placed/owned by command-palette or wave0b registries-schema-hooks; this workstream only consumes the two Action variants in ACT_HANDLERS, per audit fix 2)
- Command palette rows/Command enum (owned by command-palette; only coordinate on the WORKSPACES signature it expects)
- Deleting dead show_library/show_inspector fields (size-diet's job)
- Any change to the existing contextmenu::install() guard conditions themselves (debug/screenshot/is_installed checks stay as-is; only the settings.context_menu-equivalent consent path moves into the wizard)
- fire_hook call sites for import/export_done/project_open/project_save/marker_added (owned respectively by media-library, export-deliver, forgiveness, forgiveness, trim-model/audio-analysis per audit fix 1)

## Files

| Op | Path | What |
|---|---|---|
| create | src/ui/onboarding.rs | Non-blocking egui::Window wizard: mode card (Simple+Dynamic vs Classic+Granular -> settings.layout_mode + undo-preserving layout swap), ffmpeg status card (cached Status::compute, no sync probe), hotkeys card (deep-link to settings_ui hotkeys tab), template card (settings.project_templates), context-menu opt-in checkbox (defaults to settings.context_menu, checked). Finish sets settings.onboarded=true and, if the checkbox is checked AND !cfg!(debug_assertions) AND !contextmenu::is_installed(), calls contextmenu::install() — the same guard app.rs:940-946 already applies, now gated on this consent instead of being unconditional-on-settings-flag. |
| create | src/ui/home.rs | Central egui::Area home/empty-state overlay shown only when project+library are empty and Settings.home_screen; Open/Import/New-from-template/Recent(settings.recent_projects) buttons return a HomeAction the caller turns into pending_actions. Dismissable. |
| create | src/ui/app/layout_ctl.rs | ACT_HANDLERS entry act() for Workspace1..6/MaximizePane/TogglePin/ToggleSource (bound here) plus consuming the pre-existing Action::ToggleLayoutMode/Action::ShowWelcome variants whose hotkey rows are declared by command-palette, not here; upgraded surface(pane) reading layout_mode+pinned via reveal_auto; workspace_strip(app,ui) painter; poll_popout(hotkeys,ctx)->Vec<Action> whose signature matches whatever wave-0b's landed on_viewport: &mut dyn FnMut(&egui::Context) closure type expects (read that stub before writing this, not assumed). |
| create | src/ui/app/frame.rs | FRAME_HOOKS entry tick(app,ctx): diffs (selection, sel_transitions, timeline.sub_sel, and the edit-point field snap-engine lands on TimelineState — confirm its actual name/type in that wave-1 landing before wiring, not assumed as `timeline.edit_point`) into SelectionKind once per frame, calls layout_ctl::surface or pushes a glow entry, and calls app.fire_hook("selection_changed", payload) exactly once per distinct diffed value (owns this event, per audit fix 1) with a re-entrancy/budget guard reused from wave0b's fire_hook stub; decays the glow list via animate_until; arms onboarding on first idle frame post-boot when unset. |
| create | src/ui/app/tools_layout.rs | TOOLS: &[ToolDef] implementing layout.mode/workspace/pin/surface/maximize/list and onboarding.reset, each a thin call into layout_ctl/Layout/Settings. |
| modify | src/ui/layout.rs | Add Surfaced enum + reveal_auto (guards on find_pane().is_some(), checks pinned active sibling via tiles.parent_of); maximize/unmaximize (tree-JSON stash, same trick as profile switch); WORKSPACES:&[(&str,Glyph,fn()->Layout)]; simple_layout/audio_layout/text_layout/deliver_layout builders using the wave0b stack_unplaced helper. Behaviour gains pin:Vec<Pane> + highlight:Vec<(Pane,f32)> with tab_ui/top_bar_right_ui paint. Do NOT add the on_viewport param to show()'s signature — wave-0b (registries-schema-hooks) already lands that plus the pin-toggle output vector; this workstream only supplies poll_popout as the closure body at the call site in app/layout_ctl.rs and reads 0b's actual landed signature first. |
| modify | src/ui/tools.rs | Glyph ws:layout-modes section: Pin, Maximize variants + name()/from_name()/draw_glyph arms (small pin-outline and four-corner-arrows glyphs, painter-drawn per file convention). show() body (the tool strip, not Glyph): reorder STRIP by SelectionKind when layout_mode==dynamic. |
| modify | src/hotkeys.rs | Append `// ---- ws:layout-modes ----` actions! rows: Workspace1..Workspace6 Alt+1..Alt+6 (Simple/Edit/Color/Audio/Text/Deliver), MaximizePane Backtick, TogglePin (unbound), ToggleSource (unbound). Do NOT add ToggleLayoutMode or ShowWelcome rows — command-palette's `// ---- ws:command-palette ----` section already declares those two (per audit fix 2, avoids a duplicate-enum-variant compile error); this file's ACT_HANDLERS still consumes both variants. |
| modify | src/settings.rs | Append `// ---- ws:layout-modes ----` fields: workspace: String (default "Edit"), home_screen: bool (default true). layout_mode/onboarded already present from wave0b — not touched here. |
| modify | src/ui/app/menus.rs | Exclusive-wave-2 edit: call layout_ctl::workspace_strip(self,ui) in the menu bar's right-to-left area; add Dynamic/Granular radio pair at top of view_menu; wire Help > Show welcome again to the existing (command-palette-declared) ShowWelcome action (re-arms onboarding). |
| modify | src/ui/settings_ui/general.rs | Exclusive-wave-2 edit: layout-mode radio row and Home screen on-launch checkbox bound to settings.home_screen, next to the existing General rows. |
| modify | src/ui/app/boot.rs | Exclusive-wave-2 edit: the existing contextmenu::install() call at app.rs:940-946 is already guarded (settings.context_menu && screenshot.is_none() && !cfg!(debug_assertions) && !contextmenu::is_installed()), not unconditional. Remove it from App::new's unconditional path and instead set onboarding=Some(Onboarding::default()) when !settings.onboarded && open.is_none() && screenshot.is_none(); the install() call (with its full existing guard, debug/is_installed checks included) moves into the wizard's Finish handler behind the opt-in checkbox, which defaults to settings.context_menu so a plain Finish reproduces today's behavior bit-for-bit including in debug builds and already-installed cases. |
| modify | src/ui/app/mod.rs | Append one line each to: mod list (mod layout_ctl; mod frame;), FRAME_HOOKS (frame::tick), WINDOW_DRAWERS (onboarding::show, home::show), ACT_HANDLERS (layout_ctl::act), TOOL_TABLES (tools_layout::TOOLS) inside the pre-seeded `// ---- ws:layout-modes ----` marker sections. |

## UI changes

- Menu-bar workspace tab strip (Simple/Edit/Color/Audio/Text/Deliver) with active-workspace highlight
- View menu: Dynamic/Granular radio pair above the pane checkboxes
- Tab context menu + top_bar_right_ui gain a Pin toggle button (Glyph::Pin) beside Hide/Pop-out
- Tab bar paints an accent underline (glow) on panes in Behaviour.highlight, decaying over ~1.2s
- Backtick maximises the pane under the cursor to full-tile, backtick again restores
- First-run: non-blocking onboarding window (4 cards) instead of an unconsented context-menu install
- Central empty-state Area with Open/Import/New/Recent cards when project+library are empty
- Adaptive tool-strip button order changes with the dominant selection kind in Dynamic mode only

## New types and functions

- `pub enum Surfaced { Shown, Pinned, Absent }` — src/ui/layout.rs: Result of a programmatic reveal attempt so callers can glow instead of switch when Pinned
- `pub fn reveal_auto(&mut self, pane: Pane) -> Surfaced` — src/ui/layout.rs: Pin-aware reveal: no-ops (Absent) if the pane isn't in the tree, returns Pinned without switching if a pinned sibling tab is active, else reveal()s and returns Shown
- `pub fn maximize(&mut self, pane: Pane); pub fn unmaximize(&mut self)` — src/ui/layout.rs: Stash/restore the pre-maximise tree JSON (egui_tiles 0.14 has no native maximise) around a single-pane root swap
- `pub const WORKSPACES: &'static [(&'static str, Glyph, fn() -> Layout)]` — src/ui/layout.rs: Simple/Edit/Color/Audio/Text/Deliver registry shared by the tab strip, View menu, Alt+1..6 and layout.workspace
- `pub fn simple_layout() -> Layout; pub fn audio_layout() -> Layout; pub fn text_layout() -> Layout; pub fn deliver_layout() -> Layout` — src/ui/layout.rs: New preset builders; each calls the wave0b stack_unplaced helper so every Pane::ALL member (incl. Pane::Source) stays present
- `pub fn act(app: &mut App, a: Action) -> bool` — src/ui/app/layout_ctl.rs: ACT_HANDLERS entry for Workspace1..6/MaximizePane/TogglePin/ToggleSource plus the pre-existing ToggleLayoutMode/ShowWelcome variants (rows owned by command-palette); each mutates Settings/Layout only, no push_undo_labeled (not project state)
- `pub fn surface(app: &mut App, pane: Pane)` — src/ui/app/layout_ctl.rs: Upgrades the wave0b stub: dynamic mode -> layout.reveal_auto + glow on Pinned; granular mode -> glow only, never switches
- `pub fn workspace_strip(app: &mut App, ui: &mut egui::Ui)` — src/ui/app/layout_ctl.rs: Paints WORKSPACES as a small button row, highlighting settings.workspace; click applies the undo-preserving layout swap used by the existing preset menu
- `pub fn poll_popout(hotkeys: &crate::hotkeys::Hotkeys, ctx: &egui::Context) -> Vec<Action>` — src/ui/app/layout_ctl.rs: Body supplied to wave-0b's pre-placed on_viewport closure param on layout::show, so the popped-out viewport's own ctx gets polled and Space/JKL/etc. work in a torn-off Preview; signature confirmed against 0b's landed closure type before writing
- `pub fn tick(app: &mut App, ctx: &egui::Context)` — src/ui/app/frame.rs: FRAME_HOOKS entry: computes SelectionKind from the diffed selection signature, calls layout_ctl::surface with the mapped pane, fires fire_hook("selection_changed", payload) once per distinct value, decays app.layout_glow via animate_until, arms onboarding once post-boot
- `pub struct Onboarding { step: u8, mode: String, install_context_menu: bool }; pub fn show(ctx, st: &mut Onboarding, settings: &mut Settings, layout: &mut Layout) -> Option<()>` — src/ui/onboarding.rs: 4-card wizard; Some(()) on Finish; Finish also runs contextmenu::install() under its existing guard (debug/is_installed) when install_context_menu is checked
- `pub enum HomeAction { Open, Import, New(Option<usize>), OpenRecent(String), Dismiss }; pub fn show(ctx, settings: &Settings) -> Option<HomeAction>` — src/ui/home.rs: Central empty-state / home overlay

## Actions and hotkeys

| Action id | Label | Chord | Note |
|---|---|---|---|
| toggle_layout_mode | Layout Mode: Dynamic / Granular | Ctrl+Shift+G | Row DECLARED by command-palette's ws:command-palette actions! section, not here (audit fix 2); this workstream's ACT_HANDLERS consumes the existing Action variant |
| workspace_1 | Workspace 1 (Simple) | Alt+1 | Alt+1..6 free (Ctrl+Num row is pane toggles); row declared here |
| workspace_2 | Workspace 2 (Edit) | Alt+2 | = default_layout |
| workspace_3 | Workspace 3 (Color) | Alt+3 | = colorist_layout |
| workspace_4 | Workspace 4 (Audio) | Alt+4 |  |
| workspace_5 | Workspace 5 (Text) | Alt+5 |  |
| workspace_6 | Workspace 6 (Deliver) | Alt+6 |  |
| maximize_pane | Maximise Pane under Cursor | ` | Key::Backtick free (Premiere `); row declared here |
| toggle_pin | Pin / Unpin Pane under Cursor |  | tab-bar pin glyph / context menu only; row declared here |
| toggle_source | Show / Hide Source Monitor |  | Ctrl+Num row is full; View menu / palette; generic Pane::Source visibility plumbing only, content owned by source-monitor; row declared here |
| show_welcome | Show Welcome Again |  | Row DECLARED by command-palette's ws:command-palette actions! section, not here (audit fix 2); Help menu wiring here re-arms onboarding by setting settings.onboarded=false via the existing Action variant |

## New glyphs

- Pin
- Maximize

## Persisted fields

**Settings:**

- workspace: String (default "Edit")
- home_screen: bool (default true)

**Project (.sedit):**

- (none)

## MCP tools (required — every capability must be scriptable)

| Tool | Kind | Args | Description | Maps to |
|---|---|---|---|---|
| layout.mode | ui | mode:string:true:'dynamic'\|'granular' | Set the layout mode | Settings.layout_mode + layout_ctl::surface re-evaluation |
| layout.workspace | ui | name:string:true:one of WORKSPACES names | Switch to a named workspace | layout_ctl::workspace_strip's switch path (WORKSPACES lookup + undo-preserving swap) |
| layout.pin | ui | pane:string:true:Pane name; on:bool:true:pin state | Pin or unpin a pane against auto-surfacing | Layout.pinned toggle |
| layout.surface | ui | pane:string:true:Pane name to reveal now | Force-reveal a pane immediately | Layout::reveal_auto |
| layout.maximize | ui | pane:string:false:Pane name; omit to unmaximize | Maximise or restore a pane | Layout::maximize/unmaximize |
| layout.list | read |  | Current mode, workspace, pinned panes, maximized pane | Settings.layout_mode/workspace + Layout.pinned/maximized state |
| onboarding.reset | ui |  | Re-arm the first-run wizard | Settings.onboarded = false (+ opens the wizard now) |

**Luau:** layout.*/onboarding.reset are ordinary ToolDef rows: callable via editor.tool(name, args) exactly like any other tool, listed in editor.tools() with arg docs. ui.action(id) fires Workspace1..6/MaximizePane/TogglePin/ToggleSource by id here, plus the pre-existing ToggleLayoutMode/ShowWelcome ids (rows declared by command-palette). This workstream owns and fires the `-- @on selection_changed` event from frame.rs::tick (per audit fix 1), reusing wave0b's fire_hook budget/re-entrancy guard; the other four promised events (import, export_done, project_open, project_save, marker_added) are owned by media-library, export-deliver, forgiveness (x2), and trim-model/audio-analysis respectively — not this plan.

## Tests

| Test | File | Asserts |
|---|---|---|
| every_workspace_contains_every_pane | src/ui/layout.rs | simple_layout/audio_layout/text_layout/deliver_layout (plus existing default/colorist/fastcut) each have every Pane::ALL member present and is_visible via stack_unplaced |
| reveal_auto_respects_pinned_sibling | src/ui/layout.rs | pin Inspector as the active tab of a group; reveal_auto(Mixer) in the same group returns Pinned and leaves Inspector active; reveal_auto on a pane absent from the tree returns Absent and inserts nothing |
| maximize_round_trips_tree | src/ui/layout.rs | maximize(Effects) then unmaximize() restores the pre-maximise tree JSON byte-for-byte |
| pinned_survives_json_roundtrip | src/ui/layout.rs | Layout with pinned=[Inspector] serializes and Self::from_json round-trips the pinned vec |
| selection_kind_maps_to_expected_pane | src/ui/app/layout_ctl.rs | table-driven: Text->Inspector, Audio->Mixer, Transition->Transitions, Cue->Subtitles, None/Mixed->no surface call |
| auto_surface_only_in_dynamic_mode | src/ui/app/frame.rs | same selection change with layout_mode=granular pushes a glow entry and does not call reveal_auto |
| glow_decays_and_stops_repainting | src/ui/app/frame.rs | assert_no_idle_repaint harness: after a glow entry's age exceeds its duration, ctx.has_requested_repaint() is false with no more animate_until scheduled |
| selection_changed_hook_fires_once_per_change | src/ui/app/frame.rs | changing selection fires fire_hook("selection_changed", _) exactly once; an unchanged selection across two frames fires it zero times; respects the shared 250ms budget/disable-on-overrun from wave0b |
| popout_hotkeys_reach_pending_actions_once | src/ui/app/layout_ctl.rs | a synthetic keydown fed to poll_popout(ctx) on a popped viewport's ctx yields exactly one Action, and the same frame's root hotkeys.poll(ctx) does not also emit it |
| onboarding_skipped_for_screenshot_and_open | src/ui/app/boot.rs | App::new with screenshot=Some(_) or open=Some(_) never sets onboarding=Some(_) |
| onboarding_finish_applies_mode_and_installs_under_existing_guard | src/ui/onboarding.rs | picking Simple+Dynamic then Finish sets settings.onboarded=true, settings.layout_mode="dynamic", layout swapped to simple_layout(); with install_context_menu checked, contextmenu::install() is called only when !cfg!(debug_assertions) && !is_installed(), matching app.rs's pre-existing guard verbatim |
| home_hidden_when_disabled_or_project_nonempty | src/ui/home.rs | home::show returns None when Settings.home_screen=false or when project/library is non-empty |
| layout_tools_names_unique_and_args_parse | src/ui/app/tools_layout.rs | structural parity: TOOLS names are unique across the crate-wide all_tools(), every arg spec parses (schema_builder), and layout.list's output round-trips into layout.mode/workspace/pin inputs |
| no_duplicate_hotkey_rows_for_shared_actions | src/hotkeys.rs | the actions! table contains exactly one ToggleLayoutMode variant and exactly one ShowWelcome variant crate-wide (guards against the duplicate-declaration defect fixed by audit fix 2); no_duplicate_defaults stays green |
| every_glyph_paints_a_picture | src/ui/tools.rs | pre-existing test stays green after Pin/Maximize are added to Glyph::ALL/name/from_name/draw_glyph (no new test file, verify only) |

## Verification checklist

- [ ] cargo test (full suite incl. the 14 new tests above) green
- [ ] cargo run -- --selftest: idle-repaint step stays green after opening+closing onboarding and the home screen once, and after a glow entry fully decays
- [ ] screenshot SE_LAYOUT=simple: Library/Effects/Transitions/Subtitles/Gallery tab-stack left, Preview centre, Inspector right, Timeline bottom
- [ ] screenshot with SE_FIRST_RUN=1 (fresh settings dir): onboarding wizard visible, no home screen behind it yet
- [ ] screenshot: workspace strip with an active highlight, a pinned tab (pin glyph lit), and a glowing unpinned tab side by side
- [ ] manual: select an audio clip in Dynamic mode -> Mixer surfaces; pin Inspector -> selecting a text clip glows Inspector's tab instead of switching to it
- [ ] manual: pop out Preview, click it, press Space -> plays; press J/K/L -> shuttles
- [ ] manual: Alt+1..6 switches workspaces; backtick maximises the hovered pane, backtick again restores; Ctrl+Shift+G (declared by command-palette, consumed here) toggles Dynamic/Granular
- [ ] manual: debug build (cargo run, no --release) and an already-installed context menu both see no install() call even with the wizard checkbox checked, matching today's guard
- [ ] MCP: layout.workspace {"name":"Color"} switches layout; layout.pin {"pane":"Inspector","on":true} then selecting audio does not move Inspector's tab; onboarding.reset then relaunch shows the wizard again
- [ ] MCP/Luau: a script with `-- @on selection_changed` fires exactly once per selection change, editor.event carries the payload
- [ ] cargo build after rebasing onto command-palette's branch: no duplicate-enum-variant compile error on ToggleLayoutMode/ShowWelcome (audit fix 2 regression check)
- [ ] scripts/size.ps1 -Note layout-modes-onboarding: delta ≤ 100 KB or a named offset in the PR body

## Acceptance criteria

- [ ] default/colorist/fastcut/simple/audio/text/deliver layouts all pass every_workspace_contains_every_pane
- [ ] Pinning a pane's active tab blocks auto-surface from switching that tab group away; unpinned Dynamic-mode selections still auto-surface
- [ ] Granular mode never auto-switches tabs, only glows
- [ ] Alt+1..6, backtick and Ctrl+Shift+G (consumed, not declared, here) work as specified with no hotkey conflicts (no_duplicate_defaults green, and exactly one ToggleLayoutMode/ShowWelcome variant exists crate-wide per audit fix 2)
- [ ] Fresh install (no settings.json, no --screenshot, no file arg) shows the onboarding wizard exactly once; Finish persists onboarded=true, applies the chosen mode/layout, and installs the context-menu entry only under the same guard app.rs already applies today (debug/is_installed checks preserved)
- [ ] A popped-out Preview responds to Space/J/K/L while focused, using the on_viewport hook wave-0b pre-placed rather than a new parameter added here
- [ ] --selftest idle-repaint step stays green with onboarding/home closed and glow decayed
- [ ] All 7 layout.*/onboarding.reset tools appear in tools/list and editor.tools() with parsed arg docs; ui.action covers every new Action id plus the two consumed ones
- [ ] frame.rs's SelectionKind diff uses snap-engine's actual landed edit-point field name/type, confirmed by reading that wave-1 code before wiring, not assumed
- [ ] frame.rs fires `-- @on selection_changed` exactly once per distinct selection change, verified by selection_changed_hook_fires_once_per_change (closes audit fix 1's coverage gap for this event)
- [ ] cargo build succeeds after rebasing onto command-palette's merged branch with zero duplicate-enum-variant errors (closes audit fix 2)
- [ ] Full cargo test suite green; scripts/size.ps1 delta ≤105 KB or a named offset

## Risks

| Risk | Mitigation |
|---|---|
| SimplificationOptions can prune a container emptied by hiding panes, breaking every_workspace_contains_every_pane on a new builder | every new builder ends with the wave0b stack_unplaced helper (all_panes_must_have_tabs=true is already set); test pins is_visible for every Pane::ALL member |
| reveal_auto could insert a stray root tab for a pane the tree doesn't know about (old profile), like the pre-existing insert_into_root footgun | reveal_auto returns Absent and never calls insert_into_root when find_pane is None; only explicit user actions still use reveal()/toggle() |
| Popped-out viewport hotkeys double-fire an Action if on_viewport's poll and the root ctx's poll both see the same key event | each egui immediate viewport has isolated input state, so poll_popout(ctx) only sees that viewport's events; popout_hotkeys_reach_pending_actions_once pins exactly-one-Action per frame per viewport |
| Glow list left non-empty (e.g. a leaked entry) would keep animate_until scheduling repaints and regress the idle-CPU gate | bound glow duration to ~1.2s, well under the selftest's idle window; decay runs unconditionally in frame::tick so a stuck entry is caught by glow_decays_and_stops_repainting |
| Wave-0b's actual on_viewport closure signature (param types, return value) may differ from what poll_popout assumes, since 0b lands it, not this workstream | before writing layout_ctl::poll_popout, read the landed src/ui/layout.rs show() signature from registries-schema-hooks and match it exactly rather than guessing; flagged explicitly in scope_out and the layout.rs file entry |
| frame.rs's SelectionKind diff assumes a `timeline.edit_point` field whose name/type/location is decided by snap-engine (wave 1), a dependency this plan previously omitted | snap-engine added to depends_on; before wiring the diff, confirm the actual field snap-engine lands on TimelineState rather than assuming the name sight-unseen |
| Rewriting boot.rs's contextmenu::install() call site as if it were unconditional could regress debug-build or already-installed behavior | boot.rs and onboarding.rs entries now explicitly preserve app.rs:940-946's existing guard (screenshot.is_none() && !cfg!(debug_assertions) && !is_installed()) inside the wizard's Finish handler, adding only the consent checkbox on top; verified against source before writing this plan |
| layout::WORKSPACES's name/shape must match what command-palette (wave1, owns palette.rs) already wired for its Command::Workspace(String) row — I don't own that file | grep src/ui/palette.rs for Command::Workspace / any layout::WORKSPACES reference before finalizing the constant's signature; adjust mine to match rather than touch palette.rs |
| Ctrl+Shift+G / Alt+1..6 / backtick could collide with an OS or another action added by a concurrent wave-1 branch merged just before this one starts | no_duplicate_defaults runs in CI on rebase; these are the chords the skeleton's keymap already decided free, but re-run the test before opening the PR |
| Merging after command-palette (which already declares ToggleLayoutMode/ShowWelcome rows) without removing this plan's own former duplicate rows would be a hard compile error, not just a style issue | audit fix 2 applied: this plan's hotkeys.rs diff now declares only Workspace1..6/MaximizePane/TogglePin/ToggleSource; a dedicated no_duplicate_hotkey_rows_for_shared_actions test and a rebase-time build check guard the regression |
| The five promised Luau @on events (selection_changed + 4 others) were previously undercommitted across all plans, risking a shipped mcp_parity narrative with only 1 of 6 events actually wired | audit fix 1 applied: this plan now explicitly owns and implements selection_changed in frame.rs::tick with its own test; the other four are named to their respective owning workstreams in scope_out so docs-refresh can verify full coverage before wave 3 closes |

## Suggested implementation order

1. 0. Read wave-0b's landed src/ui/layout.rs show() signature (on_viewport param + pin-toggle output), snap-engine's landed edit-point field on TimelineState, and command-palette's landed hotkeys.rs ToggleLayoutMode/ShowWelcome rows + fire_hook stub signature before writing anything that depends on them
2. 1. src/ui/layout.rs: Surfaced, reveal_auto, maximize/unmaximize, WORKSPACES + 4 new builders, Behaviour pin/highlight (no changes to show()'s parameter list — that's 0b's); run existing + new layout.rs tests, screenshot the 3 pre-existing layouts unchanged
3. 2. src/ui/tools.rs: Glyph::Pin/Maximize + paint wiring in layout.rs's tab_ui/top_bar_right_ui
4. 3. src/hotkeys.rs: ws:layout-modes actions! rows for Workspace1..6/MaximizePane/TogglePin/ToggleSource ONLY (no ToggleLayoutMode/ShowWelcome — audit fix 2); run no_duplicate_defaults and no_duplicate_hotkey_rows_for_shared_actions
5. 4. src/settings.rs: workspace/home_screen fields in the ws:layout-modes marker section
6. 5. src/ui/app/layout_ctl.rs: act() (new rows + consuming the two existing variants), surface() upgrade, workspace_strip, poll_popout matching 0b's confirmed on_viewport signature; wire ACT_HANDLERS + the on_viewport call site where layout::show is invoked
7. 6. src/ui/app/frame.rs: tick() selection-diff using snap-engine's confirmed edit-point field + fire_hook("selection_changed",...) call (audit fix 1) + glow decay + onboarding arm; wire FRAME_HOOKS; run selection_changed_hook_fires_once_per_change
8. 7. src/ui/onboarding.rs + src/ui/home.rs (Finish folds in app.rs:940-946's exact existing guard); wire WINDOW_DRAWERS; src/ui/app/boot.rs first-run trigger swap removing the unconditional-on-consent call, preserving its debug/is_installed guard inside the wizard
9. 8. src/ui/app/tools_layout.rs: TOOLS const; wire TOOL_TABLES; run structural parity tests
10. 9. src/ui/tools.rs show() body: adaptive strip reorder
11. 10. src/ui/app/menus.rs + src/ui/settings_ui/general.rs: workspace strip call site, View menu radio, General tab rows
12. 11. Rebase onto command-palette's merged branch, confirm zero duplicate-variant compile errors; full cargo test + --selftest + screenshots; scripts/size.ps1; open PR

## Deliberate simplifications (`// ponytail:`)

- Maximize = tree-JSON stash/restore reusing the existing profile-switch trick, not a new egui_tiles primitive — egui_tiles 0.14 has no native maximise; revisit if it grows one.
- Adaptive tool strip is a single reorder of the existing STRIP const by SelectionKind, not a customizable/user-reorderable toolbar — add per-user ordering only if requested.
- Home screen cards are text+path only, no ThumbCache read — add project thumbnails when a project manifest stores one.
- Workspace switch reuses the existing undo-preserving preset-swap code verbatim instead of introducing a new LAYOUT_STEP type.
- Onboarding is a fixed 4-step linear wizard, no skip-tracking/telemetry — add only if requested.
- Onboarding's context-menu opt-in reuses app.rs's exact existing install guard rather than inventing a new consent-gating mechanism — one guard, one call site, moved not duplicated.
- selection_changed's payload is the same SelectionKind diff already computed for surface()/glow — no second selection-tracking structure, one fire_hook call reusing the existing diff result.

## Review trail

- Finding 1 (ownership, on_viewport): corrected every reference claiming this workstream adds the `on_viewport: &mut dyn FnMut(&egui::Context)` parameter to layout::show. That parameter is pre-placed by wave-0b (registries-schema-hooks), which the skeleton's 'Pop-out hotkeys' decision and owns_files list both confirm. Reworded the layout.rs file entry, the show() new_types_and_fns row (removed — it belonged to 0b), poll_popout's purpose, scope_in/scope_out, a new risk, and acceptance/implementation-order steps to say this workstream only wires poll_popout as the closure body into 0b's already-landed hook, and to read that landed signature first rather than asserting it.
- Finding 2 (ownership, edit_point dependency): frame.rs's SelectionKind diff reads timeline.edit_point, a field snap-engine (wave 1) introduces via its 'Edit-point seam selection' feature on the exclusive-wave-1-owned src/ui/timeline/mod.rs — verified current src/ui/timeline.rs has no such field today, so its name/shape was being assumed sight-unseen. Added snap-engine to depends_on, added a matching risk/mitigation, and qualified the frame.rs file entry and implementation_order step 0/6 to confirm the actual field before wiring instead of hardcoding the guessed name.
- Finding 3 (other, contextmenu::install mischaracterization): verified app.rs:940-946 — the existing install() call is already guarded on settings.context_menu && screenshot.is_none() && !cfg!(debug_assertions) && !contextmenu::is_installed(), not unconditional as originally described. Corrected the boot.rs file entry, onboarding.rs file entry/new_types_and_fns, a ui_changes line, a test (renamed and re-scoped to assert the guard is preserved, not just that a new gate around wizard-visibility exists), a verification step, an acceptance criterion, and a ponytail_note so the wizard's Finish handler reproduces the full existing guard (debug/is_installed checks included) behind the new consent checkbox, instead of only adding an onboarded/open gate around wizard display and dropping the debug/is_installed protections.
- AUDIT FIX (major, fire_hook coverage): per the new audit's cross-plan fire_hook gap, this workstream now explicitly owns and implements the `selection_changed` @on Luau event (the only one of the six promised events this plan can naturally host, since frame.rs's per-frame selection diff already exists here). Added: a fire_hook call in frame.rs::tick's new_types_and_fns/files entry, a scope_in line, a new test (selection_changed_hook_fires_once_per_change), an acceptance criterion, a verification step, a risk/mitigation naming the other four events' owners (media-library/import, export-deliver/export_done, forgiveness/project_open+project_save, trim-model or audio-analysis/marker_added — matching the audit's fix), a scope_out line disclaiming those four, a ponytail_note, and a luau-field update. No other workstream's files were touched; ownership of the remaining four events is left to the audit's assignment and documented here only as a cross-reference.
- AUDIT FIX (blocker, duplicate hotkey rows): removed this plan's ToggleLayoutMode (Ctrl+Shift+G) and ShowWelcome hotkeys.rs actions! row declarations, since command-palette's plan already declares both under its own ws:command-palette marker section — two enum variants of the same name in one macro-generated enum is a compile error. This workstream's hotkeys.rs file entry, actions_and_hotkeys table (notes updated to say 'declared by command-palette, consumed here'), layout_ctl::act's new_types_and_fns purpose, a ui_changes/verification wording tweak, a new risk/mitigation, a new test (no_duplicate_hotkey_rows_for_shared_actions), an acceptance criterion, and implementation_order steps 3/11 were all updated so this plan declares hotkey rows only for Workspace1..6/MaximizePane/TogglePin/ToggleSource and merely consumes the two shared Action variants in ACT_HANDLERS. Size/line-count estimates nudged +5KB/+15 lines for the added test and guard comments.
- Everything else (WORKSPACES registry mechanics, reveal_auto/pin/glow, home screen, remaining settings fields, MCP tools, tests not touched by any finding, unrelated ponytail notes) preserved unchanged from the prior revision — no finding touched them.
