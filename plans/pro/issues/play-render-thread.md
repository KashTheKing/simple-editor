# Render thread: parallel layer decode and cache-friendly sizing

**Workstream:** `play-render-thread` · **Plan:** [plans/pro/README.md](../README.md) · **Wave:** 1 · **Coding batch:** A · **Branch:** `feat/pro`

Read [agents.md](../../../agents.md), [goals.md](../../../goals.md), [notes.md](../../../notes.md), [ARCHITECTURE.md](../../../ARCHITECTURE.md) and the master plan [README.md](../README.md) before starting: its findings table gives the file:line evidence, and its "Execution" section gives this workstream's file ownership and the rules for the first coding pass.

## Scope

- **Parallel layer decode**: in `decode_layers`, video layers that need a real decode run on
  `std::thread::scope` threads, one per distinct source, the first on the render thread itself.
  `// ponytail: a spawn per layer per frame; persistent workers if spawn cost shows in the HUD`.
- Narrow the shared `TextRasterizer` lock to the text layers only (held across all video decodes
  today, `playback.rs:1426`).
- Motion-blur neighbours: decode the earlier sample first so MF reads forward and never seeks
  backwards each frame (`playback.rs:1687-1695`).
- Nested sequences: stop cloning the whole `Project` per frame (`playback.rs:1735`).
- Canvas size: step it (reuse `compose::layer_size`'s stepping) so dragging a pane divider or a
  DPI change does not clear both caches and renegotiate MF (`playback.rs:809-811`).
- `set_project`: skip the deep clone + JSON diff when the edit is audio/marker-only (the app
  already knows the edit category).

**Not doing unless the HUD demands it** (named so nobody re-derives them): PBO texture upload,
caching the timeline's egui shapes during playback, zero-copy DXVA (D3D11 texture shared into GL).


## Files owned

`src/playback.rs`

## Done when

- Everything under Scope is implemented, or the gap is named in the PR/report with a reason.
- Structural rules hold: ToolDef row for every new Action / Pane / `pub fn(&mut self)` on Project, an `assert_no_idle_repaint_<context>` test for every new pane or window, no new crates, menus through `ui::menu::*`.
- Verification from the master plan's "Verification" section passes for wave 1 (run later, on a quiet machine: `cargo test`, `--selftest`, a viewed `--screenshot`, `scripts/size.ps1 -Note play-render-thread`).
