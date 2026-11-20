# Hand-off: `claude/parity-preview-tools`

Section "parity-preview-tools" of `plans/cloud-handoff.md` (Shotcut parity items 7-9 of
`plans/shotcut-comparison.md`). New branch from `origin/develop` (41c9b73).

## Done

1. **Preview grid and safe areas, with snapping; 10 % and 25 % zoom** (`ui/guides.rs`, `ui/preview.rs`,
   `ui/app/preview_pane.rs`, `settings.rs`)
   - `guides::Overlay` (`Settings.preview_overlay`, its own `ws:parity-preview-tools` marker in the struct and
     `Default`): grid columns / rows, `safe`, action % (90) and title % (80). Old settings files load the default (off).
   - Preview right-click: **Grid ▸** Off, Thirds, 2 x 2, 4 x 4, 8 x 8, 16 x 9, 16 x 16, plus Columns / Rows
     drag fields (max 64); **Safe Areas ▸** Show, Action %, Title %. Saved with `save_async`.
   - Drawn in `video_body` over the picture, under the selection outline and handles; not in fullscreen, the
     trim view or the device preview. Light line over a dark under-stroke; action safe solid, title safe dashed.
   - `canvas_snap` takes `&Overlay`: shown grid lines and safe borders are snap targets (new
     `CanvasGuideKind::Grid` / `Safe`), still gated by Snap to Canvas.
   - Zoom presets: Fit, 10, 25, 50, 100, 200 %. `ZOOM_RANGE` floor 0.25 -> 0.02 of fit (10 % of a 1080p
     project in a wide pane is ~0.12 of fit), so Ctrl+wheel also zooms out further.
2. **Video denoise and deinterlace bakes** (`engine/convert.rs`, `ui/app/tools_export.rs`, `ui/app/menus.rs`)
   - `BakeFilter::DenoiseVideo { strength, best }` (hqdn3d light/medium/strong, or nlmeans s=1.5/3/6) and
     `BakeFilter::Deinterlace` (`bwdif=mode=send_frame:parity=auto:deint=all`, frame rate kept).
   - These are **source bakes** (`BakeKind::from_source`): the selected clips' source FILE is filtered (no
     render stage), then `source_swap` points the selected video clips at the new asset. Nothing is flattened:
     effects, transform, speed, trims stay; the new asset inherits master effects, folder, tags, label, range.
     Linked audio keeps the original asset. One undo step, original kept, runs in the existing bake job slot
     (Jobs pane). One video file per bake (`source_bake_target` refuses mixed selections, images, HTML layers,
     stacked-alpha files). Why source and not render-then-filter: a render has already scaled / mixed the
     fields, so deinterlacing it is wrong, and cleanup belongs before effects.
   - **Timeline ▸ Clean Up ▸** Denoise Video ▸ Fast / Best ▸ Light / Medium / Strong, Deinterlace, Stabilize,
     Denoise Audio (the last two were MCP-only before; they still render first).
   - MCP: `export.denoise_video {clip_ids, strength, method}`, `export.deinterlace {clip_ids}` (Job tools).
3. **Corner Pin and Lens Correction effects** (`model/effect.rs`, `engine/effects.rs`, `engine/shaders.rs`)
   - New `EffectKind::CornerPin` (8 params: corner x/y as layer fractions) and `LensCorrection` (centre x/y,
     r^2 and r^4 terms, zoom), appended at the END of the enum (gpu.rs keys programs by discriminant) and before
     Shader in `ALL` (now 31). Category Transform.
   - CPU path (`remap`, bilinear, 1 px soft edge, transparent outside) and GLSL bodies (`CORNER_PIN`, `LENS`)
     use the same formulas (`corner_pin_matrix` / `lens_source` are the CPU twins).

Docs: CHANGELOG `## Unreleased`, `website/docs/viewer.mdx`, `effects.mdx`, `advanced.mdx`,
`docs/customizing.md` tool rows.

## Verified here (Linux container)

- `cargo check --target x86_64-pc-windows-gnu --all-targets`: clean, no errors and no new warnings in the
  touched files. `cargo test --target x86_64-pc-windows-gnu --no-run` builds the test exe.
- **The tests below were written but NOT run.** The crate does not build for Linux (winit refuses the
  platform), and Wine 9 aborts in this container (`free(): invalid pointer` even for `wine cmd /c echo`).
  Run `cargo test guides:: small_zoom video_cleanup source_ corner_pin lens_correction effect_kind_all
  bake_filters shaders::` on Windows first.
- What did run here instead:
  - the five ffmpeg filter strings on a 320x240 30 fps 4 s file (ffmpeg on Linux): every one keeps
    320x240, 30 fps, 120 frames and the 4.0 s length, so the clips' timing holds after a source swap;
  - `corner_pin_matrix` / `corner_pin_inverse` / `homography` / `lens_source`, extracted verbatim from
    `effects.rs` into a scratch program with the tests' assertions plus a keystone round trip: all pass;
  - `CORNER_PIN` and `LENS` (with the real `PRELUDE`) through `glslangValidator`: both compile as
    `#version 330 core` (a deliberately broken copy fails, so the check is real).

## Tests added

- `ui::guides::tests`: `overlay_lines_split_the_frame_and_safe_areas_are_centred`,
  `canvas_snap_lands_on_shown_grid_and_safe_lines_only`, `overlay_settings_round_trip_and_old_files_get_the_default`
- `ui::preview::tests::small_zoom_presets_are_reachable`
- `engine::convert::tests::video_cleanup_filters_build_expected_args` (runs ffmpeg end to end when present)
- `ui::app::tools_export::tests`: `video_cleanup_bakes_filter_the_source_and_keep_timing`,
  `source_bake_target_wants_one_video_file`, `source_swap_keeps_the_clips_edits_and_master_effects`,
  `video_cleanup_tools_are_registered`
- `engine::effects::tests`: `corner_pin_homography_hits_the_corners_and_round_trips`,
  `corner_pin_default_is_identity_and_a_pin_clears_outside`, `lens_correction_keeps_the_centre_and_moves_the_edges`
- `model::tests::effect_kind_all_has_29_entries_in_declared_order` updated to 31.

## Not verified (needs Windows)

- Anything on screen: the overlay's look (screenshot it), the menus, the zoom presets.
- The two GLSL bodies have not run on a real driver (only the reference compiler). Check a Corner Pin and
  Lens Correction clip in the GPU preview matches export and the CPU path, and that the existing
  real-driver shader test covers them.
- A real interlaced source through Deinterlace, and the swap in the live app (proxy rebuild for the new
  asset, Jobs pane rows, undo).
- `docs/customizing.md`'s tool count line and the docs screenshots were not regenerated.

## Risks / merge notes

- `EffectKind::ALL` length and the `effect_kind_all_has_29_entries...` test will conflict with parity-effects
  (it adds kinds too): keep both sides' kinds and bump the count to the sum.
- `canvas_snap` gained a parameter; any other branch calling it needs `&Overlay::default()` (or the settings').
- Source bakes write `<stem>_clean.mp4` / `<stem>_progressive.mp4` beside the source; the whole file is
  filtered (not just the used range), so a long source takes long, nlmeans especially.
- Follow-up idea, not done: on-canvas draggable corner handles for Corner Pin.
