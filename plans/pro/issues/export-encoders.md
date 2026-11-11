# Encoders: probed hardware encoders, auto pick, smart render

**Workstream:** `export-encoders` · **Plan:** [plans/pro/README.md](../README.md) · **Wave:** 2 · **Coding batch:** B · **Branch:** `feat/pro`

Read [agents.md](../../../agents.md), [goals.md](../../../goals.md), [notes.md](../../../notes.md), [ARCHITECTURE.md](../../../ARCHITECTURE.md) and the master plan [README.md](../README.md) before starting: its findings table gives the file:line evidence, and its "Execution" section gives this workstream's file ownership and the rules for the first coding pass.

## Scope

- **Probe once, cache**: test-encode one tiny frame per hardware encoder
  (`ffmpeg -f lavfi -i color -frames:v 1 -c:v <enc> -f null -`), cache the working set keyed by
  ffmpeg path + GPU name. Encoders that cannot run are hidden instead of failing the export.
- **"auto" picks the fastest working encoder** (NVENC, then QSV, then AMF, then libx264), with a
  visible "Speed / Balanced / Smallest file" choice mapping to encoder presets (NVENC `p1/p4/p7`,
  x264 `veryfast/medium/slow`). Quality stays CRF-style (`-cq` / `-global_quality` / `-crf`).
- `*_mf` encoders get real rate arguments or are hidden.
- **Smart render (last, optional, behind its own toggle)**: extend `lossless_segments`
  (`export.rs:611-742`) from all-or-nothing to per-span: stream-copy untouched GOPs, re-encode
  only the spans an edit touches, concat. Only for H.264/HEVC sources whose parameters the
  re-encode can match, and every result is checked with a decode pass
  (`ffmpeg -v error -i out -f null -`); any error falls back to a full encode.


## Files owned

new `src/engine/encoders.rs`, `codec_args` / `detect_encoders` / `lossless_segments` in `src/engine/export.rs`, the Advanced grid in `src/ui/export_ui.rs`, `encoder_options` in `src/ui/mod.rs`

## Done when

- Everything under Scope is implemented, or the gap is named in the PR/report with a reason.
- Structural rules hold: ToolDef row for every new Action / Pane / `pub fn(&mut self)` on Project, an `assert_no_idle_repaint_<context>` test for every new pane or window, no new crates, menus through `ui::menu::*`.
- Verification from the master plan's "Verification" section passes for wave 2 (run later, on a quiet machine: `cargo test`, `--selftest`, a viewed `--screenshot`, `scripts/size.ps1 -Note export-encoders`).
