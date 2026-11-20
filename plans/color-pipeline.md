# Colour pipeline: 16-bit float, linear light, 10-bit end to end

Section `parity-color-pipeline` of `plans/cloud-handoff.md`, item 12 of `plans/shotcut-comparison.md`.
Shotcut 25.12 added a Processing Mode setting (native 8-bit, native 10-bit, linear 8-bit, linear 10-bit).
Simple Editor composites in RGBA8, so HDR and 10-bit exports carry 8-bit banding from the middle of the
pipeline. This plan adds a **Settings ▸ Performance ▸ Processing mode** choice, with today's 8-bit path kept
as the fast default.

## Where the 8 bits are today (develop, 41c9b73)

| Stage | Where | Format |
|---|---|---|
| Decode | `media::mf` (RGB32 out), `media::ffpipe` (`-pix_fmt rgba`), D3D11 zero-copy (`gpu/interop.rs`) | 8-bit BGRX / RGBA, `media::Frame` |
| Layer upload | `GpuRenderer::upload` / `upload_gpu` | `RGBA8` textures |
| Effect chains, node graphs, masks | `run_chain`, `eval_graph_on`, `render_mask` (ping-pong `Target`s from `acquire`) | `RGBA8` FBOs: every pass rounds to 8 bits |
| Compositing, transitions, fades | `composite_cropped` → `shaders::COMPOSITE` (every layer, crossfade, fade-to-colour, subtitle and shader transition ends here) | `RGBA8` canvas, blending on gamma-encoded values |
| Preview | `render_preview_texture` → `register_native_glow_texture` | canvas texture painted by egui |
| Export readback | `render_frame` / `pbo_read` (RGBA or the `NV12_MAIN` pack) → ffmpeg `rawvideo` | 8-bit; HDR goes `rgba` → `zscale` → `yuv420p10le` (10-bit container, 8-bit content) |
| CPU fallback | `engine::compose::Compositor`, `engine::effects` | RGBA8 |

The good news: everything that mixes two pictures funnels through **one** shader (`COMPOSITE`) and every
off-screen target comes from **one** allocator (`GpuRenderer::acquire`). The format and the maths can both
change in one place each.

## The setting

`Settings::processing_mode: ProcessingMode` (serde string, default `Fast8`; old settings files load as `Fast8`).

| Mode | Label | Targets | Compositing | Export of a 10-bit-capable encode |
|---|---|---|---|---|
| `Fast8` | 8-bit (fast) | RGBA8 | gamma-encoded (today, byte-for-byte unchanged) | 8-bit, as today |
| `Float16` | 16-bit float | RGBA16F | gamma-encoded, no rounding between passes | 16-bit readback → 10-bit encode |
| `Linear16` | 16-bit float, linear light | RGBA16F | **linear light** blends, opacity, scaling, crossfades | 16-bit readback → 10-bit encode |

Shotcut's "linear 8-bit" is left out on purpose: linear values in 8 bits band visibly in the shadows,
which is the thing this work removes. Labels follow the house rule (just the name; the explanation in a tooltip).

