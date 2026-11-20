# Handoff: mixed-footage (`claude/feat-mixed-footage-83d1e8`)

Merged `origin/develop` (clean, no conflicts), made the paused work compile, and finished the scope.

## Done

- **Batch-wide project match** (`ui/app/project_match.rs`, `media/footage.rs`): every video in an import counts,
  not just the first. A mixed batch picks the frame rate the others divide into evenly (25 for 25 + 50, 30 for
  30 + 60, 24 for 24 + 48, 29.97 for 29.97 + 59.94). A mix that doesn't divide picks the most common rate and
  shows "some clips will drop or repeat frames". The card has 1080p / 4K delivery buttons, the reasoning is in
  its tooltip, and Match is one undo step.
- **Every-frame slow motion** (`ui/inspector_footage.rs`, `ui/timeline/menus.rs`, `ui/timeline/mod.rs`): a rate
  badge ("50") on clips whose rate is a whole multiple of the project's, right-click ▸ Speed ▸ Slow motion
  (every frame), and an Inspector **Preset** row (Normal / Every frame / Custom).
- **Frame rate** row for odd mixes: Frame sampling (default), Frame blending (the existing Frame Blend effect),
  or Optical flow, an offline `minterpolate` bake (`BakeKind::Flow` in `tools_export.rs`) that renders the clip
  at its source rate first. Nothing runs on the playback path.
- **Zoom with a 1:1 mark**, a **Max sharp** row, and an **Upscaled** hint with an AI Upscale button when the enhancer is installed.
- **Proxy offer** for 4K long-GOP / 10-bit HEVC that wouldn't get a proxy anyway. The answer is remembered
  (`Settings.footage_proxies`), and **Settings ▸ Performance ▸ Heavy footage** (Ask / Make proxies / Play
  originals) changes it (added this session so a "No" can be undone).
  - **For forum-preview:** call `App::offer_proxies(&[asset ids])` (`footage_ctl.rs`) instead of drawing a
    second card. It honours the remembered answer and skips sources that already have a proxy or are already
    waiting. `footage_ctl::heavy(&Asset)` is `pub(crate)` too.
- **Camera detection and colour match**: make/model from ffprobe tags (QuickTime, Android, plain, the XAVC
  brand, GoPro/DJI handlers), cached on the import probe so nothing probes on the UI thread. Two or more
  cameras on the timeline offer "Match colours between cameras" with a reference picker (runs `color.match`,
  one undo step, never automatic).
- **Log / HLG LUT offers**: S-Log3, **S-Log2** and **untagged HLG** (added this session) each get a generated
  33³ `.cube` installed into the library on first use, put first on the source's effects, once.
  - Tagged HLG (`arib-std-b67`) is already tone-mapped on decode, so it gets no offer.
- **Docs**: `website/docs/mixing-cameras.mdx` (in the sidebar), a pointer from `project-settings.mdx`, and CHANGELOG lines under `## Unreleased`.

## Verified here

- `cargo check --target x86_64-pc-windows-gnu --all-targets`: clean, with no warnings from the touched files.
- The pure `media/footage.rs` tests, compiled natively in a scratch crate together with `engine/lut.rs`'s parser
  (6/6 pass): batch rate choice, rate multiples, tag parsing, all three LUTs (grey near 42 %, neutral, black
  stays black, white reaches white) and the HLG tagged/untagged rule. The LUT curves were also checked
  numerically in Python.

## Not verified (needs Windows)

- The crate's own test binary. Linux can't build it (winit refuses the platform). It links for
  windows-gnu, but Wine 9 in this container crashes even on a hello-world exe. Run on Windows:
  `cargo test footage project_match inspector_footage timeline_clips_group conversion_lut`.
  - `camera_model_is_read_from_generated_files` needs ffmpeg/ffprobe.
- Nothing was rendered: none of the cards, Inspector rows, the timeline badge, the Zoom 1:1 tick position or the Settings row. Take screenshots before merging.
- Optical-flow bake end to end (the clip should swap onto the result at the right length).

## Risks

- `footage_ctl::poll` runs every frame from the `project_match` window drawer. It's one hash over the
  timeline's video clips and requests no repaint. Still, run the idle-repaint tests.
- The Zoom row writes `clip.scale`, the same field as the Transform Scale row, so the two can show the same value twice. Decide if that's acceptable.
- The S-Gamut→709 and Rec.2020→709 matrices are the standard published ones. The highlight shoulder is a taste
  choice shared by all three LUTs.
- No MCP tools were added. The scope didn't ask for any, and no `Project` `pub fn` was added, so
  `every_edit_op_has_a_tool` is unaffected.
