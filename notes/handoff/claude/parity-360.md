# Handoff: parity-360 (`claude/parity-360`, new from `origin/develop` at 41c9b73)

Shotcut comparison item 10: the 360° video filters.

## What's done

**Six library shaders** (`library/effects/`, with `index.json` entries and Gallery thumbnails):

| File | Shotcut filter it covers | Params |
|---|---|---|
| `360-transform.glsl` | 360 Transform | yaw, pitch, roll (default yaw 45, so the library test sees a change) |
| `360-to-flat.glsl` | Equirectangular to Rectilinear | yaw, pitch, roll, fov |
| `360-little-planet.glsl` | Equirectangular to Stereographic | yaw, pitch (-90 = planet, 90 = tunnel), roll, fov |
| `fisheye-to-360.glsl` | Hemispherical to Equirectangular | fov, radius, center_x/y, yaw/pitch/roll, dual (two circles side by side), equisolid, blend |
| `flat-to-360.glsl` | Rectilinear to Equirectangular | yaw, pitch, roll, hfov, vfov (rest transparent) |
| `360-mask.glsl` | Equirectangular Mask | yaw, width, top, bottom, feather, invert (default hides the bottom 20°: the tripod) |

They all use one convention (written in `src/engine/vr360.rs`'s header and `360-transform.glsl`): u = 0.5 is
forward, v = 0 is up, +yaw looks right, +pitch looks up.

**360 Stabilize** (`src/engine/vr360.rs`, `src/ui/app/vr360_ctl.rs`):
- On a job thread (`export::spawn_job`, `bg::lower_thread`), every project frame of the clip is decoded at 512x256.
  80 patches on the sphere are matched between each pair of frames (coarse NCC at half size ±14°, then
  Lucas-Kanade sub-pixel). Edge-like patches are skipped. The rotation is a 3x3 least-squares fit with an
  outlier refit and a second-order correction.
- The rotations chain into the camera path. A Gaussian quaternion average over `smooth` seconds (or
  identity = lock) gives the kept motion. The difference becomes yaw/pitch/roll keys (thinned where a line
  already fits) on the clip's **360 Transform** effect: found by `@name`, or inserted first in the stack
  with the compiled-in copy of the library file (`include_str!`), so it works before the library downloads.
  A constant yaw/pitch/roll the user set by hand is composed in, not overwritten. It's one undo step
  ("360 Stabilize").
- UI: the Inspector's effect row right-click on a 360 Transform effect ▸ **360 Stabilize** ▸ Smooth /
  Lock View / Level Horizon. That goes through `EffectsResponse.stabilize_360` → `inspector::take_stabilize_360`
  → `vr360_ctl::tick`. Progress is a toast.
- MCP: `clip.stabilize_360 {clip_id?, smooth?, yaw?, pitch?, roll?}` (Job). Registered in `TOOL_TABLES`
  and listed in `docs/customizing.md`.
- Registry entries: I added a `// ---- ws:parity-360 ----` marker plus a line in `TOOL_TABLES`, `FRAME_HOOKS`, the
  `App` struct and its init. There's no new Action, Pane or `Project` method, so no extra ToolDef rows are needed.

Docs: `website/docs/effects.mdx` ("360° video"), `CHANGELOG.md` (Unreleased ▸ 360° video),
`docs/customizing.md` row (clip.* count 16 → 17), `plans/shotcut-comparison.md` line updated.

## Verified here (Linux container)

- `cargo check --target x86_64-pc-windows-gnu --all-targets`: passes, no new warnings.
- The six shaders compile with `glslangValidator` once wrapped exactly like `shaders::user_shader` does it (prelude, knob
  defines, `MAIN`).
- I rendered them on Mesa llvmpipe (moderngl, same prelude and VERT) and checked the output by eye and by number:
  - 360 Transform yaw 30 equals a 30° roll of the picture (mean error 1/255).
  - Flat to 360 round-trips a 360 to Flat view (0.7/255).
  - Fisheye to 360 unwraps a synthetic dual fisheye back to the original equirect (2.9/255).
  - The mask's alpha is right.
- Thumbnails come from the same test picture, checkerboard and ffmpeg filter as `scripts/library-index.ps1`.
  My pipeline reproduces the shipped `effects-fisheye.jpg` to within 0.17/255.
- `index.json`: a Python mirror of `build_index` re-generates all 44 existing effect entries byte-for-byte
  and adds the 6 new ones.
- `engine::vr360` tests (9) run natively in a scratch crate with stubbed `Frame`/`DecoderPool`/`Progress`
  and pass:
  - turns are measured to within 0.015° (0.2° to 11° per frame, on a value-noise sphere)
  - an end-to-end shaky sequence locks to the true path within 0.1°
  - smoothing keeps a pan and removes shake
  - key thinning works, and the compiled-in shader equals the library file

## Not verified (needs Windows)

- The full `cargo test`. Wine crashes in this container at startup (`free(): invalid pointer`), so the
  Windows test exe couldn't run. That means `vr360_ctl`'s two tests haven't run:
  - keys land on one front 360 Transform effect
  - a hand-set yaw is kept
  The `library_index_matches_files` test and the real-driver `library_shaders_compile_and_draw_on_the_real_driver`
  (it checks each effect "draws something" and "changes something" with defaults) haven't run either.
- Real 360 footage through the app: decode at 512x256, speed (~30 ms a frame for the matching in an
  optimised build), and how the keys look in the Animator. Live-check: put 360 Transform on a shaky
  equirect clip ▸ right-click ▸ 360 Stabilize ▸ Smooth, and watch the toast and the result.
- Optional: run `scripts/library-index.ps1` to regenerate the 6 thumbnails with the real driver. They should
  come out nearly identical.

## Risks / notes

- `effects_ui.rs` and `inspector.rs` get small hunks (a response field, a thread-local, one submenu), and
  `parity-effects` may touch the same files. `library/index.json` will conflict textually with any branch
  adding library effects. Resolve it by re-running `scripts/library-index.ps1` (or keeping both sides'
  entries in id order).
- Stabilize isn't listed in the Jobs pane (only a progress toast). Adding it means a row in
  `jobs_pane`/`JobKind`, which I left alone to stay out of shared files.
- Stabilize assumes equirect input. On flat footage it still runs but the result means nothing.
- Size: about 1 new source file of maths plus the 2 KB shader string. I expect well under the +64 KB gate, but
  it wasn't measured (that needs a Windows release build).
