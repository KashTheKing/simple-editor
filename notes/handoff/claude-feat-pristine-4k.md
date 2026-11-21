# claude/feat-pristine-4k: pristine 4K text and graphics

The user asked whether the blurry text in the Olympus background project comes from the text code or
the engine. It came from the engine. Three causes, all fixed here.

## What changed

1. **Text and shapes are rasterised at their on-screen size** (`compose::raster_zoom`, `half_octave`).
   - Before, a Scale-3 title was a canvas-scale bitmap stretched 3x.
   - Now the rasteriser draws at `s × z`. `z` is `|scale| × max(|scale_x|, |scale_y|)` as a half-octave step: the smallest 2^(k/2) that is at least z / 1.09, from 1 to 64.
   - Scale 3 draws at 2.83x. A 1→3 zoom uses 4 cached steps.
   - Text: `TextRasterizer::render_zoomed` steps `z` down while the bitmap would pass 8192 px a side or 16 Mpx. It judges this from the zoom-1 bitmap, which stays cached beside the zoomed one, and returns the zoom it used.
   - Shapes: `shapes.render(style, s × z)` (that rasteriser already clamps).
   - CPU: `draw_bitmap` now takes the zoom-1 `native` size as f32 and places the bitmap at that size (`bitmap_placement`). Effects get `img_scale = s × (bitmap / native)`, where it used to be `s`.
   - GPU: `playback::layer_for` pushes `LayerSet::zoom` (the browser boost × z for text, bitmap width / round(shape width × s) for shapes).
   - `gpu::draw_clip` divides the placement by that zoom, falling back to `browser::draw_boost` for hand-made sets as before. The non-contain `img_scale` is now `s × zoom`, which also fixes effect radii under a zoomed browser.
2. **Upscaled export renders natively** (`export::render_size`, `same_aspect`).
   - An `out_size` larger than the project, with the same shape (within 1 px), is rendered at `out_size` on the CPU and in the GPU `FrameSource::Gpu` requests (`w, h`).
   - `output_filters` then adds no `scale=`.
   - Downscales, other shapes (stretch and letterbox) and an `overlay` (drawn at project size) keep the old path.
   - Export Frame: `frame_render_size` returns the target for a same-shape upscale. `App::render_frame_sized` and the playback one-shots (`Cmd::RenderOnce` / `LayersOnce`) now go up to `playback::ONCE_MAX_W` = 7680.
   - `render_frame_now` keeps its project-width clamp, so scopes, attachments and `render.frame` are unchanged.
3. **Paused preview uncapped** (`preview_pane::preview_cap`, `preview_cap_for`, `PRISTINE_MAX_W` = 3840).
   - In fullscreen, or with Playback Resolution on Auto, a paused frame renders at the pane's pixel size, up to 3840.
   - Playing keeps `preview_max_width`. On Auto, a play that ends clean at level 100 sets `AutoRes::smooth = Some(true)`, and later plays run uncapped.
   - The first drop there sets `Some(false)`: capped for the rest of the session, with the level untouched, so it never flip-flops.
   - A fixed resolution in a window is unchanged.

## Verified here
- `cargo check --target x86_64-pc-windows-gnu --all-targets`: clean (only the warnings that were already there).
- The Linux test binary can't build (winit has no Linux target in this build). I copied the pure functions into a scratch crate and ran the same assertions as the new tests. All pass:
  - `half_octave`, including the 4 steps for a 1→3 zoom, the step-down lattice and Scale 3 → 2.83
  - `render_size` / `same_aspect`
  - `frame_render_size`
  - `preview_cap_for`

## Not run (please run on Windows)
- `cargo test half_octave scaled_text upscaled_export frame_export_and_preview_sizes paused_frames zoomed_text`
  - `compose::tests::scaled_text_is_rasterised_at_its_on_screen_size`: the bitmap is about 3x, and the drawn box is within 3 px of the stretched zoom-1 box.
  - `text::tests::zoomed_text_reuses_half_octave_steps`: at most 4 rasters for a 1→3 zoom.
  - `text::tests::zoomed_text_stays_under_the_size_cap`
  - `playback::tests::scaled_text_and_shape_layers_carry_their_zoom`: GPU layers carry the zoom.
  - `export::tests::upscaled_export_renders_natively_without_a_scale_filter`
  - `app::tests::frame_export_and_preview_sizes` (updated: 4K from 1080p is now `(3840, 2160)`)
  - `preview_pane::auto_res_tests::paused_frames_are_drawn_at_the_pane_size_in_fullscreen_or_on_auto`
- Full `cargo test`, `--selftest`, `bench_4k_preview`, and the perf gate (`scripts/perf-bench.ps1 -Synthetic`).

## Check by eye on Windows
1. Open the Olympus background project and pause on a big title at Scale 2-3.
   - Auto resolution or fullscreen: the edges should be crisp, with no soft halo, in the preview with the GPU on and off.
   - Compare with `develop`.
2. Scrub through a scale/zoom animation on a title.
   - Play it: no stutter, and no visible jump in size or position at the step changes (√2 apart). Text must not shift by more than a pixel when a step changes.
   - Perf trace: text rasterises a few times, not per frame.
3. Export the 1080p project at 3840x2160.
   - Text, shapes and shader effects should be sharp at 100 % zoom, compared with a `develop` export.
   - Export takes longer, since it renders 4x the pixels.
   - Export at 1280x720 should look and behave as before.
4. Export Frame at 2x and 4x: sharp text.
5. Text with a Glow, Blur, Shadow or Wobble effect at Scale 3: the effect's look (radius) should match `develop`. Only the sharpness should change.
6. Caption-bar styles (TikTok / Snapchat BOX_BAR) at Scale ≠ 1: the bar still spans the frame exactly.
7. A text or shape under a zoomed Browser clip: still sharp and correctly placed (`LayerSet::zoom` replaces `draw_boost` there).
8. Preview, Auto resolution: press play.
   - The first play is capped at 1280. After a smooth play, later plays run at the pane size.
   - On a slow machine, the first drop goes back to the cap for good.
   - Pausing shows the sharp frame within one render.
9. Fixed 50 % resolution, windowed: same as before.

## Risks
- Paused frames at up to 3840 make paused scrubbing and idle pre-render heavier (4x the pixels of 1920). Scrubbing on Auto might feel slower on weak GPUs.
- The canvas size changes on play/pause with Auto or fullscreen, which re-renders and flushes that size's frame cache. Auto already did this when its level was under 100.
- Scale-up text bitmaps cost memory: up to 16 Mpx (64 MB) each, in a 64-entry cache. With z > 1, the zoom-1 bitmap is also kept.
- Not done: node-graph `String` nodes (`gpu.rs text_node`) still rasterise at canvas scale (graph output size). They are rare.
- Not done: Browser-clip internals have their own `raster_scale`.
- Size: a few KB of code, no new dependencies.
