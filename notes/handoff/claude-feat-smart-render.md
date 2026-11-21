# claude/feat-smart-render

Smart render: saving or exporting a plain cut at the source's own size, frame rate and codec copies whole GOPs and
re-encodes only the partial GOP at each cut. It is the default; the old `-c copy` cut stays as "Lossless (keyframe cuts)".

## Done
- `src/engine/smart_render.rs` (new): probe + cache (size + mtime), open-GOP-safe keyframe list, span → part plan with
  exact frame counts, matched encoder args (H.264 → libx264, HEVC → libx265, else the probed hardware encoder of the
  same family), parts run 4 at a time, concat join with audio, verification.
- `src/engine/export.rs`: `smart_segments` (eligibility), `start_best_export`, `start_smart_export` (automatic
  fallback to `run_export`), `Progress::note` / `set_note` / `shortcut`, `cut_audio_streams` (shared with
  `run_lossless`), `#[allow(dead_code)]` removed from the planner.
- Wired in: `files.rs` (Export window and Save over original), `mcp_exec.rs` (MCP export), `finish_export` toasts the
  note, `export_fix::succeeded` skips the speed learning for a smart render.
- Labels: Settings ▸ General and the Export window's checkbox are now "Lossless (keyframe cuts)" with the explanation
  in the tooltip.
- Selftest step `smart-render`; CHANGELOG and notes.md entries.

## Verified here (Linux container)
- `cargo check --target x86_64-pc-windows-gnu --all-targets`: clean, no new warnings (313 before and after).
- The crate can't build or test on Linux, so `smart_render.rs` was compiled in a scratch crate with shims for its
  few imports (Progress / TempFile / wait_ffmpeg copied from export.rs, ffpipe without the Windows flag) and its
  8 tests ran against ffmpeg 6.1.1, all passing:
  - unit: packet parsing, open-GOP rejection, part frame counts, encoder matching / refusal, audio codec mapping,
    ffprobe JSON parsing.
  - `smart_render_real_is_frame_accurate_and_encodes_only_the_cut_gops`: testsrc 1280x720 @ 60, GOP 2 s, 40 s;
    trim 3.5–31.75 (MP4) and two pieces (MKV). Exact frame count, first pts 0, last pts (n-1)/60, even spacing,
    min per-frame PSNR vs the source 57 dB / 52 dB (copied frames are bit-exact), 135 / 195 frames encoded,
    1.6 s / 2.8 s vs 9.2 s for a full ffmpeg re-encode of the range.
  - `smart_render_refuses_what_it_cant_match_and_leaves_the_output_alone`: MPEG-4 Part 2 source, WebM output, a
    missing audio stream; the existing output is untouched and no temp files are left.
- The user's case, simulated: 132 s 1920x1080 @ 60 with a 2 s GOP, trimmed at both ends: 3.9 s, 120 frames
  encoded, 7080 copied, exactly 7200 frames out, vs 102 s for an ffmpeg-only full re-encode (the app's full export
  is slower still).

## Not verified (needs Windows)
- `cargo test smart` (export.rs: `smart_segments_only_when_the_output_is_the_source_as_is`, `smart_export_real`,
  `smart_export_falls_back_to_the_full_reencode`) and `cargo test smart_render` (the 8 above, in the real crate).
- `cargo run -- --selftest` (new `smart-render` step).
- Live: Ctrl+S over a real recording (OBS / NVENC / phone), then play the result in the app (Media Foundation) and in
  Windows' own player across the joins. Check the Settings row and Export checkbox render and read well.

## Risks
- Joined parts carry different SPS/PPS in-band (x264's next to the source encoder's). FFmpeg decodes them cleanly
  and the join check uses it; Media Foundation should too, but that is the one thing to watch on Windows. If MF
  shows corruption right at a cut, the fix is to put the parameter sets for both into the sample entry or to tag
  `avc3`/`hev1`.
- Single-piece saves copy the audio, so it can run up to one AAC frame (21 ms) past the video at the end; several
  pieces re-encode the audio (same codec, source bitrate) to keep it sample-accurate.
- Encoder "auto" counts as "the source's codec", so an HEVC source keeps HEVC on Save; an explicit encoder of another
  family (or a different size / rate / loudness / range / target size / stems / chapters) uses the full re-encode.
- Interlaced, VFR without timestamps, 12-bit and codecs other than H.264 / HEVC fall back to the full re-encode.
