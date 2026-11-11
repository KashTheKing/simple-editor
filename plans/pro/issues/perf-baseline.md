# Frame-time HUD, export and source-monitor benches, baselines

**Workstream:** `perf-baseline` · **Plan:** [plans/pro/README.md](../README.md) · **Wave:** 0 · **Coding batch:** 0 · **Branch:** `feat/pro`

Read [agents.md](../../../agents.md), [goals.md](../../../goals.md), [notes.md](../../../notes.md), [ARCHITECTURE.md](../../../ARCHITECTURE.md) and the master plan [README.md](../README.md) before starting: its findings table gives the file:line evidence, and its "Execution" section gives this workstream's file ownership and the rules for the first coding pass.

## Scope

- **Frame-time HUD** (a View toggle + `playback.status` fields): UI-thread frame ms, GL composite
  ms, texture upload ms, per-frame decode ms (sum and worst layer), cache hit rate, dropped frames.
  Timers are `Instant` pairs around existing calls in `ui/app/mod.rs` (`update`),
  `engine/gpu.rs` (`upload`, `render_canvas`) and `playback.rs` (`decode_layers`).
- **`bench_export`** (`#[ignore]`, next to `bench_4k_preview`): a fixed 10 s 1080p60 two-layer
  project through CPU path, GPU path, libx264 and the best hardware encoder; prints fps.
- **`bench_source_monitor`**: a 1080p60 H.264 file through the Source player, frames shown vs due.
- Record baselines in notes.md and turn them into budgets in goals.md (targets to confirm against
  the baseline: 1080p60 single clip plays with 0 dropped frames in both monitors; UI frame p95
  under 8 ms while playing; export at least 3x the baseline fps on this machine).
- goals.md: the size-budget decision above.


## Files owned

new `src/ui/hud.rs`; timers in `src/ui/app/mod.rs` (`update`), `src/engine/gpu.rs`, `src/playback.rs`; `src/ui/app/tools_playback.rs`; benches; `goals.md`, `notes.md`

## Done when

- Everything under Scope is implemented, or the gap is named in the PR/report with a reason.
- Structural rules hold: ToolDef row for every new Action / Pane / `pub fn(&mut self)` on Project, an `assert_no_idle_repaint_<context>` test for every new pane or window, no new crates, menus through `ui::menu::*`.
- Verification from the master plan's "Verification" section passes for wave 0 (run later, on a quiet machine: `cargo test`, `--selftest`, a viewed `--screenshot`, `scripts/size.ps1 -Note perf-baseline`).
