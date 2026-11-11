# Decode path: no per-frame copies, no CPU swizzle

**Workstream:** `play-decode-path` · **Issue:** [#143](https://github.com/KashTheKing/simple-editor/issues/143) · **Plan:** [plans/pro/README.md](../README.md) · **Wave:** 1 · **Coding batch:** A · **Branch:** `feat/pro`

Read [agents.md](../../../agents.md), [goals.md](../../../goals.md), [notes.md](../../../notes.md), [ARCHITECTURE.md](../../../ARCHITECTURE.md) and the master plan [README.md](../README.md) before starting: its findings table gives the file:line evidence, and its "Execution" section gives this workstream's file ownership and the rules for the first coding pass.

## Scope

In order; each step is kept only if `bench_4k_preview` and the HUD say it is faster:
1. **Stop copying**: decode straight into a pooled buffer that becomes the cached `Arc<Frame>`
   (kills `Arc::new(out.clone())` at `media/mod.rs:439` and the `emit` memcpy at `mf.rs:602`);
   make `Frame::resize` not zero-fill when the decoder overwrites every byte; let the spare pool
   hold more than 8 buffers so a cold cache does not allocate per frame.
2. **Stop swizzling**: keep MF's BGRX as-is and upload with `GL_BGRA` (a `Frame.bgrx` flag; the
   shader forces alpha to 1; CPU consumers convert lazily). Removes the per-pixel Rust loop
   (`mf.rs:643`) from every video frame.
3. **Spike, measure, then decide: NV12 end to end.** Ask MF for NV12 instead of RGB32, upload two
   planes (R8 + RG8), convert in the layer shader. Expected: no CPU colour conversion, 2.7x fewer
   bytes to upload, 2.7x more frames in the same cache budget. Risk: MF's processor must still
   scale to the requested size in NV12; if it does not, keep step 2 and drop this.
4. Precompute source-cache keys (no `to_ascii_lowercase` String per lookup,
   `media/mod.rs:226,243`).
5. Only if the HUD still shows hot Rust loops after 1-3: move them into a workspace crate built at
   `opt-level=3` (the `se-engine` path already named in `plans/ui-overhaul/README.md`). Skipped
   otherwise.


## Files owned

`src/media/mf.rs`, `src/media/mod.rs`, `upload` in `src/engine/gpu.rs`, layer sampling in `src/engine/shaders.rs`, lazy BGRX conversion where `src/engine/compose.rs` reads a frame

## Done when

- Everything under Scope is implemented, or the gap is named in the PR/report with a reason.
- Structural rules hold: ToolDef row for every new Action / Pane / `pub fn(&mut self)` on Project, an `assert_no_idle_repaint_<context>` test for every new pane or window, no new crates, menus through `ui::menu::*`.
- Verification from the master plan's "Verification" section passes for wave 1 (run later, on a quiet machine: `cargo test`, `--selftest`, a viewed `--screenshot`, `scripts/size.ps1 -Note play-decode-path`).
