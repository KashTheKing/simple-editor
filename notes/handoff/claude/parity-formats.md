# Hand-off: parity-formats (`claude/parity-formats`)

Section "parity-formats" of `plans/cloud-handoff.md`, branched from `origin/develop` at `41c9b73`.
No new crates. Everything slow runs on job threads (`export::spawn_job`); `formats_ctl::tick` lands results.

## Done

| Item | Where | How |
|---|---|---|
| ASS/SSA import | `engine/subtitles.rs` (`is_ass`, `parse_ass`, `ass_text`, `read_file`, `IMPORT_EXTS`) | `parse()` auto-detects `[Script Info]`/`[Events]`; `Format:` column order honoured; comma-safe Text; override blocks and `\p` drawings stripped; `\N` kept. UTF-16 (BOM) and ANSI files decode. Pickers, `subtitles.import`, the player's side-car list all take .ass/.ssa. |
| Extract subtitles from a media file | `ui/app/formats_ctl.rs` (reuses `player::tracks::probe/cues`) | Subtitles ⋯ ▸ **Import from Media File…**: probe on a job thread, one text track is read at once, several open a small chooser window. Lands through `timeline_pane::offer_cues` (split out of `import_subtitles`: replace inline, or confirm). MCP `subtitles.tracks`, `subtitles.extract`. |
| Soft subtitle tracks in MP4/MKV export | `engine/formats.rs` (`mux_args`, `embed_subtitles`, `cues_in_range`), `files.rs` hooks, `export_ui.rs` checkbox, `Settings.export_subtitle_track` | A **remux after the export** (stream copy), so `ExportOptions` / the export pipeline are untouched (no overlap with gap-alpha-export / forum-export). mov_text in MP4/MOV, SubRip in MKV, WebVTT in WebM; an old subtitle track is replaced, never stacked. In/Out range exports get shifted cues. Subtitles ⋯ ▸ **Embed in Video File…** writes `<name> (subtitled).<ext>`. MCP `subtitles.embed`. |
| AVIF | `media/mod.rs` `is_image_path`, `ffpipe.rs` probe, `folder_import::MEDIA_EXTS` | ffmpeg decodes it; an animated AVIF (nb_frames > 1) is a video, like GIF. |
| SVG | `engine/formats.rs` (`svg_size`, `svg_page`, `svg_to_png`), `formats_ctl::intercept` in `import_files` | Drawn to a PNG by the HTML layer renderer (Edge forced) at its natural size fitted in 4K, written to `Generated\` beside the project (else `%APPDATA%\SimpleEditor\Generated`), then imported. Only the Import dialog offers .svg (not folder import / linked bins, which would ffprobe it). |
| Lottie | `engine/lottie.rs` (native renderer), `engine/synth.rs` (`encode_lottie`, `lottie_alpha`), `formats_ctl::start_lottie` | Shape/solid/null/precomp layers, parenting, groups, path/rect/ellipse/star, fill (non-zero/even-odd), stroke, linear/radial gradient fill+stroke, trim paths, linear/bezier/hold/spatial keyframes, old `s`/`e` keyframes. Rendered to `<name>.stacked.mp4` (transparent) / `.mp4` (opaque) / `.png` (still). `.json` is sniffed by content; `.lottie` is unzipped with Windows' `tar`. Toast names what it was drawn without (masks, mattes, text/image layers, repeaters, merge paths, effects, expressions, time remap, 3D). MCP `media.import_lottie`; `media.import` and drops take these files too. |
| Rive | not done | No permissive runtime fits: rive-runtime is C++ (needs a C++ build in build.rs) and the Rust `rive-rs` pulls in vello/wgpu (several MB, a second GPU stack). Revisit as an optional helper exe like `se-html`. |
| Generators | `engine/synth.rs`, `formats_ctl::start_generate`, Library right-click ▸ **Generate** | Colour Bars (SMPTE ECR-1-style, a PNG still), Noise (seeded per frame, grey or colour), Plasma, Counter (digits pre-rendered once by the app's `TextRasterizer`, Consolas bold, a fifth of the frame high, steady digit cells; transparent by default). Land in a "Generated" bin at project size/fps. MCP `media.generate` (duration, size, fps, colour, speed, down, start, format clock/precise/seconds/frames, color, background, seed). |
| Stacked-alpha probe | `media/mod.rs` `probe` | A `.stacked.mp4` now probes at half its coded height (what `StackedAlpha` decodes to). Only made files have that name; html renders set their own size. |

Docs: CHANGELOG (Unreleased ▸ Formats), `docs/customizing.md` tool tables, `website/docs/captions.mdx`,
`library.mdx`, `export.mdx`.

## Verified here (Linux container)

- `cargo check --target x86_64-pc-windows-gnu --all-targets`: clean, no warnings in the new/changed code.
- The crate can't build or run on Linux (winit), and Wine is broken in this container (`free(): invalid
  pointer` on any exe). So the pure modules were tested by `#[path]`-including them in a scratch crate with
  stubs: **31 tests pass** - all of `engine::subtitles` (old + new), `engine::formats`, `engine::lottie`,
  `engine::synth`. Two of them ran **real ffmpeg 6** (Ubuntu): MP4 gets a `mov_text` track and MKV a
  `subrip` one, re-embedding keeps one track, no temp files left; plasma / bars PNG / stacked counter encode.