The setting is global (Shotcut's is too) so it never changes what a project file means. It applies to the GPU
renderer only; the CPU compositor (no GL, `--selftest`, GPU preview off) stays 8-bit and the row says so in its tooltip.

## Design

### 1. Float targets (`Float16`, `Linear16`)

- `GpuRenderer` gets `mode: ProcessingMode` and `set_mode(mode)`. A mode change frees the pool, the loaned
  targets, the Source canvas and every layer texture (they are rebuilt next frame: one frame of work, never
  while a frame is half drawn because `set_mode` runs from `App::sync_gpu` between frames).
- `acquire` allocates `RGBA16F` (`HALF_FLOAT`) instead of `RGBA8` when the mode is not `Fast8`. Everything that
  reads a target keeps working: samplers return floats either way, `glReadPixels(.., UNSIGNED_BYTE)` converts,
  the `NV12_MAIN` pack reads with `texelFetch`, and egui samples the preview texture like any other.
- **Storage stays gamma-encoded** (BT.709/sRGB-style display values, 0..1) in every mode. That is the key
  choice: every effect body, LUT, curve, keyer, user shader and transition was written for display values, and
  keeps getting them. Half floats have an 11-bit significand, so encoded 0..1 values hold better than 10 bits.
- Values are not clamped between passes in float targets (an effect can overshoot and a later one bring it
  back); `COMPOSITE` keeps its existing clamps on the sampled layer, so the final canvas is in range.
- Layer textures stay `RGBA8` for 8-bit sources (no upload cost, no extra memory). 16-bit sources (step 4) upload
  as `RGBA16`.
- Memory: a 4K float target is 64 MB instead of 32 MB. `Pool::MAX_FREE_PX` counts pixels, so the pool's byte
  budget doubles in float modes; it is halved for them (`max_free_px()`), keeping ~256 MB.

### 2. Linear light (`Linear16`)

- `COMPOSITE` gets `uniform int u_linear`. When set, each tap of `sample_layer` and the destination texel are
  decoded to linear (`to_lin`, the sRGB piecewise curve, which is what a display shows Rec.709 SDR as), the
  blend (opacity, coverage, the blend modes, crossfades, fade-to-colour) runs on linear values, and the result
  is encoded back (`to_enc`) before it is written. Scaling (bilinear / bicubic) also averages linear values, so
  a downscaled fine pattern keeps its brightness.
- Because storage stays encoded, linear light is **one uniform in one shader**: no other pass, no extra
  conversion pass at the end, nothing for preview or export to undo.
- Blend modes defined on display values (Overlay, Soft Light, Screen…) get linear inputs in this mode, as they
  do in other linear-light editors; Normal, Add, Multiply and every crossfade are where the difference shows.
- Cost: two `pow`s per tap (4 per pixel bilinear, 16 bicubic, plus the destination). Measured against the
  perf gate on Windows before this leaves draft (see Verification); if bicubic at 4K is too slow, linearise the
  4-tap bilinear result per tap and the bicubic result once (approximation documented in the shader).

### 3. 10-bit export

- `GpuFrameRequest` gets `deep: bool`: read the canvas back as `RGBA` / `UNSIGNED_SHORT` (8 bytes a pixel) and
  feed ffmpeg `-pix_fmt rgba64le` (`VideoIn::Rgba64`). The export picks it when the mode is not `Fast8`, the GPU
  path is on, there is no `FrameOverlay` (those are 8-bit CPU functions) and the encode is 10-bit:
  - HDR projects (already `yuv420p10le` / `p010le`): `hdr_vf`'s `zscale` now gets real 16-bit input.
  - SDR exports whose encoder takes 10 bits (libx265, `hevc_*`, the AV1 encoders, ProRes): `deep_pix_fmt`
    switches `-pix_fmt yuv420p` to `yuv420p10le` (`p010le` for hardware encoders) plus `-profile:v main10` for
    HEVC. H.264 and VP9 stay 8-bit (10-bit H.264 plays almost nowhere).
- The NV12 readback is skipped for deep exports (it is an 8-bit pack). A P010 pack shader (`R16` target, half the
  bytes of `rgba64le`) is the follow-up if the pipe turns out to be the bottleneck.
- A deep frame that falls back to the CPU (GPU stopped answering) is widened from RGBA8 (`widen_rgba16`), so the
  ffmpeg input format never changes mid-file.

### 4. 10-bit decode

- `media::Frame` gets `deep: bool`: `rgba` holds `rgba64le` (8 bytes a pixel). Deep frames only ever reach a
  `LayerSet` for the GPU; `to_rgba`/`make_rgba` narrow them for any CPU reader, so nothing that reads 4-byte
  pixels sees one.
- `GpuRenderer::upload` uploads a deep frame as `RGBA16` / `UNSIGNED_SHORT`.
- Producers, in order:
  1. ffmpeg pipe decode (`ffpipe::open_video`) asks for `rgba64le` when the mode is not `Fast8` and the source's
     `bits_per_raw_sample` / pix_fmt is above 8 bits (probed once per asset).
  2. Media Foundation: ask the source reader for `MFVideoFormat_A16B16G16R16F` / `P010` on 10-bit sources.
     Needs a Windows machine to test; not attempted from the cloud session.
  3. Zero-copy D3D11 (`interop`) shares `DXGI_FORMAT_P010` as two GL textures and converts in a shader. Later.
- Proxies stay 8-bit (they exist to be cheap); exports always decode the originals.

### 5. UI and tools

- Settings ▸ Performance ▸ **Processing mode**: a combo with the three labels; the tooltip explains each and
  says the CPU compositor stays 8-bit.
- The perf HUD shows the mode (`8-bit` / `16f` / `16f lin`) next to the GPU timings.
- MCP: `settings.set processing_mode` works through the generic settings tool; `editor.context` reports it.

## Playback never stutters

- `Fast8` is byte-for-byte today's path: `acquire` and `COMPOSITE` take the same branches, `u_linear = 0`.
- Float modes add no CPU work on the UI thread: the format change is in GL allocation, linear light is ALU in
  the shader. Readback happens only in export (off the preview path), and deep decode runs on the decoder threads.
- Mode changes free targets between frames; the first frame after one re-allocates (like a preview resize does today).
- Gate on Windows: `scripts/perf-bench.ps1 -Synthetic` in each mode; `Float16` must hold 0 dropped frames at
  1080p60, and `Linear16` at 1080p60 bilinear. If a GPU fails that, the tooltip says so; the mode is the user's choice.

## Steps (one commit and push each)

1. This document.
2. `ProcessingMode` + the setting + the Settings row + `GpuRenderer::set_mode`, float targets (`Float16`).
3. Linear light in `COMPOSITE` (`Linear16`), with CPU reference functions and tests for the transfer curves.
4. 10-bit export: deep readback, `rgba64le` input, `deep_pix_fmt`, tests on the argument building.
5. Deep `Frame` + `RGBA16` upload + the ffmpeg pipe's 16-bit decode for >8-bit sources.
6. Hand-off notes, CHANGELOG, docs.

## Verification

From the cloud session (Linux): `cargo check --target x86_64-pc-windows-gnu --all-targets`, and unit tests that
build on Linux. The coordinator on Windows: `cargo test` (the GPU tests run on the real driver there; new ones:
a float target holds a value an 8-bit one rounds, linear 50% crossfade of black/white reads ~0.735 encoded, a
deep export frame reads back 16-bit), `--selftest`, a 10-bit export checked with `ffprobe -show_streams`
(`pix_fmt=yuv420p10le`) and a gradient's banding by eye, and the perf gate per mode.

## Out of scope

- Wide-gamut working space (BT.2020 linear compositing) and scene-referred HDR grading: the working space stays
  Rec.709 primaries; HDR stays an export transform.
- Making the CPU compositor float (it is the fallback, and `Fast8` is the fast default).
- Display colour management of the preview window (egui paints 8-bit sRGB).
