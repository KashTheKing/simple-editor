# Pre-seed registries, schema fields and Pane::Assistant

**Workstream:** `pro-schema` · **Plan:** [plans/pro/README.md](../README.md) · **Wave:** 0 · **Coding batch:** 0 · **Branch:** `feat/pro`

Read [agents.md](../../../agents.md), [goals.md](../../../goals.md), [notes.md](../../../notes.md), [ARCHITECTURE.md](../../../ARCHITECTURE.md) and the master plan [README.md](../README.md) before starting: its findings table gives the file:line evidence, and its "Execution" section gives this workstream's file ownership and the rules for the first coding pass.

## Scope

- New marker sections for the wave 1-3 workstream names in `TOOL_TABLES` / `ACT_HANDLERS` /
  `FRAME_HOOKS` / `WINDOW_DRAWERS` / `PANE_DRAWERS` (`ui/app/mod.rs`), `actions!`
  (`hotkeys.rs`), `Glyph`, `Pane::ALL`, `Settings`, `App`.
- Serde fields, all `#[serde(default)]`: `Marker.done: bool`, `Marker.author: String`;
  `ExportOptions.audio: AudioLayout` (`Mix` | `Streams` | `Stems`) and `stem_by` (`Track` | `Bus` |
  `Role`); `Settings.assistant_*` (backend, command template, auto-send voice), `Settings.hud`.
- `Pane::Assistant` variant declared (empty drawer), following ARCHITECTURE.md's new-pane checklist.


## Files owned

shared tables: `src/ui/app/mod.rs` registries + `App`, `src/hotkeys.rs`, `src/ui/tools.rs`, `src/ui/layout.rs`, `src/settings.rs`, `src/model/marker.rs`, `ExportOptions` in `src/engine/export.rs`

## Done when

- Everything under Scope is implemented, or the gap is named in the PR/report with a reason.
- Structural rules hold: ToolDef row for every new Action / Pane / `pub fn(&mut self)` on Project, an `assert_no_idle_repaint_<context>` test for every new pane or window, no new crates, menus through `ui::menu::*`.
- Verification from the master plan's "Verification" section passes for wave 0 (run later, on a quiet machine: `cargo test`, `--selftest`, a viewed `--screenshot`, `scripts/size.ps1 -Note pro-schema`).
