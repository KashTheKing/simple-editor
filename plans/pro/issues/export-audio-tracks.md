# Export separate audio tracks (streams in file, WAV stems)

**Workstream:** `export-audio-tracks` · **Issue:** [#147](https://github.com/KashTheKing/simple-editor/issues/147) · **Plan:** [plans/pro/README.md](../README.md) · **Wave:** 2 · **Coding batch:** B · **Branch:** `feat/pro`

Read [agents.md](../../../agents.md), [goals.md](../../../goals.md), [notes.md](../../../notes.md), [ARCHITECTURE.md](../../../ARCHITECTURE.md) and the master plan [README.md](../README.md) before starting: its findings table gives the file:line evidence, and its "Execution" section gives this workstream's file ownership and the rules for the first coding pass.

## Scope

- **Stems = one mix pass per group** on a cloned project with only that group audible (reuses
  `mix_to_wav` unchanged), groups run on parallel threads.
  `// ponytail: N passes decode each source N times; single-pass multi-destination mix_tracks if
  audio time ever matters`. A stem is post-fader through its bus chain, clamped per stem.
- Grouping: by **track** (default, named from `Track.name`), by **bus**, or by **role**
  (`AudioRole`: Dialogue / Music / SFX / Ambience).
- **Streams mode**: stream 0 is the full mix (default disposition), then one stream per group:
  `-i stemN -map N:a -metadata:s:a:N title=<name>`; loudnorm only on the mix
  (`-filter:a:0`). Offered for mp4/mov/mkv; greyed out with a reason elsewhere.
- **Stems mode**: `<output name> stems/NN <name>.wav`, 24-bit 48 kHz, beside the video.
- Export pane: an "Audio" row (Mix / Separate tracks in file / WAV stems + group-by);
  `export.video` gains `audio` and `stem_by` args (ToolDef row updated); size estimate counts
  streams (`estimate_export_bytes`).
- 5.1 fix: sources with more than 2 channels get a proper downmix instead of L/R only
  (`mf.rs:766`; simplest: route those files through the ffmpeg audio decoder, which uses `-ac 2`).


## Files owned

new `src/engine/stems.rs`, `src/engine/mixer.rs`, audio inputs/maps in `run_export`, `estimate_export_bytes`, the Audio row in `src/ui/export_ui.rs`, `export.video` args in `src/ui/app/tools_export.rs`, the >2-channel downmix in `src/media/mf.rs`

## Done when

- Everything under Scope is implemented, or the gap is named in the PR/report with a reason.
- Structural rules hold: ToolDef row for every new Action / Pane / `pub fn(&mut self)` on Project, an `assert_no_idle_repaint_<context>` test for every new pane or window, no new crates, menus through `ui::menu::*`.
- Verification from the master plan's "Verification" section passes for wave 2 (run later, on a quiet machine: `cargo test`, `--selftest`, a viewed `--screenshot`, `scripts/size.ps1 -Note export-audio-tracks`).
