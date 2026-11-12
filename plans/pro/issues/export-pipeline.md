# Export pipeline: GPU from the Export pane, batched readback, overlapped stages

**Workstream:** `export-pipeline` · **Issue:** [#145](https://github.com/KashTheKing/simple-editor/issues/145) · **Plan:** [plans/pro/README.md](../README.md) · **Wave:** 2 · **Coding batch:** A · **Branch:** `feat/pro`

Read [agents.md](../../../agents.md), [goals.md](../../../goals.md), [notes.md](../../../notes.md), [ARCHITECTURE.md](../../../ARCHITECTURE.md) and the master plan [README.md](../README.md) before starting: its findings table gives the file:line evidence, and its "Execution" section gives this workstream's file ownership and the rules for the first coding pass.

## Scope

1. **Export pane uses the GPU**: `export_ui.rs:123` takes `export_frames()` like every other
   entry point. Fixes speed and the silently dropped GPU-only effects in one line; keep the
   existing CPU fallback and its `cpu_gaps` message.
2. **More than one frame per repaint**: the export thread sends a small window of requests
   (bounded channel of 4, replies in order); `serve_gpu_exports` serves for a ~12 ms budget per
   `update`; reply buffers are recycled instead of `Frame::default()` + zeroed resize per frame.
3. **Async readback**: a ring of 3 PBOs in `GpuRenderer::render_frame` so `glReadPixels` never
   stalls the pipeline.
4. **NV12 out of the GPU**: one extra shader pass converts the canvas to NV12 before readback;
   ffmpeg gets `-pix_fmt nv12` input. 2.7x fewer bytes read back and piped, and ffmpeg's swscale
   pass disappears. (CPU path keeps RGBA.)
5. **Overlap the stages**: audio mix on its own thread while video renders (it already has its own
   decoder pool and temp file); a writer thread with a 3-frame queue so the pipe write overlaps
   the next decode/render. Parallel layer decode from wave 1 applies here for free.
6. Pipe audio to ffmpeg as headerless `f32le` (`-f f32le -ar 48000 -ac 2`): deletes `wav_header`
   and its 4 GB / 3 hour ceiling (`export.rs:524`).
7. Fix found on the way: `loudnorm` without `-ar` can leave WAV/FLAC at 192 kHz; add `-ar 48000`.


## Files owned

`run_export` / `mix_to_wav` / `GpuScratch` in `src/engine/export.rs`, `serve_gpu_exports` in `src/ui/app/gpu.rs`, `render_frame` + readback in `src/engine/gpu.rs`, the `frames:` line in `src/ui/export_ui.rs`

## Done when

- Everything under Scope is implemented, or the gap is named in the PR/report with a reason.
- Structural rules hold: ToolDef row for every new Action / Pane / `pub fn(&mut self)` on Project, an `assert_no_idle_repaint_<context>` test for every new pane or window, no new crates, menus through `ui::menu::*`.
- Verification from the master plan's "Verification" section passes for wave 2 (run later, on a quiet machine: `cargo test`, `--selftest`, a viewed `--screenshot`, `scripts/size.ps1 -Note export-pipeline`).
