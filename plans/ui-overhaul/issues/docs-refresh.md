# docs-refresh: finalize ARCHITECTURE.md, goals.md, CHANGELOG.md, agents.md, se-* commands, docs/customizing.md

**Workstream:** `docs-refresh` · **Issue:** [#38](https://github.com/KashTheKing/simple-editor/issues/38) · **Wave:** 4 · **Branch/worktree:** `chore/docs-refresh` → `../simple-editor-wt/docs-refresh` · **Depends on:** pro-timeline, pro-monitor, text-titles · **~550 new lines · Δ exe ≈ +0 KB**

Read [agents.md](../../../agents.md), [goals.md](../../../goals.md), [notes.md](../../../notes.md) and the master plan [README.md](../README.md) (registry protocol, keymap, modifier table) before starting.

## Summary

Docs-only, serial wave-4 closer (moved out of wave 3's concurrent set since it depends on three wave-3 branches): rewrite ARCHITECTURE.md's module map and add Registries/Gesture-model/Keymap/Alt-render/New-pane-checklist/Size-gate sections; refresh goals.md's Achieved list (grouped bullets covering every merged workstream) and binary-size baseline from size_log.csv; consolidate CHANGELOG.md and notes.md; extend agents.md and the three se-* slash commands with the marker-section/size-gate/parity-test/wave-ordering protocol; write docs/customizing.md (scripts, MCP tools, keymaps, layout) with its MCP-tool table transcribed by hand from the raw curl tools/list JSON response, and its @on event list built from a grep of real fire_hook call sites rather than restating mcp_parity's unverified six-event promise (only export_done was confirmed committed at planning time). No Rust source touched; verification is a final full-suite no-regression pass.

## Motivation

Closes critique hooks 8/12 (self-documenting tool catalogue, new-pane checklist) and goals.md's 'Deep user customizability' core goal. Last-landed workstream by design - now placed in its own serial wave 4 since it depends on three wave-3 branches (pro-timeline, pro-monitor, text-titles) and cannot run concurrently with them. Documents what actually shipped: the size number, module list, tool table, and @on event ownership are transcribed from the merged tree/size_log.csv/live tools-list/fire_hook grep, never re-typed from the skeleton's unverified promises.

## In scope

- Doc rewrites: ARCHITECTURE.md, goals.md, CHANGELOG.md, agents.md, notes.md
- Slash-command updates: .claude/commands/se-plan.md, se-implement.md, se-review.md
- New user-facing doc: docs/customizing.md
- New reference script: scripts/gen_docs.luau
- Final full verification run (cargo test, --selftest, size gate) as a no-regression check
- Grepping the merged tree for real fire_hook call sites to document actual (not promised) @on event ownership

## Out of scope

- Any Rust source change (no owns_files under src/)
- Any new MCP tool, Action, Pane, Glyph, or settings/project field
- Automated doc-drift CI tests (see ponytail_notes)
- Editing src/selftest.rs (touches_shared is 'none' for this workstream - size-diet already owns the idle-selftest step)
- A `docs.tools_md` MCP tool or any other automation to keep docs/customizing.md's tool table in sync (documented upgrade path only)
- Adding missing fire_hook call sites to other workstreams' code - this workstream only documents what landed, it does not implement the gap

## Files

| Op | Path | What |
|---|---|---|
| modify | ARCHITECTURE.md | Rewrite module map (lines ~61-105 today) for the split app/model/timeline trees; add sections: Registries, Gesture model (frozen modifier table, ~25 rows), Keymap (~80-row summary), Alt-render channel, New-pane checklist, Size & idle-CPU gates; update measured-baseline line (~9-13) from size_log.csv's final row and the final test count; extend Feature inventory with wave 1-3 capabilities; add OP_TOOLS/ToolKind note to Conventions. ~250-300 new lines given six new sections plus a rewritten module map. |
| modify | goals.md | Update binary-size line under Core goals from size_log.csv's final byte count; add ~14 compressed Achieved bullets grouped by workstream area (a bullet may name 2-3 related workstreams so every merged workstream is traceable to a bullet); update In-progress list (keep the size goal open if still >10MB, note the -Os gate outcome and the se-engine follow-up as the documented path if it wasn't taken). |
| modify | CHANGELOG.md | Add one new version section (e.g. `## beta-0.3.0`) folding the current `## unreleased` PR-12/14 entries plus one terse paragraph per wave-1/2/3 workstream area (command palette, forgiveness, trim/gestures, JKL+loop, audio analysis/dsp, color engine, layout modes, canvas/monitor, library, export, inspector/gallery, transcript/captions, source monitor, pro-timeline, pro-monitor, titles) - match the existing bullet density, no per-PR essay. |
| modify | agents.md | Add a `### Concurrent worktree protocol` section after "Multi-agent work": wave shape (0 serial, 1-3 concurrent <=8 worktrees, docs-refresh as a serial wave-4 closer since it depends on wave-3 branches), marker-section rule (edit only your `// ---- ws:<name> ----` line in TOOL_TABLES/ACT_HANDLERS/FRAME_HOOKS/WINDOW_DRAWERS/PANE_DRAWERS, actions!, Glyph, Settings/Project/App), `.gitattributes merge=union` scope (size_log.csv + CHANGELOG.md ONLY, never .rs files). Add a `### Size gate` subsection under Verification: `scripts/size.ps1 [-Note]`, size_log.csv format, /se-verify's +64KB threshold and +300KB named-offset rule. Add to Verification list: every new Action/Gesture/Pane/Project mutator needs a ToolDef row (`every_edit_op_has_a_tool`/OP_TOOLS test enforces it), every new pane/overlay/window needs an `assert_no_idle_repaint` test, and every new `-- @on` hook event needs exactly one documented `fire_hook` call site owned by the workstream that introduces the triggering action. |
| modify | notes.md | Append one consolidated note (newest-at-top, ~500 chars) on cross-cutting gotchas surfaced across the overhaul: Luau's io/os/ffi sandbox means `editor.log()` only ever renders as a 5-10s auto-expiring toast (app.rs:1092, ~6365) with no copy button - script-generated docs must be transcribed via `curl` on tools/list, not by reading toast output; alt-render requests must check `export.is_some()` before firing to avoid GPU contention; registry marker-section hunks never got a real union conflict because Rust sources were excluded from .gitattributes union; the six planned `-- @on` hook events had only one committed fire_hook call site (export_done) at the wave-1/2 planning stage - verify the other five (selection_changed, import, project_open, project_save, marker_added) actually landed before documenting them as shipped. |
| modify | .claude/commands/se-plan.md | Add a step: if the plan adds an Action, Gesture, Pane, a `pub fn ...(&mut self` on Project, or a new `-- @on` hook event, the plan must include a matching ToolDef row (or fire_hook call site) and name which TOOL_TABLES/registry section it lands in. A plan that depends on same-wave workstreams must be placed in the next wave, not left in theirs. |
| modify | .claude/commands/se-implement.md | Add to worktree step: when working inside a named workstream's marker section (TOOL_TABLES etc.), edit ONLY that section's line; add to Verify step: run `scripts/size.ps1 -Note <name>` when the change adds code, add an `assert_no_idle_repaint` test for any new pane/overlay/window, and confirm any new `-- @on` event has a real fire_hook call site before claiming it shipped. |
| modify | .claude/commands/se-review.md | Add point 5: MCP/tool parity - flag a new Action/Gesture/Pane/Project mutator with no matching ToolDef row, a new pane/overlay/window with no idle-repaint test, and a documented `-- @on` event with no fire_hook call site anywhere in the diff. Add point 6: size gate - flag a diff that grows compiled code without a `size: +-N KB` note. Add point 7: wave ordering - flag a plan whose depends_on includes a same-wave workstream (it must move to the next wave). |
| create | docs/customizing.md | User-facing doc: Luau script folder + sandbox (no io/os/ffi, 5s run budget, 250ms `-- @on` hook budget) + header metadata (`@name/@desc/@icon/@hotkey/@on <event>/@budget_ms`) + event list with the actual owning call site per event as landed (verify each of selection_changed/import/export_done/project_open/project_save/marker_added against the merged tree - do not restate the skeleton's unverified six-event promise); MCP tool catalogue table (grouped by namespace, kind Read/Mutate/Job/Ui, undo semantics for Mutate) - populated once by hand from the raw `curl` tools/list JSON response, not from a script's toast output; keymap presets (default/Avid/Premiere/Resolve) + command palette + cheat sheet; layout: Simple vs Dynamic vs Granular, workspaces Alt+1..6, pin/lock, pop-outs, `.sedit-layout`/`.sedit-theme` sharing; pointer to ARCHITECTURE.md's Gesture model table for the modifier chords; one paragraph documenting the undocumented/unbuilt `lite` feature-set escape hatch. Table alone is ~100-150 lines at current+projected tool count. |
| create | scripts/gen_docs.luau | Reference script (copy into %APPDATA%\SimpleEditor\scripts, run from the Scripts menu): calls `editor.tools()`, formats a markdown table (name/kind/args/desc), emits it via `editor.log()`. Purpose is narrowed to a live sanity-check that editor.tools() enumerates the same tool set curl's tools/list returned (spot-check a few names/count in the toast before it expires) - NOT a transcription source, since editor.log is a 5-10s auto-expiring toast with no copy affordance (app.rs:1092, ~6365; scripting.rs:38). |

## UI changes

- none (docs-only workstream; no UI code touched)

## New types and functions

- `-- gen_docs.luau: local tools = editor.tools(); editor.log(markdown_table(tools))` — scripts/gen_docs.luau: Spot-check dump of editor.tools() as a toast, to eyeball against curl's tools/list count/names; no persistent state, no new bridge fn, not a transcription path.

## MCP tools (required — every capability must be scriptable)

| Tool | Kind | Args | Description | Maps to |
|---|---|---|---|---|

**Luau:** No new Luau engine surface. scripts/gen_docs.luau is a plain user script using the existing editor.tools()/editor.log() bridge (src/scripting.rs) - a live spot-check that Luau's view of the catalogue matches curl's, not a docs transcription source (editor.log is a 5-10s auto-expiring toast, app.rs:1092/~6365, no copy button).

## Tests

| Test | File | Asserts |
|---|---|---|
| (none new - docs-only workstream) | n/a | This workstream adds no Rust code, so it adds no tests. It instead re-runs the full suite as an acceptance gate: `cargo test` (all existing tests including every structural-parity test - every_edit_op_has_a_tool/OP_TOOLS, ui_action_covers_every_action, mutate_rows_roll_back_on_error, gestures_have_tool_twins, every assert_no_idle_repaint instance, every_glyph_paints_a_picture, reserved_chords_are_free), `cargo run -- --selftest`, and `scripts/size.ps1` all pass/run clean before any doc is written and again before declaring done. |

## Verification checklist

- [ ] cargo test - full suite green, including every structural-parity/idle-repaint test added by earlier waves
- [ ] cargo run -- --selftest - clean
- [ ] scripts/size.ps1 - run once to confirm the final byte count before writing it into goals.md/ARCHITECTURE.md
- [ ] grep ARCHITECTURE.md's module-map fenced block against `ls -R src/ui/app src/model src/ui/timeline` - no missing or stale entries
- [ ] Toggle MCP on, `curl` tools/list, transcribe docs/customizing.md's table directly from that JSON response (names + kind + args), then spot-check the count/a few names against it - must match exactly
- [ ] Grep the merged tree for `fire_hook(` call sites and write docs/customizing.md's @on event list from what's actually there, not from the six-event promise in mcp_parity's narrative - note any event that never got a call site
- [ ] Run scripts/gen_docs.luau from the in-app Scripts menu once, purely as a live spot-check that editor.tools()'s visible names/count agree with the curl response before the toast expires
- [ ] Read every other workstream's actual owns_files diff (not just the skeleton) before writing CHANGELOG.md bullets, so shipped behavior is described, not planned behavior
- [ ] Confirm every merged wave 0-3 workstream name is traceable to at least one goals.md Achieved bullet, even where bullets are grouped
- [ ] Confirm this workstream's wave placement (4) is not duplicated inside wave 3's concurrent list anywhere in the broader plan

## Acceptance criteria

- [ ] ARCHITECTURE.md module map lists every file under src/ui/app/*, src/model/*, src/ui/timeline/* that exists in the tree at merge time (grep ls src/ui/app src/model src/ui/timeline against the fenced block: zero missing, zero stale entries)
- [ ] ARCHITECTURE.md has Registries, Gesture model, Keymap, Alt-render channel, New-pane checklist, Size & idle gates sections; each cites real file:line or the const/fn name it documents
- [ ] goals.md binary-size line matches size_log.csv's last row exactly (byte count + MB); goals.md's Achieved list represents every merged wave 0-3 workstream (grouped bullets allowed - every workstream name/feature must be traceable to at least one bullet)
- [ ] CHANGELOG.md has one section covering all wave 0-3 workstreams, terse (no per-PR prose duplication of goals.md)
- [ ] agents.md documents the marker-section rule, the size gate command, assert_no_idle_repaint and OP_TOOLS parity requirements so a fresh agent needs no other source to follow the protocol
- [ ] docs/customizing.md's MCP tool table matches a direct curl of the live tools/list JSON-RPC call (tool count and names), spot-checked by hand - not compared against gen_docs.luau's toast output
- [ ] docs/customizing.md's @on event list names the correct owning workstream/call site for each of the six events (selection_changed, import, export_done, project_open, project_save, marker_added) as actually landed in the merged tree, not the unowned six-event promise from mcp_parity's original narrative
- [ ] se-plan/se-implement/se-review all reference the registry/marker-section protocol and the size gate
- [ ] cargo test, cargo run -- --selftest, and scripts/size.ps1 all pass/run clean at time of this PR (this workstream changes no Rust code, so this is a no-regression check, not new coverage)
- [ ] This workstream's own wave placement (4, serial-after wave 3) is consistent with it depending on three wave-3 workstreams - it must not be listed in wave 3's concurrent set

## Risks

| Risk | Mitigation |
|---|---|
| Written before wave 1/2/3 merge (out of order) - docs describe planned files/registries that don't exist yet, drifting immediately. | depends_on pins this to run only after pro-timeline, pro-monitor, text-titles (the last wave-3 branches) land, and it now occupies its own serial wave 4 rather than being falsely concurrent with them; step 1 of implementation_order is a green cargo test/--selftest gate before writing anything. |
| goals.md's binary-size line gets hand-guessed instead of read from size_log.csv, immediately contradicting the actual shipped exe. | implementation_order step 2 makes reading size_log.csv's last row mandatory before touching goals.md or ARCHITECTURE.md. |
| docs/customizing.md's MCP tool table drifts the moment the next feature PR adds a tool, since it's hand-transcribed. | documented as a known ceiling in ponytail_notes with the `docs.tools_md` upgrade path; not worth automating for a doc that changes a few times a year. |
| Verification step wrongly assumed editor.log's toast output could serve as a byte-for-byte comparison surface for an 80-150 row table, which would have made an earlier acceptance criterion unpassable (toasts auto-expire in 5-10s with no copy button). | Verification and acceptance criteria require the tool table be transcribed directly from curl's tools/list JSON; gen_docs.luau is downgraded to a live spot-check of a few names/count, not a transcription or diff step. |
| docs/customizing.md documents all six @on hook events as shipped with named owners, but only export_done had a committed fire_hook call site across the reviewed wave 1/2 plans - the other five (selection_changed, import, project_open, project_save, marker_added) had no matching diff in any workstream's files[]. | implementation_order step 5 requires grepping the merged tree's real fire_hook call sites before writing the event list; docs/customizing.md and ARCHITECTURE.md report only events that actually have a call site, with any gap noted explicitly rather than silently restating the unowned promise. Also flagged for se-review (point 7 addition) so a future PR that adds an @on event without a call site is caught before merge. |
| docs-refresh was listed in the same wave-3 concurrent set as the three branches it depends_on, which is self-contradictory (it can't start until they finish, so it isn't actually concurrent with them). | Moved this workstream's `wave` field from 3 to 4 (a serial closer). depends_on is unchanged (still the three wave-3 branches, now correctly one wave earlier). ponytail_notes flags the surrounding skeleton's waves[] array as needing the same correction if it still shows docs-refresh under wave 3's concurrent list. |

## Suggested implementation order

1. 1. Run `cargo test && cargo run -- --selftest` to confirm the merged tree (waves 0-3) is green before documenting it.
2. 2. Read `size_log.csv`'s last row for the true final byte count; do not guess or reuse the skeleton's projected 12.2MB.
3. 3. `ls -R src/ui/app src/model src/ui/timeline` and rebuild ARCHITECTURE.md's module map from the real file list, not the skeleton's planned list.
4. 4. Grep every registry const (`TOOL_TABLES`, `ACT_HANDLERS`, `FRAME_HOOKS`, `WINDOW_DRAWERS`, `PANE_DRAWERS`) and `arm(` in src/ui/timeline/arm.rs for the real signature; write ARCHITECTURE.md's Registries and Gesture model sections from what's actually there.
5. 5. Grep every `fire_hook(` call site in the merged tree; record the real owning file:line for each of the six planned @on events (selection_changed, import, export_done, project_open, project_save, marker_added) - do not assume all six landed as originally promised.
6. 6. Write ARCHITECTURE.md's Keymap/Alt-render/New-pane-checklist/Size-gate sections from this issue's skeleton context (those are design-time decisions, not code to re-derive).
7. 7. Update goals.md (size line + grouped Achieved bullets covering every merged workstream + In-progress).
8. 8. Update CHANGELOG.md.
9. 9. Toggle MCP on in Settings, `curl -s -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' -H 'content-type: application/json' http://127.0.0.1:<port>/mcp` and write docs/customizing.md's tool table directly from the JSON response.
10. 10. Write scripts/gen_docs.luau, drop it in the scripts folder, run it once via the Scripts menu as a spot-check only (compare a few visible tool names/count against step 9's curl output before the toast expires) - do not attempt to transcribe the full table from it.
11. 11. Write the rest of docs/customizing.md (scripts/keymap/layout sections), including the @on event list built from step 5's grep, not from the skeleton's unverified promise.
12. 12. Update agents.md (protocol + size gate + verification additions + wave-ordering rule) and the three .claude/commands/se-*.md files.
13. 13. Append the notes.md consolidation entry.
14. 14. Final pass: grep ARCHITECTURE.md's module-map fence against `ls src/**` for drift; re-run cargo test/--selftest/size.ps1 one last time.

## Deliberate simplifications (`// ponytail:`)

- scripts/gen_docs.luau can't write docs/customizing.md directly - Luau is sandboxed (no io/os/ffi) AND editor.log only ever surfaces as a 5-10s auto-expiring toast (app.rs:1092, ~6365) with no copy affordance. So the tool table is transcribed by hand from curl's tools/list JSON, and gen_docs.luau is demoted to a live spot-check. Upgrade path if this doc is regenerated often: a Read-kind MCP tool `docs.tools_md` returning the same markdown as a string. Skipped because this doc changes maybe once a quarter.
- No new Rust test for 'ARCHITECTURE.md matches the filesystem' - it's a one-time grep in the implementation checklist, not a standing CI check.
- docs/gesture-table.md and docs/screens/README.md from the source design were folded into ARCHITECTURE.md/docs/customizing.md instead of new files.
- CHANGELOG.md collapses ~14-22 workstreams into one terse section per the existing bullet-list density rather than one heading per workstream.
- goals.md's Achieved list is explicitly grouped (~14 bullets covering ~22 workstreams), not 1:1 - the acceptance criterion only requires every workstream be traceable to some bullet, not a dedicated line each.
- Audit fix (wave ordering): this workstream's `wave` field is now `4`, a serial closer, not `3`. It must never appear in wave 3's own concurrent list - a workstream cannot be concurrent with the branches it depends on. If the surrounding skeleton's waves[] array still lists docs-refresh under wave 3's concurrent set, that array is stale and should be corrected to a standalone wave-4 entry with `concurrent: ['docs-refresh']` and a note that it runs after wave 3 fully lands.
- Audit fix (fire_hook events): mcp_parity's original narrative promised six @on events but only export_done had a committed call site among the wave 1/2 plans reviewed. This workstream does not fabricate the other five as shipped - implementation_order step 5 makes grepping the real fire_hook call sites mandatory before docs/customizing.md's event list is written, so the doc reports whichever subset actually landed (with a gap noted for any event no workstream ended up wiring).

## Review trail

- Prior round (retained): Finding 1 (editor.log toast limitation) APPLIED - verified src/scripting.rs:38/40/200 and src/ui/app.rs:1092,~6365 confirm editor.log() renders only as an auto-expiring (5-10s) egui toast with no copy affordance. Tool table transcribed by hand from curl's raw tools/list JSON; gen_docs.luau demoted to a live spot-check only.
- Prior round (retained): Finding 2 (est_new_lines too low) APPLIED - raised est_new_lines from 60 to 550, justified by ARCHITECTURE.md's six new sections (~250-300 lines) plus docs/customizing.md's ~100-150 line tool table plus remaining doc additions.
- Prior round (retained): Finding 3 (Achieved-bullet count contradiction) APPLIED - resolved in favor of the grouped ~14-bullet plan; acceptance criterion reworded to 'every merged workstream traceable to at least one bullet, bullets may group several workstreams'.
- This round - Finding A (wave-ordering self-contradiction) APPLIED: verified via cross-plan wave/depends_on check that docs-refresh (wave 3, depends_on pro-timeline/pro-monitor/text-titles, all also wave 3) was listed in wave 3's own concurrent set alongside its dependencies - impossible, since it can't start until they finish. Changed this workstream's `wave` field from '3' to '4' (a standalone serial closer); depends_on left unchanged (now correctly one wave earlier). Added a risks[] entry and a ponytail_notes entry flagging that the surrounding skeleton's waves[] array needs the matching correction (docs-refresh removed from wave 3's concurrent list, given its own wave-4 entry) if it still shows the old grouping. Added a matching acceptance_criteria/verification line so a reviewer checks this workstream is never re-listed as concurrent with what it depends on.
- This round - Finding B (unowned @on hook events) APPLIED: cross-checked mcp_parity's promised six events (selection_changed, import, export_done, project_open, project_save, marker_added) against every workstream's files[]/touches_shared text and found only export_done (export-deliver's finish_export) had a committed fire_hook call site; the other five had no matching diff in any reviewed plan. Since this workstream only documents what shipped and cannot itself add fire_hook call sites to other workstreams' code (scope_out), the fix implemented here is: (1) implementation_order gained a new step 5 requiring a grep of real fire_hook( call sites in the merged tree before writing any event documentation, (2) docs/customizing.md's file description and the acceptance criteria/verification now require the event list to reflect only events that actually have a call site (with gaps noted, not silently restated as shipped), (3) agents.md and se-review gained explicit rules that a new @on event needs a documented fire_hook call site, so future workstreams introducing the other five events are caught if they omit the wiring, (4) a new risks[] entry records the gap and its mitigation path. The underlying fix of actually wiring the five missing call sites belongs to the owning workstreams (layout-modes-onboarding for selection_changed, forgiveness for project_open/project_save, media-library for import, trim-model/audio-analysis for marker_added) named in the audit's own fix text - out of scope for this docs-only workstream to implement directly.
- Both findings in this round cited verifiable evidence (a direct depends_on/wave self-contradiction; a direct absence of fire_hook call sites in the reviewed files[] diffs) and both fixes are applied within this workstream's own scope (wave placement changed; documentation policy tightened to report reality, not the promise). No finding was rejected. Everything else preserved verbatim from the prior revision: scope_in/scope_out intent, motivation, luau surface, tests, ui_changes, model/engine/mcp_tools/settings/project fields (all empty, unchanged).
