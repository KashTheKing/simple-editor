# Hand-off: addon-whisperx (`claude/addon-faster-whisper-6d9415`)

Cloud session, 2026-11-20. Scope: `plans/cloud-handoff.md` ▸ addon-whisperx.

## What's done

The paused WIP (`85192c7`) already held nearly the whole feature; this session merged `origin/develop`,
made it compile for Windows, fixed a bug in the aligner, and finished the docs.

- **Add-on** (`src/engine/fast_whisper.rs`, `library/bin/faster-whisper.json`, `src/engine/fw_runner.py`):
  embeddable CPython 3.12.10 plus 11 wheels unzipped from PyPI (no pip; the user needs no Python),
  Systran faster-whisper models, and wav2vec2-base-960h (ONNX, Xenova) as the aligner. Every file is
  pinned by URL, size and SHA-256. PyAV and huggingface_hub are stubbed out (the app decodes with its
  own ffmpeg and downloads the models itself). It installs to `%APPDATA%\SimpleEditor\bin\faster-whisper`,
  and the models go in the whisper models cache.
- **Engine choice** next to whisper.cpp in the Transcript pane's Transcribe section and in Settings ▸
  General ▸ Transcription. `Settings.transcribe_engine` "" means faster-whisper once installed.
  **Precise word timing** (`transcribe_align`) is on by default. If faster-whisper is asked for but not
  installed, whisper.cpp runs and a fallback note says why.
- **Same transcript model**: the runner returns one word per `Segment`, exactly as whisper.cpp's `-ml 1`
  does, so captions, Cut words, fillers and subtitles are unchanged downstream.
- **MCP**: `transcribe.run` / `media.transcribe` take `engine` and `align`; `transcribe.install` takes
  `engine` and `remove`; `transcribe.status` reports `run`, `engine_default`, `align_default` and `addon`.
- **This session**:
  - merged `origin/develop` (clean merge)
  - **aligner fix**: the CTC Viterbi backtrack started at the first frame where every character fit, so
    the frames after it were ignored and the last words of a segment got squeezed together. It now
    backtracks from the window's last frame, a held character can stay on its own score, and a
    character's span ends at the last frame it is heard.
  - `tools/fw-runner-check/check.py`: a synthetic check of the aligner (numpy only, no models)
  - the installed check is cached for 2 s, since it's asked every frame
  - one engine list feeds both pickers, and `transcribe.status` reads the add-on through `addon()`
  - docs: the `docs/customizing.md` MCP rows; removed a `FW_NUMBERS` placeholder and an HTML comment
    from `website/docs/captions.mdx` (MDX 3 can't build `<!-- -->`); a `### Transcription` heading in
    CHANGELOG

## Verified here (Linux container)

- `cargo check --target x86_64-pc-windows-gnu --all-targets`: compiles, no errors, no new warnings.
- `python -I tools/fw-runner-check/check.py src/engine/fw_runner.py`: passes (Viterbi spans, edge snap,
  per-segment re-timing with a fake onnx session). It failed before the aligner fix.
- All 11 wheel pins (URL, size, SHA-256) match PyPI's JSON API. Licences: MIT, BSD-3, Apache-2.0,
  MPL-2.0+MIT (tqdm, unmodified). No GPL.
- Unit tests did **not** run. The crate can't build for Linux: winit has no Linux backend enabled, and
  alsa-sys is missing. The Windows test exe builds (`cargo test --target x86_64-pc-windows-gnu --no-run`),
  but Wine 9 crashes in this container even on a hello-world exe. Run these first on Windows:
  `cargo test fast_whisper` (8 tests) and
  `cargo test transcript_ctl::tests::faster_whisper_falls_back transcript_ctl::tests::transcribe_tools_take`.

## Not verified (needs Windows)

- The Python embed zip pin: python.org is blocked from this container.
- The Hugging Face pins (models and the aligner): huggingface.co is blocked here.
- The whole install, `check_runs`, a real transcription and the UI. Nothing was rendered.
- `real_run_word_times_against_whisper_cpp` (ignored; it downloads about 590 MB into a scratch profile):
  run it on Windows to measure the word-start error against whisper.cpp. It should print a median under
  150 ms with Precise word timing. The runner's alignment changed this session, so this run matters.
- `cargo test` in full, `--selftest`, and a screenshot of the Transcribe section and the Settings row.

## Risks

- The aligner change is checked only on synthetic emissions. If real wav2vec2 output puts words worse
  than before, the ignored real-run test will show it. The old backtrack is in `85192c7` to compare.
- Install size is about 445 MB plus the model. The release exe gains only the manifest, the runner
  (about 10 KB, `include_str!`) and the code.
- CUDA is used only when cuBLAS 12 and cuDNN 9 are already on the machine. That path is untested.
