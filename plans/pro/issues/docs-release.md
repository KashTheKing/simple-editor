# Docs and release notes for the Pro plan

**Workstream:** `docs-release` · **Issue:** [#153](https://github.com/KashTheKing/simple-editor/issues/153) · **Plan:** [plans/pro/README.md](../README.md) · **Wave:** 4 · **Coding batch:** C · **Branch:** `feat/pro`

Read [agents.md](../../../agents.md), [goals.md](../../../goals.md), [notes.md](../../../notes.md), [ARCHITECTURE.md](../../../ARCHITECTURE.md) and the master plan [README.md](../README.md) before starting: its findings table gives the file:line evidence, and its "Execution" section gives this workstream's file ownership and the rules for the first coding pass.

## Scope

- `website/` pages for the assistant, voice, separate audio tracks and Avid hand-off; regenerate
  screenshots (`scripts/docs-shots.ps1`) and the shortcuts page (`--dump-hotkeys`).
- ARCHITECTURE.md (threads, export pipeline, assistant), `docs/customizing.md` tool table,
  CHANGELOG, goals.md "Achieved", notes.md with the measured before/after numbers.


## Files owned

`website/`, `ARCHITECTURE.md`, `docs/customizing.md`, `CHANGELOG.md`, `goals.md`, `notes.md` (no screenshot regeneration in this pass)

## Done when

- Everything under Scope is implemented, or the gap is named in the PR/report with a reason.
- Structural rules hold: ToolDef row for every new Action / Pane / `pub fn(&mut self)` on Project, an `assert_no_idle_repaint_<context>` test for every new pane or window, no new crates, menus through `ui::menu::*`.
- Verification from the master plan's "Verification" section passes for wave 4 (run later, on a quiet machine: `cargo test`, `--selftest`, a viewed `--screenshot`, `scripts/size.ps1 -Note docs-release`).
