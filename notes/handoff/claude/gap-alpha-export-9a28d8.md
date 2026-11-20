# gap-alpha-export hand-off (cloud session, 2026-11-20)

Branch `claude/gap-alpha-export-9a28d8`, `origin/develop` merged in (clean, no conflicts).

## What's in the branch

The paused WIP (`0ef3246`) already had most of the scope. This session merged develop, made it build, reviewed it, and fixed the gaps it found.

- **Transparent export** (`ExportOptions.alpha`, Export pane "Transparent background", MCP `export.video alpha`):
  - MOV: ProRes 4444 (`prores_ks`, `yuva444p10le`) by default, or PNG / QuickTime Animation (`qtrle`, `argb`) from the pane's Codec row.
  - WebM: VP9 `yuva420p`.
  - PNG / TIFF sequences: `rgba`.
  - Any other format exports opaque, and the progress line explains why (`alpha_refusal`). HDR exports are always opaque.
  - New built-in card: **Overlay (ProRes 4444)**.
- **Image sequences:** PNG / JPEG / TIFF, `<folder>/<name>/<name>_00001.png`.
  - Frames render into a hidden sibling folder, which is moved into place only when the export succeeds. A cancelled or failed export leaves nothing behind.
  - Frame step (every Nth frame) and In/Out range.
  - MCP: `format: png_sequence | jpg_sequence | tiff_sequence`, plus `every` and `in_out`.
- **No dark fringes:**
  - CPU compositor: `blend::over_clear`, a straight-alpha "over" used whenever the canvas pixel isn't opaque. It's the same formula the GPU `COMPOSITE` shader already used.
  - GPU export: the canvas clears to `Custom([0,0,0,0])` through `GpuRenderer::bg_override`, set per export frame in `ui/app/gpu.rs`. The live preview keeps its own background.
  - An alpha export never takes the NV12 readback (its pix_fmt isn't `yuv420p` / `nv12`).

### Fixed this session (commit `6e3c61f`)
- Letterbox bars and the even-size `pad` filled opaque black on alpha exports. They now use `color=black@0` when the codec args write alpha (`writes_alpha`). VP9 `yuva420p` now also gets the even-size pad, which it needs.
- Re-exporting a sequence into the same folder with fewer frames (shorter range or bigger step) left the old extra frames behind. `TempDir::commit_with` now removes `<name>_NNNNN.<ext>` frames this run didn't write. Other files in the folder are kept.
- A user preset saved with a sequence format made Quick Export write one `name.png_sequence` file. It now writes into a fresh `<project>_<preset>` folder.
- The temp-folder error text said "stems to …" for every export kind.
- Removed the unused `SEQUENCE_EXTS` const (a new warning).

## Verified here (Linux container)
- `cargo check --target x86_64-pc-windows-gnu --all-targets`: builds, and none of the 300-odd warnings are in files this branch touches.
- `cargo test --target x86_64-pc-windows-gnu --no-run`: the test exe links.
- **The exact ffmpeg arg strings, run with Linux ffmpeg** on a soft-edged white rect over a clear 160x90 RGBA canvas, then decoded back to RGBA (`/tmp/.../scratchpad/ff/check.py`, not committed):

  | output | read back as | corners | soft-edge pixels | darkest edge RGB |
  |---|---|---|---|---|
  | ProRes 4444 `.mov` | `yuva444p12le` | α 0 | 480 | 254 |
  | PNG `.mov` | `rgba` | α 0 | 480 | 255 |
  | Animation `.mov` | `argb` | α 0 | 480 | 255 |
  | VP9 `.webm` | alpha present when decoded with libvpx-vp9 | α 0 | 480 | 253 |
  | PNG / TIFF sequence | `rgba`, 30 files | α 0 | 480 | 255 |
  | JPEG sequence | `yuvj444p` (opaque, as intended) | | | |

  The odd-size `pad … color=black@0` and the letterbox `pad … color=black@0` both read back fully clear (VP9 alpha).
- The `over_clear` maths checked by hand against the unit test's expected bytes: 128 over clear gives 128, a second 128 gives 192, and Multiply over clear draws the layer itself.

## NOT verified (needs the Windows box)
- **No test ran.** The crate doesn't build for Linux (winit refuses the platform), and Wine in this container crashes even on a hello-world exe. Run:
  - `cargo test blend::` (`composite_onto_clear_canvas_has_no_dark_fringe`)
  - `cargo test export::tests::alpha` / `png_sequence_names_counts_and_alpha` / `sequence_frames_and_see_through_pads` / `transparent_prores_keeps_alpha` / `alpha_on_mp4_falls_back_with_a_message`
  - `cargo test a_transparent_export_frame_keeps_alpha_on_the_gpu` (real GPU, CPU vs GPU coverage within 16)
  - the full suite once, since `ExportOptions` gained a field.
- The Export pane UI by eye:
  - The Transparent background checkbox greys out on MP4 and similar formats, and "Use project background" greys out while it's on.
  - The Codec row shows only for MOV + transparent, and Frame step only for sequences.
  - The folder picker for sequences, and the "N frames" output line.
- An exported ProRes 4444 / VP9 file placed over footage in another editor (Resolve / Premiere).
- `--selftest`, the size gate (`scripts/size.ps1 -Note gap-alpha-export`) and the perf gate. Playback isn't touched: `bg_override` is only set inside `serve_export_request`, and `composite_row`'s opaque fast path adds one `d[3] < 255` compare per pixel.

## Risks / notes
- `composite_row` now branches on the canvas pixel's alpha. A normal (opaque-background) preview never takes the new path. A project whose background is `Custom` with alpha < 255 now blends correctly where it used to keep the old maths.
- Quick Export (Ctrl+M) of a *Custom* last export doesn't remember the transparent tick: `ExportPresetRef::Custom` stores only ext and size. The Overlay card and user presets do carry `alpha`.
- The pane's MOV alpha codec choice (`alpha_codec`) isn't saved in presets. A preset with `alpha` on a MOV is always ProRes 4444.
- forum-export (`claude/forum-export-reliability`) also touches export. This branch keeps its changes in its own `ws:gap-alpha-export` sections and functions.
