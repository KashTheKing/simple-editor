# Hand-off: parity-color-pipeline (branch `claude/bold-galileo-67nrfo`)

Section "parity-color-pipeline" of `plans/cloud-handoff.md`. The section names the branch
`claude/parity-color-pipeline`. This cloud session may only push to `claude/bold-galileo-67nrfo`, so the work
is there, branched from `origin/develop` at `41c9b73`. Design: `plans/color-pipeline.md`.

## Done

- **Settings ▸ Performance ▸ Processing mode** (`settings::ProcessingMode`: `Fast8` default, `Float16`,
  `Linear16`), stored by name, so old settings files load as `Fast8`. The row uses selectable buttons with a tooltip on each.
  `App::sync_gpu` passes it to `GpuRenderer::set_mode` (a no-op unless it changed). The perf HUD shows it, and
  `editor.context` has `processing_mode` (null = no GPU renderer).
- **Float targets**: `GpuRenderer::acquire` makes `RGBA16F` targets in the float modes (`target_format`). The pool
  is keyed by format: `Pool::set_float` frees the free list, `Pool::put` frees a target of the old format when it
  comes back off loan, and its pixel budget halves so the bytes stay the same. Values stay gamma-encoded in every
  target, so effects, LUTs, user shaders, readback, NV12 and the preview are untouched.
- **Linear light**: `COMPOSITE` has `u_linear`. Taps and the destination are decoded with the sRGB curve, mixed,
  then encoded back. Every layer, crossfade, fade-to-colour, subtitle and shader-transition result goes through it.
  CPU reference: `gpu::to_linear` / `to_encoded`.
- **10-bit export**: `FrameSource::Gpu(tx, float)`; `GpuFrameRequest::deep`; `gpu::Readout`
  (`Rgba` / `Nv12` / `Rgba16`). A float mode's GPU export with a >8-bit `-pix_fmt` reads back
  `RGBA`/`UNSIGNED_SHORT` and feeds ffmpeg `rgba64le` (`VideoIn::Rgba64`). `deep_args` turns SDR HEVC / AV1 into
  `yuv420p10le` / `p010le` (+ `main10` for HEVC). H.264, VP9 and MPEG-4 stay 8-bit. HDR exports are already 10-bit
  and now get 16-bit input. A frame that falls back to the CPU is widened (`widen_rgba16`). `rgba_bt709` handles the
  10-bit formats.
- **Deep frames**: `Frame::deep` (rgba64le pixels) uploads as an `RGBA16` texture (`LayerTex::deep`, counted at 8
  bytes in the texture budget). `to_rgba` / `make_rgba` / `copy_from` / `resize` narrow it or reset it for CPU
  readers. Interop never reuses a deep texture as a D3D copy target.
- CHANGELOG (Unreleased ▸ Processing mode), `website/docs/interface.mdx`.

## Not done (next, in this order)

1. **A decoder that produces deep frames.** Nothing sets `Frame::deep` yet, so 10-bit sources are still decoded
   to 8 bits. The float modes remove the rounding *between* passes and at export, but not at decode.
   - Media Foundation (the main decoder): ask the source reader for P010 / `A16B16G16R16F` on >8-bit sources. That
     needs Windows to test.
   - The ffmpeg pipe (`ffpipe::VideoPipe`): `-pix_fmt rgba64le` for >8-bit sources, plus a 16-bit `box_down`.
     Only frames headed for a `LayerSet` should be deep (`DecoderPool::frame_at` promises RGBA).
   - Zero-copy D3D11: share P010 as two GL textures and convert in a shader.
2. **A P010 readback pack** (`R16` target, like `NV12_MAIN`): it moves 3 bytes a pixel instead of the 8 that
   `rgba64le` moves, if the pipe turns out to slow 4K exports.
3. Shotcut's "linear 8-bit" mode is left out on purpose (see the design doc).

## Verified here (Linux cloud container)

- `cargo check --target x86_64-pc-windows-gnu --all-targets` passes after each step, with no new warnings.
- **No tests were run.** `cargo test` cannot build on Linux: winit fails with "The platform you're compiling for
  is not supported by winit", because eframe ships with no Linux backend enabled. Wine 9.0 aborts with
  "free(): invalid pointer" on any program in this sandbox, including `wine cmd /c echo hi`. The new tests compile;
  run them on Windows:
  - `settings::tests::processing_mode_defaults_to_8_bit_and_round_trips`
  - `engine::gpu::tests::{a_mode_change_never_hands_out_a_target_of_the_old_format, the_float_pool_keeps_the_same_bytes, transfer_curves_round_trip_and_mid_grey_is_where_srgb_puts_it, the_composite_shader_carries_the_linear_switch}`
  - `engine::gpu::color_pipeline_tests::*` (real driver: 50 % white over black reads ~128 in 8-bit / 16f and
    ~188 in linear; switching modes back and forth keeps rendering)
  - `engine::export::tests::{deep_args_make_hevc_and_av1_10_bit_and_leave_h264_alone, rgba64_input_and_the_cpu_widening_are_16_bit_little_endian, a_float_mode_export_writes_10_bit_hevc}`.
    The last one runs a real ffmpeg export with a stand-in UI thread and checks for `yuv420p10le` / `Main 10`.
  - `media::tests::deep_frames_narrow_for_cpu_readers`
  - the existing `export_tests` (it now passes `Readout::Rgba`).

## For the coordinator to check on Windows

- `cargo test` (the whole suite, since `Frame` and the export readback changed), and `--selftest`.
- The Settings ▸ Performance row by eye (a screenshot), and switching modes while playing: the first frame after a
  switch re-allocates targets. Look for a hitch.
- The perf gate in each mode: `scripts/perf-bench.ps1 -Synthetic` with the setting at 16-bit float and then at
  linear. `Linear16` adds two `pow`s per tap: bicubic is 16 taps, so check 4K bicubic.
- A 10-bit export in 16-bit float mode: `ffprobe` shows `yuv420p10le` / `Main 10`, and a gradient shows no 8-bit banding.
- `scripts/size.ps1`: expect well under +64 KB.

## Risks

- glReadPixels of an `RGBA16F` target as `UNSIGNED_BYTE` (preview stats, `render.frame`, NV12) relies on desktop
  GL's conversion. GLES would refuse it, but the app runs on desktop GL 3.3 core.
- egui paints the `RGBA16F` preview texture through `register_native_glow_texture`, like any other texture. Not seen by eye yet.
- Blend modes defined on display values (Overlay, Soft Light, Screen…) look different in `Linear16`. That is
  expected for a linear-light mode, but users may notice.
