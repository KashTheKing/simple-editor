# addon-voice: `claude/addon-voice-tts-dc9111`

Cloud session, 2026-11-20. Scope: `plans/cloud-handoff.md` ▸ addon-voice.

## Done
- Merged `origin/develop` (clean, no conflicts) into the paused WIP.
- The WIP compiles for `x86_64-pc-windows-gnu` (`cargo check --all-targets`: 0 errors, no new warnings in the
  voice files). What it contains:
  - `src/engine/tts_addon.rs`: Kokoro-82M (Apache) through sherpa-onnx's `sherpa-onnx-offline-tts.exe`, and
    Chatterbox (MIT, ONNX export) through a downloaded embeddable Python + onnxruntime-directml
    (`assets/tts/chatterbox_runner.py`). Pinned manifests with SHA-256 in `library/bin/kokoro.json` and
    `chatterbox.json`; install into `%APPDATA%\SimpleEditor\bin\<engine>`, one download at a time, cancellable.
  - `src/engine/tts.rs`: `Engine` (Windows / Kokoro / Chatterbox), `Speech`, cached WAVs, speed + pitch in one
    ffmpeg pass, cloned voices in `%APPDATA%\SimpleEditor\voices`, SAPI fallback with a visible note.
  - Voiceover pane ▸ Speech (`ui/speech_ui.rs`, `ui/app/speech_ctl.rs`): engine, voice + Play preview, Speed,
    Pitch, Intensity, Clone, a script box (`0:12` lines placed at that time), Speak at playhead, Speak selection.
    Speech goes onto a "Voiceover" audio track, one job at a time.
  - Settings ▸ General: Install / Remove rows. MCP: `tts.speak` options (engine, voice, speed, pitch,
    intensity, clone_ref), new `tts.voices`, `tts.status`, `tts.install`.
- Added in this session:
  - Cloning needs a ticked "I have permission to use this voice" box (cleared after each clone).
  - A finished install becomes the engine when Windows was picked, so Kokoro is the default once installed
    (`speech_ctl::adopt_engine`, tested).
  - `speech_addon_row` no longer sits between `caches_bytes_for_ui` and its doc comment.
  - CHANGELOG and `website/docs/audio.mdx` updated.

## Verified here
- `cargo check --target x86_64-pc-windows-gnu --all-targets`: passes.
- The pure tests of `tts_addon.rs` and `speech_ui::script_lines` ran on Linux in a scratch crate with stubbed
  deps: 7/7 pass (`manifests_are_complete`, `numbers_read_as_words`, `unknown_words_are_spelled_from_lexicon_pieces`,
  `text_is_cut_into_sentences_and_chunks`, `kokoro_command_args` and `chatterbox_request_fields` with `/` for
  `\`, `script_lines_take_an_optional_time`).
- Manifest URLs by HEAD: the sherpa-onnx exe archive, the Kokoro model archive and the onnxruntime-directml,
  numpy and tokenizers wheels exist with exactly the pinned sizes.

## Not verified (run on Windows)
- The full suite: the crate can't build on Linux (winit), and Wine in this sandbox crashes even on a hello-world
  exe. Run `cargo test tts speech jobs transcript` first: `jobs_pane`/`jobs_ui` tests were changed (TTS is now
  cancellable), `assert_no_idle_repaint_speech_panel`, `voice_list_round_trip`, `sapi_fallback_when_no_addon_installed`,
  `speed_and_pitch_become_one_filter`, `settings_become_the_request`, `a_finished_install_becomes_the_engine_only_from_windows`.
- The python.org and huggingface.co parts of the Chatterbox manifest (11 files): the sandbox proxy blocks
  those hosts, so neither existence nor size was checked.
- Real installs and runs: `cargo test real_install -- --ignored`, then `real_run_kokoro` / `real_run_chatterbox`
  / `real_run_windows` (ignored tests).
- The UI by eye: Voiceover pane ▸ Speech, the permission box, Settings ▸ General rows. No screenshot taken.
- Size gate: not run (needs a Windows release build). The add-ons are downloads, not in the exe.

## Risks
- Chatterbox depends on an embeddable Python + three wheels; a wheel's ABI or DirectML on an old GPU may fail
  at `check_runs` (the install then fails cleanly and Speak falls back to Windows).
- `Speech::cache_name` stats the reference WAV on the UI thread when a job is queued (one `metadata` call).
- `extra_lexicon` is a greedy split, not letter-to-sound: unknown words read roughly.
