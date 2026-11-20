# addon-demucs (`claude/addon-demucs-e6f150`): cloud hand-off

## Done
- Merged `origin/develop` (clean, no conflicts) into the paused WIP; it compiled for Windows as-is.
- **Helper rebuilt and documented**: the local session's MSVC zip (`vcomp140.dll`, sha `a2646fd4…`) was never
  uploaded, so it's replaced by `tools/demucs-helper/build.sh` (README beside it). It builds demucs.cpp @ `4edae49`
  with mingw-w64: static, no OpenMP/BLAS, libnyquist WAV-only, UTF-8 code-page manifest, the one-line Windows
  patch (`p_target.string()`). Two exes: AVX2+FMA (`demucs_mt.cpp.main.exe`) and SSE4.2 (`…main.v2.exe`).
  The build is reproducible: two builds on Ubuntu 24.04 / mingw GCC 13-posix gave the same zip,
  sha256 `d62a536ebc2e2b5c77fc2b273f54a83e95ce0a9ad0d23b85375342d4c13f0307`, 1,642,379 B. That's now in
  `library/bin/demucs.json` (version `…-mingw1`).
- `engine::demucs`: picks the v2 exe on CPUs without AVX2/FMA (an older single-exe zip still works); the
  separator runs in the work folder with `in.wav` / `raw` by name, so non-ASCII cache paths never reach it.
- Stem tracks go right under the clip's track, named Drums / Bass / Other / Vocals.
- "Duck the vocals only": the Duck Music under Dialogue action lowers the song's placed vocals stem when it
  has one (`stems_ctl::duck_target`); `audio.duck stem=` stays for MCP.
- "Beats from the drums stem": Beat Aligner Detect and `beats.detect` (new default `source=auto`) read the
  cached drums stem once the song is separated (`stems_ctl::beat_source`).
- Docs: `website/docs/audio.mdx`, `library/README.md` (also fixed a `\b` eaten into `SimpleEditorin`),
  `tools/demucs-helper/README.md`, CHANGELOG `## Unreleased`.

## Verified here
- `cargo check --target x86_64-pc-windows-gnu --all-targets`: no errors; no new warnings in the touched files.
- The helper's CLI contract, on a native Linux build of the same patched source: no args prints the usage
  string `check_runs` looks for (exit 1); a 16-bit 44.1 kHz WAV loads through the WAV-only libnyquist; a bad
  model fails with "Error loading model" on stderr. The `[THREAD i] ( x%)` progress format and the
  `target_<i>_<name>.wav` names were checked against the source.
- The zip holds only KERNEL32/msvcrt imports (no DLLs to ship).

## Not verified (coordinator, on Windows)
- No tests could run: the crate is bin-only and winit refuses Linux, and wine crashes in this container even
  on a hello-world. Run `cargo test stems` and `cargo test demucs`; new tests:
  `placing_aligns_four_linked_copies_and_mutes_the_original` (now checks track order and names),
  `autoduck_lowers_only_the_placed_vocals`, `beats_read_the_drums_stem_once_separated`,
  `the_sse_build_is_picked_only_without_avx2`, `the_separator_runs_on_names_inside_the_work_folder`.
- The Windows exes never ran. The model (huggingface.co) was blocked here, so there was no real separation.
- To publish: build the zip (`tools/demucs-helper/build.sh` on Ubuntu 24.04 / WSL gives the manifest's hash),
  `gh release create demucs-v1 …`, then Settings ▸ General ▸ Stem Separation ▸ Install and
  `cargo test demucs_real_run -- --ignored --nocapture` (also time it: the docs no longer promise a speed).
- UI: no screenshot taken (the Stems submenu, the Inspector rows and the Settings row came from the WIP unchanged).

## Risks
- Inserting stem tracks mid-list shifts the indices of the audio tracks below them (the same thing happens
  when a video track is added). Check that per-track mixer/bus state follows tracks by id, not index.
- The `beats.detect` default changed from the song to the drums stem when one is cached.
- The GCC runtime (libstdc++/libgcc) is GPLv3 with the Runtime Library Exception: no GPL terms on the exes,
  noted in the zip's `licenses/`. If the maintainer still wants zero GCC code, build with MSVC instead (no
  script for that; the old zip's route) and update the manifest's hash.
