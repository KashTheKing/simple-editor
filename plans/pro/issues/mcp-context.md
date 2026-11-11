# MCP: editor context, comments, grouped undo, images, external live mode

**Workstream:** `mcp-context` · **Plan:** [plans/pro/README.md](../README.md) · **Wave:** 3 · **Coding batch:** A · **Branch:** `feat/pro`

Read [agents.md](../../../agents.md), [goals.md](../../../goals.md), [notes.md](../../../notes.md), [ARCHITECTURE.md](../../../ARCHITECTURE.md) and the master plan [README.md](../README.md) before starting: its findings table gives the file:line evidence, and its "Execution" section gives this workstream's file ownership and the rules for the first coding pass.

## Scope

- **`editor.context`** (Read): one snapshot of what the user is looking at: playhead, in/out,
  selected clips (names, times, tracks), selected markers, cue selection, edit point, selected
  transcript words *with their text*, current sequence and page. Built from `frame::SelSig`
  (`ui/app/frame.rs:21-51`) plus `timeline.selected_marker`, `markers.selected`,
  `TranscriptUiState.sel`.
- **Comments = markers with notes**: `markers.list` returns `note`/`icon`/`done`/`author`;
  new `markers.set` (uses the existing `Project::marker_mut`); `markers.add` fires
  `marker_added`. Timeline draws range markers as spans and shows the note on hover
  (`ui/timeline/mod.rs:1633-1661`, `paint.rs:222-262`). A "Comment" action adds a marker over
  the in/out range or selection and focuses its note. Agents can list open comments and mark
  them `done`.
- **One undo step per request, attributed**: `App::begin_group(label)` / `end_group()` around the
  snapshot logic in `handle_tool` / `run_tool_undoable` (same pattern `run_script` already uses,
  `mcp_exec.rs:27-86`); exposed as `edit.begin{label}` / `edit.end` and a `batch{calls}` tool.
  The undo entry is labelled "Claude: <label>" (`UndoEntry.label`, `mod.rs:491-555`); a group
  left open closes itself after 30 s idle.
- **Visible feedback**: when a group ends, the clips it changed glow on the timeline (reuse the
  existing glow in `ui/app/layout_ctl.rs`) and a toast offers Undo (`feedback.rs:74`).
- **Speed**: `poll_mcp` drains calls for a few ms per frame instead of one per frame
  (`mcp_exec.rs:158-163`); only `render.frame` keeps its own frame.
- **Agents can see**: tool results that are images return MCP image content blocks instead of
  base64 text (`mcp/mod.rs:290-298`); new `render.contact_sheet{from,to,count}` returns a grid of
  frames in one image.
- **Works for every client**: the `initialize` reply carries an `instructions` guide (call
  `editor.context` first, wrap edits in `edit.begin/end`, cut back-to-front, use island
  transcripts for cut points, check results with `render.frame`).
- **External live mode**: `assistant.wait{timeout}` (Job) blocks until the user sends a request
  from the panel or adds a comment, then returns it with the context. An outside agent loops on
  it; no server push needed.
- Fix `.claude/commands/se-coedit.md` and the "Copy command" text to report the port actually in
  use, and pick a free port instead of switching MCP off when the configured one is taken
  (`mcp_exec.rs:130-134`).


## Files owned

`src/mcp/*`, `src/ui/app/mcp_exec.rs`, new `src/ui/app/tools_context.rs`, marker tools in `src/ui/app/tools_subtitles.rs`, `src/model/ops/markers.rs`, marker drawing in `src/ui/timeline/mod.rs` + `paint.rs`, `.claude/commands/se-coedit.md`

## Done when

- Everything under Scope is implemented, or the gap is named in the PR/report with a reason.
- Structural rules hold: ToolDef row for every new Action / Pane / `pub fn(&mut self)` on Project, an `assert_no_idle_repaint_<context>` test for every new pane or window, no new crates, menus through `ui::menu::*`.
- Verification from the master plan's "Verification" section passes for wave 3 (run later, on a quiet machine: `cargo test`, `--selftest`, a viewed `--screenshot`, `scripts/size.ps1 -Note mcp-context`).
