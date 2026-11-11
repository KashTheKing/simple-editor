# Interchange: richer FCP7 XML, EDL export for Avid, markers

**Workstream:** `interchange` · **Issue:** [#148](https://github.com/KashTheKing/simple-editor/issues/148) · **Plan:** [plans/pro/README.md](../README.md) · **Wave:** 2 · **Coding batch:** A · **Branch:** `feat/pro`

Read [agents.md](../../../agents.md), [goals.md](../../../goals.md), [notes.md](../../../notes.md), [ARCHITECTURE.md](../../../ARCHITECTURE.md) and the master plan [README.md](../README.md) before starting: its findings table gives the file:line evidence, and its "Execution" section gives this workstream's file ownership and the rules for the first coding pass.

## Scope

- FCP7 XML export gains: track names, clip levels/pan, `<link>` for A/V pairs, cross dissolves,
  sequence and clip **markers** (name, note, duration, colour). XML import reads markers back.
- **EDL (CMX3600) export**, one per video track plus audio: the route into **Avid** (and a
  fallback for Premiere/Resolve), paired with the separate-track audio export above.
- Markers export as an Avid marker text file next to the existing CSV / YouTube-chapters options
  (`ui/markers_ui.rs:236`).
- Not doing: AAF (binary structured-storage format; no dependency-free writer) and OTIO. Add OTIO
  when a user needs Resolve round-trips richer than XML.


## Files owned

`src/engine/xmeml.rs`, `src/engine/import.rs`, new `src/engine/edl.rs`, new `src/ui/app/tools_interchange.rs`, marker export options in `src/ui/markers_ui.rs`

## Done when

- Everything under Scope is implemented, or the gap is named in the PR/report with a reason.
- Structural rules hold: ToolDef row for every new Action / Pane / `pub fn(&mut self)` on Project, an `assert_no_idle_repaint_<context>` test for every new pane or window, no new crates, menus through `ui::menu::*`.
- Verification from the master plan's "Verification" section passes for wave 2 (run later, on a quiet machine: `cargo test`, `--selftest`, a viewed `--screenshot`, `scripts/size.ps1 -Note interchange`).
