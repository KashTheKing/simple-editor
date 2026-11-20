# Hand-off: addon-sam2 (`claude/addon-sam2-b6943f`)

SAM 2 object masks as an optional add-on: click points in the Preview, Track through the clip, a cached
mask sequence with a CPU fallback, MCP `mask.object` / `mask.track` (+ `mask.install`).

## What's done

The paused WIP already held the whole scope; this session merged `origin/develop`, got it compiling and
closed the gaps found on review.

- **Merge `origin/develop`** (`Merge origin/develop into addon-sam2`). One conflict, `src/engine/compose.rs`
  (3 hunks): develop added a `crop` argument to `draw_layer_masked`, the branch widened `clip_mask`'s
  placement to `(cx, cy, w, h, rot)` for object masks. Kept both.
- **Already on the branch (from the WIP commit), reviewed, unchanged:**
  - `src/engine/sam2.rs`: install from `library/bin/sam2.json` (SHA-256 checked, resumable, swapped in
    atomically), the long-lived `se-sam2.exe` child process (JSON lines, below-normal priority while
    playing), `segment` (one frame), `track` (the whole clip into `cache/masks/<key>/NNNNNN.png` +
    `mask.json`), and the render-side cache: during playback nothing decodes on the playback thread
    (nearest decoded frame + a background decode queue); off playback it decodes the exact frame.
  - `src/engine/gpu/object.rs` + `gpu.rs`: the object mask is applied in layer space on the GPU; effect
    and node masks go through `layer_mask`. CPU twin in `compose.rs` (`coverage_canvas` / `coverage_layer`).
  - `src/ui/app/object_mask_ctl.rs`: click mode over the Preview (left = add, Shift = not the object,
    right = remove nearest, Esc/Done leaves), live tinted mask, Track as a job in the Jobs pane with
    cancel, toasts, and the MCP tools `mask.object`, `mask.track` (`action=status` too), `mask.install`.
  - Entry points: Inspector ▸ Mask ▸ Object Mask, the clip menu's Mask submenu, actions `object_mask` /
    `object_mask_track` (palette), Settings ▸ General ▸ Object masks (Install / Uninstall).
  - `MaskShape::Object` + `Mask.matte` (skipped in JSON when empty, so old projects are unchanged).
  - `tools/sam2-helper` (own crate, never in the editor's build), `scripts/build-sam2.ps1`,
    `website/docs/masks.mdx`, CHANGELOG under `## Unreleased`.
- **Fixed this session** (`fix(object-mask): …`):
  - Idle repaint: the frame hook asked for a frame every 150 ms for as long as click mode was open and
    the helper had not reported a device, which is forever after a failed warm-up. The session now holds
    the warm-up job; the busy check is a pure `wants_frames` with a new
    `assert_no_idle_repaint_object_mask` test. A failed warm-up shows its reason on the strip.
  - House rule (a button that toggles is lit while on): Inspector's Object Mask button is
    `.selected(...)` while click mode is on for that clip (`ui::app::object_mask_clip` re-export at the
    `ws:addon-sam2` line in `src/ui/app/mod.rs`).
  - The Inspector tooltip had ~35 spaces embedded twice (a string broken across lines without `\`).
- **Tests added** for the helper's pure logic (`tools/sam2-helper`, 3 tests: logits→mask resize, point
  parsing/clamping, PNG frame write + area). `#[cfg(test)]` only: the shipped exe and its checksum in
  `sam2.json` do not change.

## Verified here (Linux container)

- `cargo check --target x86_64-pc-windows-gnu --all-targets`: clean for the editor (0 errors; the only
  warnings in the new files are dead code: `AddonState`/`addon_state`, left for a shared add-ons list, and
  `iou`, used by tests).
- `cargo check --target x86_64-pc-windows-gnu --all-targets` for `tools/sam2-helper`: clean.
- `cargo test` in `tools/sam2-helper` on Linux: 3/3 pass.
- `cargo test --target x86_64-pc-windows-gnu --no-run` links the editor's test exe with mingw.

## Not verified (needs the Windows box)

- **No editor test ran.** The crate is Windows-only, and Wine in this container crashes on a mingw
  "hello world" (`free(): invalid pointer`). Run: `cargo test sam2`, `cargo test object_mask`, then the
  full suite. `sam2_real_run_tracks_a_moving_box` needs a real install (`SE_SAM2` pointing at a built
  helper with its DLLs and models beside it).
- Nothing was run live: click mode, the overlay mapping on a scaled/rotated clip, Track, the GPU vs CPU
  render of an object mask (compare the Preview to an export), Esc in click mode.
- The install from the real manifest.

## Risks / for the coordinator

1. **The helper release does not exist yet.** `library/bin/sam2.json` points `se-sam2.exe` at
   `releases/download/se-sam2-v0.1.0/se-sam2.exe`; there is no such tag on origin. Until it is published,
   Install fails at the first part. Steps: `scripts/build-sam2.ps1` (it rewrites the exe's size and
   SHA-256 in the manifest; the current values came from the earlier local build), commit the manifest,
   upload `dist/sam2/se-sam2.exe` to that release. Cloud sessions may not make tags or releases, so it
   is yours.
2. **DirectML licence.** `DirectML.dll` (fetched from NuGet at install, never linked or shipped in the
   exe) is under Microsoft's redistributable terms, not an open-source licence. Everything else is
   MIT/Apache. The house rule says "only permissively licensed code and models"; confirm that a
   download-at-install redistributable is acceptable, or drop it and go CPU-only (remove the DirectML
   part from the manifest; the helper already falls back to the CPU).
3. **CPU compositor cost.** `coverage_canvas` walks every canvas pixel with a bilinear lookup for an
   object-masked layer. It runs only on the CPU path (export fallback / no GPU), not on the GPU preview,
   but watch it in the perf gate if a CPU-path bench covers masks.
4. The first click on DirectML compiles the decoder (measured 48 s on an RTX 2060 Super); the helper keeps
   the decoder on the CPU by default (`SE_SAM2_CPU_GRAPHS=dec`), 1.4 s. Worth a look in the live check.
