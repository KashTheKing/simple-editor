# Hand-off: gap-pitch-speed (`claude/gap-pitch-speed-f4c741`)

Cloud session, 2026-11-20. Scope from `plans/cloud-handoff.md`: pitch-preserving clip speed with a
"Keep pitch" toggle, audible from 0.25x to 4x, in playback, export and bakes; no GPL; proven by a sine test.

## Done
- Merged `origin/develop` (clean, no conflicts).
- The paused WIP (`315a074`) was complete in scope. It compiles for Windows unchanged:
  - `engine::stretch` (WSOLA, no new crate).
  - `Mixer` stretches every Keep pitch clip, which covers playback, scrubbing, export and mixer-based bakes.
    Sequence clips, ramps and reverse are included.
  - The JKL shuttle stretches from 0.25x to 4x.
  - `Clip.keep_pitch`: on for new clips; old projects load with it off and save unchanged.
  - Inspector tick, `clip.set {fields:{keep_pitch}}`, `timeline.list` field, CHANGELOG and docs.
- Added `stretch_is_clean_from_quarter_to_4x`: a windowed Goertzel check. For 150/440/1000 Hz at
  0.25/0.5/0.8/1.25/2/3/4x, every ~170 ms window keeps at least 98.4 % of its energy at the tone, and less
  than 20 % at the pitch a resample would give. The zero-crossing test alone could not show seams.
- Fix: the **Slow motion bake** (`export.slowmo`) slowed only the picture. A swapped audio clip played
  full-speed sound from the wrong offset. It now adds `atempo` at the same factor through ffmpeg.exe, run as
  a separate process like `afftdn`. Below 0.5x it chains `atempo=0.5` steps (`convert::atempo_chain`).
- Fix: `swap_clip_asset` (every bake) rebuilt clips with `Clip::new`, which turned Keep pitch on for old
  clips. It now copies the setting across.
- Removed a run of stray spaces from the Keep pitch tooltip.

## Verified here
- `cargo check --target x86_64-pc-windows-gnu --all-targets`: builds. There are no new warnings in touched
  files. The `inspector_audio.rs:177` f32 fallback warning comes from develop.
- `engine::stretch` tests: all 3 pass. They ran in a standalone Linux harness that compiles `stretch.rs`
  with a stubbed `Clip`/`SAMPLE_RATE`.
- `atempo_chain` against real ffmpeg: a 2 s 440 Hz sine becomes 7.95 s at 440.1 Hz (0.25x) and
  3.98 s at 440.0 Hz (0.5x).

## Not verified (please run on Windows)
- The app's own test binary could not run here. The crate needs winit/Windows, and Wine 9.0 in this container
  aborts on any Windows exe ("free(): invalid pointer"). So none of these have ever run:
  - the mixer tests `keep_pitch_stretches_the_sound`, `keep_pitch_same_sound_in_any_block_size`,
    `keep_pitch_mutes_above_4x` and `old_projects_load_without_keep_pitch`
  - the edited `speed_reverse_freeze` and sequence speed tests
  - `bake_filters_build_expected_args` and `swap_clip_asset_flattens_onto_the_new_asset`
  Run `cargo test keep_pitch stretch bake_filters swap_clip_asset speed_reverse` first, then the full suite.
- Live checks:
  - a 50 % and a 200 % clip with Keep pitch on and off
  - a speed ramp
  - reverse
  - J/L shuttle at 0.25x / 2x / 4x
  - export of a stretched clip
  - Slow motion bake of a video with its linked audio selected
- Perf gate: the stretch adds a few ms of audio-thread work per second of each stretched clip. Run
  `scripts/perf-bench.ps1 -Synthetic` with a Keep pitch clip at 0.5x.
- Size gate: no new crate. The code is ~250 lines, so expect well under +64 KB.

## Risks
- Two instances of the same nested sequence playing at once share one grain cache per inner clip, keyed by clip
  id. The chosen grains still agree, but the source cache re-reads (decoder seeks) each block. This is rare.
- Editing a ramp key keeps the stretch signature. For about 30 ms the old grain choices are used. This is
  intended (see the `ponytail` comment in `stretch::sig`).
- The standalone media player (`player/engine.rs`) still mutes above 2x and resamples. It is out of scope;
  see notes.md.
- For time-remap (`claude/feat-time-remap-8a3a4e`): drive audio through `stretch::stretches` and keep
  `clip.src_time` as the mapping. Then remapped clips stretch with no extra code.
