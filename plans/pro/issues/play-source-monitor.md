# Source monitor on the GPU, proxy and cache path

**Workstream:** `play-source-monitor` · **Issue:** [#142](https://github.com/KashTheKing/simple-editor/issues/142) · **Plan:** [plans/pro/README.md](../README.md) · **Wave:** 1 · **Coding batch:** A · **Branch:** `feat/pro`

Read [agents.md](../../../agents.md), [goals.md](../../../goals.md), [notes.md](../../../notes.md), [ARCHITECTURE.md](../../../ARCHITECTURE.md) and the master plan [README.md](../README.md) before starting: its findings table gives the file:line evidence, and its "Execution" section gives this workstream's file ownership and the rules for the first coding pass.

## Scope

- One `App::configure_player(&mut Player)` that does what `app.player` gets today (`set_gpu`,
  `set_cache_bytes`, `set_proxies`), called for the Source player (`ui/app/source_pane.rs:125`,
  `ui/source_ui.rs:468,545`) and wherever proxies/GPU state change (`ui/app/gpu.rs:12-24`,
  `ui/app/jobs.rs:296`).
- Source pane paints the GPU canvas texture like the Preview does (`ui/app/gpu.rs:86`
  `gpu_preview_texture`), using a second canvas target in `GpuRenderer`, the same way the
  alt-render channel already renders a second view (`ui/app/monitor.rs`). Deletes the
  `ColorImage::from_rgba_premultiplied` copy and the unconditional per-frame repaint
  (`source_pane.rs:187,198`).
- Source player gets a small cache share (e.g. 1/8 of `cache_mb`) rather than the 512 MB default.


## Files owned

`src/ui/app/source_pane.rs`, `src/ui/source_ui.rs`, `configure_player` + source canvas texture in `src/ui/app/gpu.rs`, second canvas target in `src/engine/gpu.rs`, the `set_proxies` call in `src/ui/app/jobs.rs`

## Done when

- Everything under Scope is implemented, or the gap is named in the PR/report with a reason.
- Structural rules hold: ToolDef row for every new Action / Pane / `pub fn(&mut self)` on Project, an `assert_no_idle_repaint_<context>` test for every new pane or window, no new crates, menus through `ui::menu::*`.
- Verification from the master plan's "Verification" section passes for wave 1 (run later, on a quiet machine: `cargo test`, `--selftest`, a viewed `--screenshot`, `scripts/size.ps1 -Note play-source-monitor`).