- Real Lottie files (lottie-web `adrock`, lottie-ios `LottieLogo1`, lottie-android `Shapes`) rendered and
  looked at as contact sheets: correct shapes, strokes, trims, gradients, parenting; 1-4 ms per frame at
  345x456. `adrock` (390 frames) encodes in 6.6 s. A stacked encode's raw frame has premultiplied colour on
  top and the alpha mask below, as `StackedAlpha` expects.
- AVIF probe logic checked against real ffprobe output (still: nb_frames 1, no duration; animated: 10).

## Not verified (needs Windows)

- None of the `ui::app` tests ran: `formats_ctl` tests (`assert_no_idle_repaint_subtitle_tracks`,
  `only_text_tracks_are_offered`, `lottie_files_are_recognised_by_content`, `srt_of_numbers_from_one`,
  `tools_are_registered`), plus whatever existing tests count Library / Subtitles menu rows or the
  tool table (`tools_registry_tests`, docs tool counts). Run `cargo test formats` and the full suite.
- Live: Import from Media File on an MKV with 2+ tracks (chooser) and an MP4 with mov_text; Export with
  Subtitle track on (and Quick Export); Embed in Video File; an SVG import (Edge cold start ~1.3 s);
  a Lottie drop; each Generate item, especially **Counter** (glyphs come from `TextRasterizer`; if it crops
  glyphs differently than assumed, digits may sit unevenly - `synth::compose_text` centres each glyph and
  puts '.' on the digits' baseline).
- WIC thumbnails for AVIF fall back to ffmpeg when Windows lacks the AV1 extension (existing path).
- Size gate not run (no release build here). Expect roughly +80-150 KB (lottie.rs is the bulk).

## Risks / notes

- Subtitle track on export renames over the exported file a moment after "Export finished"; if something
  opens the file in between (Quick Export's open-in-player), the rename can fail -> an error toast, the
  export itself is intact.
- A dropped SVG / Lottie lands in the Library, not on the timeline at the drop point (it is converted first).
- Soft subtitle language is tagged "und" (no project language setting exists).
- Lottie: no masks / mattes is the main visible gap on After Effects exports (backgrounds meant to be
  masked show whole, as in `adrock`'s panel).
- Shared files touched (small hunks, marked `ws:parity-formats`): `ui/app/mod.rs` (mod, App field,
  TOOL_TABLES / FRAME_HOOKS / WINDOW_DRAWERS rows, the import funnel gate), `files.rs` (export start/finish
  hooks, media dialog extensions), `panes.rs`, `library_pane.rs`, `jobs_pane.rs`, `timeline_pane.rs`
  (`offer_cues` split), `subtitles_ui.rs`, `library.rs`, `export_ui.rs`, `settings.rs`, `media/mod.rs`,
  `media/ffpipe.rs`, `player/tracks.rs`, `engine/folder_import.rs`, `engine/mod.rs`.
