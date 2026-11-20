# Hand-off: parity-audio (`claude/parity-audio`, new from `origin/develop` at 41c9b73)

Section "parity-audio" of `plans/cloud-handoff.md` (Shotcut parity, `plans/shotcut-comparison.md` § Audio).

## Done

| Item | Where |
|---|---|
| **Audio filters on a single clip** - `Clip.audio_filters` (serde default, skipped when empty), run in the mixer before the clip's volume / pan / fades / transitions, so playback and export share it. Params keyed in clip-local time. Filter state per (clip, depth), dropped the block a clip stops sounding. A clip with no enabled filter takes the old path unchanged. | `model/clip.rs`, `engine/mixer.rs` (`add_clip_audio`, `resample_into`, `gain_add`) |
| Inspector ▸ Audio ▸ **Filters**: an Add menu, then the Mixer's own filter list (refactored into `mixer_ui::filter_list` / `add_filter_menu`, shared by bus strips and clips). One undo per gesture. | `ui/inspector_audio.rs`, `ui/mixer_ui.rs` |
| **Ten filter types** (`FilterKind`, marker `ws:parity-audio`): BandPass, Notch, Expander, Declick, MidSide, StereoWidth, Channels (copy L>R / R>L, swap, invert L / R / both), Downmix, Eq15 (ISO 2/3-octave graphic EQ), Normalize (two-pass). Band-pass / notch / 15-band draw response curves; choice params get a dropdown (`FilterKind::choices`). | `model/audio.rs`, `engine/mixer_fx.rs` |
| **Two-pass normalize**: the filter applies `target - measured` (capped by Max gain; 0 until measured). Its **Analyze** button (or `audio.normalize_loudness`) measures the clip on a worker through the filters ahead of it (`audio_tools::measure_lufs`, BS.1770-shaped `Lufs`), then writes Measured LUFS back as one undo step. | `engine/audio_tools.rs`, `ui/app/tools_audio_parity.rs` |
| **Audio Scopes** pane (`Pane::AudioScopes`): spectrum (31 log bands, falling peaks, hover readout), vectorscope / goniometer, correlation meter, peaks. Audio thread copies each played block into a fixed ring (`audio_tools::SCOPE_FEED`, no allocation after the first block); the pane recomputes only when new frames arrived and clears on stop. Audio page: a tab beside the Mixer; Window menu. | `ui/audio_scopes_ui.rs`, `playback.rs` (2 lines), `ui/layout.rs`, `ui/app/menus.rs` |
| **Track auto-fade and seam**: `Track.auto_fade_ms`, `Track.seam`; track header right-click ▸ **Auto Fade** (10 ms) / **Smooth Seams**. Auto-fade fades every clip edge without a transition or seam; a seam extends both touching clips 5 ms past the cut with a linear cross-fade that sums to unity. Edge gains are evaluated per frame (block-lerped they would not sum to one). `Project::set_track_audio_edges` + `OP_TOOLS` row. | `model/track.rs`, `model/ops/tracks.rs`, `engine/mixer.rs` (`clip_edges`, `edge_gain`), `ui/timeline/{mod,header}.rs` |
| **Tone generator**: `Action::ToneGenerator` (Clip ▸ Add ▸ Tone Generator…, palette): sine / square / sawtooth / triangle (PolyBLEP) / white / pink noise, frequency, level, length -> 48 kHz stereo WAV in `%APPDATA%\SimpleEditor\generated\`, written on a worker, added and placed at the playhead (asset built without a probe: format is known). | `engine/audio_tools.rs`, `ui/app/tools_audio_parity.rs`, `hotkeys.rs` |
| **MCP**: `clip.audio_filter_add`, `clip.audio_filter_set`, `clip.audio_filter_remove`, `clip.audio_filters`, `track.audio_edges`, `audio.normalize_loudness`, `audio.tone` (Job), `audio.scopes` (Read). `audio.add_filter` now takes every kind via `FilterKind::parse` / `AudioFilter::with_params`. | `ui/app/tools_audio_parity.rs`, `ui/app/tools_clip.rs` |
| Docs: CHANGELOG `## Unreleased`, `website/docs/audio.mdx`, tool rows in `docs/customizing.md`. | |

Registries touched, each on its own `ws:parity-audio` line: `TOOL_TABLES`, `ACT_HANDLERS`, `FRAME_HOOKS`,
`PANE_DRAWERS`, `Pane` / `Pane::ALL`, `actions!` + `group()`, `OP_TOOLS`. `MAIN_PANES` grew to 21.

## Verified here (Linux container)

- `cargo check --target x86_64-pc-windows-gnu --all-targets`: clean, no new warnings (only the newer
  rustc's existing "falling back to f32" lint, which the whole crate already shows).
- The crate cannot build or test on Linux (winit refuses the target) and Wine aborts on a hello-world in
  this container. So I compiled the real `model/audio.rs`, `engine/mixer_fx.rs` and
  `engine/audio_tools.rs` into a small Linux harness crate against stubs (`Animated`, `Project` buses,
  `Clip`, `AudioSource`, `fft`) and ran their tests: **43 passed**, including every new filter, the
  existing mixer_fx suite, the tone generator, the spectrum / correlation maths, the scope ring and
  `measure_lufs`.

## Not verified (needs the Windows run)

- Tests written but not run: `engine::mixer` (`clip_filters_run_before_the_clip_gain`,
  `clip_filter_state_is_dropped_once_the_clip_stops_sounding`, `track_auto_fade_dips_at_every_cut`,
  `track_seam_cross_fades_touching_clips_at_unity`), `tools_audio_parity::tests`,
  `audio_scopes_ui::tests::assert_no_idle_repaint_audio_scopes`, the updated
  `layout::tests::the_audio_page_has_every_sound_pane`, `tools_registry_tests`.
- Nothing was rendered or live-checked: the Inspector Filters row, the Audio Scopes pane look, the Tone
  Generator window, the track menu items. Worth a screenshot pass and a `docs-shots.ps1` row for the scopes.
- Perf gate / size gate not run. Expect a modest size increase (one pane, ten DSP kernels, one window).
- `docs/customizing.md` rows were added by hand; regenerate from a live `tools/list` at release.

## Risks / notes

- Per-clip `FilterState::new` allocates on the audio thread the first block a filtered clip sounds (the
  bus graph already does the same). Reverb is the biggest (~100 KB).
- Clip filter state is keyed by (clip id, nesting depth): two instances of the same nested sequence
  sounding at once would share one filter state for their inner clips.
- A filter tail (reverb / echo) ends with the clip; Shotcut does the same.
- Analyze works on audio clips only (not a Sequence clip, not a bus); on a bus Normalize, the button
  explains that and Measured LUFS can be typed by hand. Analyze again after changing earlier filters.
- `Track.seam` reads the source a few ms past each clip's out point (like a transition does); at the
  very end of a file that is silence, so the seam dips there.
- The scope ring is fed at mix time, which runs `LEAD_SECS` (0.12 s) ahead of what is heard - the same
  lag the Mixer meters have.
- `website/docs/shortcuts.md` (generated) does not list Tone Generator yet: re-run `--dump-hotkeys`.
