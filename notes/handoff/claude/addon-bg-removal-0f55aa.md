# Hand-off: addon-bgremove (`claude/addon-bg-removal-0f55aa`)

## Done

- Merged `origin/develop` (clean, no conflicts) into the paused WIP.
- The WIP was already complete in scope and compiled once merged:
  - `engine::bgremove`: install on demand from `library/bin/bgremove.json` (helper, ONNX Runtime 1.22 DirectML
    from NuGet, MODNet = Fast, BiRefNet lite = Best), checksum-verified; the matte cache (format in notes.md
    "bg-removal"); the job (ffmpeg → `se-matte` stdin → mattes), 3-frame median temporal smoothing; Edge
    (shrink / feather) and Show Matte, applied on both the GPU (`gpu::draw_clip`) and CPU (`compose.rs`) paths.
  - `tools/se-matte`: the helper (separate crate, `ort` `load-dynamic`, DirectML with CPU fallback).
  - UI: clip menu ▸ Remove Background ▸ Fast / Best / Off, palette actions, the Inspector's Remove Background
    section, Settings ▸ General ▸ Background removal row, Jobs pane rows with cancel, toasts.
  - MCP `clip.remove_background` (status / install / uninstall / off / edge / show_matte / ep).
  - Docs page `website/docs/background-removal.mdx`, CHANGELOG under `## Unreleased`.
- New in this session (`dc5ec73`):
  - **UI-thread fix:** the Inspector called `bgremove::coverage` every frame, which lists the matte folder
    (thousands of PNGs for a long clip) on the UI thread. It now uses `clip_coverage_cached`: the count runs
    on a short-lived thread (refreshed every 400 ms while a job writes, 3 s otherwise) and requests a
    repaint when it lands. MCP status still counts directly. Test:
    `the_inspector_count_lands_off_the_ui_thread`.
  - Removed the unused `bgremove::addon_status` (dead-code warning).
  - Settings' Best size tooltip said 225 MB; the install offer says 240 MB (helper + runtime + model). Both
    say about 240 MB now.

## Verified here

- `cargo check --target x86_64-pc-windows-gnu --all-targets`: passes, no warnings in the add-on's files.
- `tools/se-matte`: `cargo check --target x86_64-pc-windows-gnu` passes.
- `cargo test --target x86_64-pc-windows-gnu --no-run`: the test binary builds.
- The NuGet runtime download's SHA-256 matches the manifest.

## Not verified (coordinator, on Windows)

- **No test ran.** The crate doesn't build for Linux (winit), and Wine in this container aborts even on a
  hello-world exe (`free(): invalid pointer`). Run `cargo test bgremove` (16 tests, 2 `#[ignore]`d:
  the real-model run and the bench).
- **The helper release is not published:** `releases/download/bgremove-v0.1.0/se-matte.exe` returns 404, so
  every install fails until it exists. Build it (`tools/se-matte/README.md`), publish
  `bgremove-v0.1.0`, and make sure the `helper` part's size / SHA-256 in `library/bin/bgremove.json` match the
  uploaded exe (they were filled in from the local build before the pause).
- The Hugging Face model URLs are blocked by this container's proxy, so their checksums weren't rechecked.
- Live check: Remove Background on a video and a picture (Fast and Best), Edge drag (one undo step), Show
  Matte, cancel a job, a paused preview picking up new frames, playback with a keyed clip (no stutter),
  export with a keyed clip, Chroma Key / mask on top of a keyed clip, the ignored real-run test and bench.

## Risks

- DirectML can run out of GPU memory for BiRefNet at 1024² on the first inference (notes.md gotcha 1); the
  helper falls back to the CPU in `--ep auto`, which is slow for long clips.
- MODNet is portrait-only: on objects it finds little. The tooltips steer users to Best for objects.
- `Cmd::Mattes` evicts every span that uses the source, but only while paused; while playing, new frames show
  once the read-ahead reaches them.
